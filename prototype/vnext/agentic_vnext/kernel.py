"""保存やAgent呼出しを行わず、現在状態とNextActionだけを導出するKernel。"""

from __future__ import annotations

from dataclasses import replace
from typing import Iterable

from .model import (
    DetectionReport,
    KernelDecision,
    NextAction,
    ProjectSnapshot,
    RequirementInstance,
    RuleIndex,
    SignalCandidate,
    canonical_digest,
)


class ThinKernel:
    """Rule評価と状態遷移だけを行う、副作用のないKernel。

    同じSnapshot・Rule Index・Detection Reportには同じ結果を返すことが
    KernelのContractである。LLM、network、filesystemへの依存は置かない。
    """

    def evaluate(
        self,
        snapshot: ProjectSnapshot,
        rule_index: RuleIndex,
        detection: DetectionReport,
    ) -> KernelDecision:
        if detection.coverage.status != "complete":
            diagnostics = tuple(
                "detection coverage incomplete: "
                f"{gap['kind']}"
                + (f" ({gap['ref']})" if gap.get("ref") else "")
                + f": {gap['reason']}"
                for gap in detection.coverage.gaps
            )
            return KernelDecision(
                state="blocked-detection",
                action=None,
                requirement_instances=(),
                diagnostics=diagnostics,
            )
        try:
            dispositions = self._candidate_dispositions(snapshot)
            confirmed = self._confirmed_candidates(detection, dispositions)
            unreviewed = tuple(
                candidate
                for candidate in detection.candidates
                if dispositions.get(candidate.fingerprint)
                not in {"confirmed", "excluded"}
            )
            instances = self._instantiate(snapshot, rule_index, confirmed)
        except ValueError as error:
            return KernelDecision(
                state="invalid",
                action=None,
                requirement_instances=(),
                diagnostics=(str(error),),
            )

        instances = tuple(
            replace(
                instance,
                status=(
                    "satisfied"
                    if (
                        instance.requirement_id == "risk-signals-reviewed"
                        and not unreviewed
                    )
                    or self._is_satisfied(instance, snapshot)
                    else "unsatisfied"
                ),
            )
            for instance in instances
        )

        # New candidate fingerprints always take precedence over work already in
        # progress. Existing dispositions are reusable because the fingerprint
        # includes detector version, bindings, and evidence digests.
        if unreviewed:
            risk_instances = tuple(
                instance
                for instance in instances
                if instance.requirement_id == "risk-signals-reviewed"
            )
            action = self._make_action(
                snapshot,
                role="Analyst",
                action="review-risk-signals",
                instances=risk_instances,
                reason=f"{len(unreviewed)}件の新しいSignal候補を確認してください",
                expected_result_schema="result.risk-signal-review",
                candidate_fingerprints=tuple(
                    candidate.fingerprint for candidate in unreviewed
                ),
            )
            return KernelDecision(
                state="needs-analysis",
                action=action,
                requirement_instances=instances,
            )

        # Human Authorityは通常のRequirement実行より優先する。未決定事項を
        # AnalystやBuilderが暗黙に補完したまま先へ進ませないためである。
        pending_request = self._pending_decision_request(snapshot)
        if pending_request is not None:
            request_id = pending_request["id"]
            if not self._has_fresh_human_answer(snapshot, request_id):
                action = self._make_action(
                    snapshot,
                    role="Human",
                    action="answer-decision-request",
                    instances=(),
                    reason=f"判断依頼 {request_id} への回答が必要です",
                    expected_result_schema="result.human-answer",
                    extra=request_id,
                )
                return KernelDecision(
                    state="needs-human-decision",
                    action=action,
                    requirement_instances=instances,
                )
            if not self._decision_is_recorded(snapshot, request_id):
                targets = tuple(
                    instance
                    for instance in instances
                    if instance.status == "unsatisfied"
                    and instance.role == "Analyst"
                    and instance.phase == "before-build"
                )
                action = self._make_action(
                    snapshot,
                    role="Analyst",
                    action="record-human-decision",
                    instances=targets,
                    reason=f"回答済みの判断 {request_id} をDecisionとContractへ反映してください",
                    expected_result_schema="result.analysis",
                    extra=request_id,
                )
                return KernelDecision(
                    state="needs-decision-recording",
                    action=action,
                    requirement_instances=instances,
                )

        # 実行可能なものだけを選び、依存先が未充足のInstanceは次回へ残す。
        before_build = tuple(
            instance
            for instance in instances
            if instance.phase == "before-build"
            and instance.status == "unsatisfied"
            and self._dependencies_satisfied(instance, instances)
        )
        if before_build:
            action = self._batch_action(snapshot, before_build)
            state = (
                "needs-pre-build-challenge"
                if action.role == "Challenger"
                else "needs-analysis"
            )
            return KernelDecision(
                state=state,
                action=action,
                requirement_instances=instances,
            )

        blocked_before_build = tuple(
            instance
            for instance in instances
            if instance.phase == "before-build" and instance.status == "unsatisfied"
        )
        if blocked_before_build:
            return KernelDecision(
                state="invalid",
                action=None,
                requirement_instances=instances,
                diagnostics=("before-build Requirementの依存関係を解決できません",),
            )

        if snapshot.repository.get("phase", "pre-build") != "post-build":
            action = self._make_action(
                snapshot,
                role="Builder",
                action="implement-change",
                instances=(),
                reason="実装前に必要なRequirementを満たしました",
                expected_result_schema="result.build",
            )
            return KernelDecision(
                state="ready-to-build",
                action=action,
                requirement_instances=instances,
            )

        # build後にだけ有効になるRuleは、同じKernelでbefore-mergeとして扱う。
        before_merge = tuple(
            instance
            for instance in instances
            if instance.phase == "before-merge"
            and instance.status == "unsatisfied"
            and self._dependencies_satisfied(instance, instances)
        )
        if before_merge:
            action = self._batch_action(snapshot, before_merge)
            if action.role == "Builder":
                state = "needs-evidence"
            elif action.role == "Challenger":
                state = "needs-post-build-challenge"
            else:
                state = "needs-post-build-analysis"
            return KernelDecision(
                state=state,
                action=action,
                requirement_instances=instances,
            )

        blocked_before_merge = tuple(
            instance
            for instance in instances
            if instance.phase == "before-merge" and instance.status == "unsatisfied"
        )
        if blocked_before_merge:
            return KernelDecision(
                state="invalid",
                action=None,
                requirement_instances=instances,
                diagnostics=("before-merge Requirementの依存関係を解決できません",),
            )

        return KernelDecision(
            state="ready-to-merge",
            action=None,
            requirement_instances=instances,
        )

    def _candidate_dispositions(
        self,
        snapshot: ProjectSnapshot,
    ) -> dict[str, str]:
        dispositions: dict[str, str] = {}
        for result in snapshot.results:
            if (
                result.get("result_schema") != "result.risk-signal-review"
                or result.get("role") != "Analyst"
            ):
                continue
            for review in result.get("payload", {}).get("reviewed_candidates", []):
                dispositions[review["fingerprint"]] = review["status"]
        return dispositions

    def _confirmed_candidates(
        self,
        detection: DetectionReport,
        dispositions: dict[str, str],
    ) -> tuple[SignalCandidate, ...]:
        # 未確認候補はRuleへ渡さない。Detectorの推測だけでRequirementが
        # 増えることを防ぐため、fingerprintが確認済みの候補に限定する。
        return tuple(
            candidate
            for candidate in detection.candidates
            if dispositions.get(candidate.fingerprint) == "confirmed"
        )

    def _instantiate(
        self,
        snapshot: ProjectSnapshot,
        rule_index: RuleIndex,
        confirmed: Iterable[SignalCandidate],
    ) -> tuple[RequirementInstance, ...]:
        selected: dict[str, RequirementInstance] = {}
        # NoneはSignalに依存しないbaseline Ruleを一度だけ評価するための番兵。
        candidates: tuple[SignalCandidate | None, ...] = (None, *tuple(confirmed))
        for candidate in candidates:
            for rule in rule_index.rules:
                if rule.condition == "always" and candidate is not None:
                    continue
                if rule.condition == "signal":
                    if candidate is None or rule.signal != candidate.signal:
                        continue
                if (
                    rule.repository_phase is not None
                    and rule.repository_phase != snapshot.repository.get("phase")
                ):
                    continue
                definition = rule_index.requirements[rule.requirement_id]
                subjects = tuple(
                    sorted(
                        self._resolve_subject(expression, snapshot, candidate)
                        for expression in rule.subjects
                    )
                )
                instance_key = "|".join((definition.id, *subjects))
                instance = RequirementInstance(
                    requirement_id=definition.id,
                    subject_refs=subjects,
                    instance_key=instance_key,
                    selected_by=(rule.id,),
                    definition_digest=definition.definition_digest,
                    phase=definition.phase,
                    role=definition.role,
                    result_schema=definition.result_schema,
                    depends_on=definition.depends_on,
                    context=definition.context,
                )
                existing = selected.get(instance_key)
                if existing is None:
                    selected[instance_key] = instance
                elif existing.definition_digest != instance.definition_digest:
                    raise ValueError(
                        f"Requirement Instanceの定義が競合しています: {instance_key}"
                    )
                else:
                    # 複数Ruleが同じ対象へ同じRequirementを要求しても一件にする。
                    # selected_byはexplain時に全選択理由を示すため失わない。
                    selected[instance_key] = replace(
                        existing,
                        selected_by=tuple(sorted(set(existing.selected_by + (rule.id,)))),
                    )
        return tuple(sorted(selected.values(), key=lambda item: item.instance_key))

    def _resolve_subject(
        self,
        expression: str,
        snapshot: ProjectSnapshot,
        candidate: SignalCandidate | None,
    ) -> str:
        if expression == "change.id":
            return snapshot.change_id
        if expression.startswith("binding.") and candidate is not None:
            key = expression.removeprefix("binding.")
            if key not in candidate.bindings:
                raise ValueError(f"Signal bindingがありません: {key}")
            return candidate.bindings[key]
        raise ValueError(f"解決できないsubject式です: {expression}")

    def _is_satisfied(
        self,
        instance: RequirementInstance,
        snapshot: ProjectSnapshot,
    ) -> bool:
        for result in snapshot.results:
            for outcome in result.get("payload", {}).get("outcomes", []):
                if (
                    result.get("result_schema") == instance.result_schema
                    and result.get("role") == instance.role
                    and outcome.get("instance_key") == instance.instance_key
                    and outcome.get("definition_digest") == instance.definition_digest
                    and outcome.get("status") == "satisfied"
                    and self._outcome_is_fresh(result, outcome, snapshot)
                ):
                    return True
        return False

    def _outcome_is_fresh(
        self,
        result: dict[str, object],
        outcome: dict[str, object],
        snapshot: ProjectSnapshot,
    ) -> bool:
        """Check the sources used for one outcome, not the entire batched Result.

        Older Result Records may not have outcome-level refs, so the prototype keeps
        a record-level fallback until the persisted schema is finalized.
        """

        refs = outcome.get(
            "freshness_refs",
            result.get("freshness_refs", result.get("input_refs", {})),
        )
        if not isinstance(refs, dict):
            return False
        return all(
            snapshot.artifact_digests.get(str(ref)) == digest
            for ref, digest in refs.items()
        )

    def _is_fresh(self, result: dict[str, object], snapshot: ProjectSnapshot) -> bool:
        """Resultが参照した全正本のdigestが現在値と一致するか確認する。"""

        refs = result.get("freshness_refs", result.get("input_refs", {}))
        if not isinstance(refs, dict):
            return False
        return all(
            snapshot.artifact_digests.get(str(ref)) == digest
            for ref, digest in refs.items()
        )

    def _dependencies_satisfied(
        self,
        instance: RequirementInstance,
        instances: tuple[RequirementInstance, ...],
    ) -> bool:
        for dependency in instance.depends_on:
            dependency_instances = [
                value for value in instances if value.requirement_id == dependency
            ]
            if not dependency_instances or any(
                value.status != "satisfied" for value in dependency_instances
            ):
                return False
        return True

    def _pending_decision_request(
        self,
        snapshot: ProjectSnapshot,
    ) -> dict[str, object] | None:
        requests: list[dict[str, object]] = []
        for result in snapshot.results:
            if not self._is_fresh(result, snapshot):
                continue
            requests.extend(result.get("payload", {}).get("decision_requests", []))
        unresolved = [
            request
            for request in requests
            if not self._decision_is_recorded(snapshot, str(request["id"]))
        ]
        return unresolved[0] if unresolved else None

    def _has_fresh_human_answer(
        self,
        snapshot: ProjectSnapshot,
        request_id: str,
    ) -> bool:
        return any(
            result.get("result_schema") == "result.human-answer"
            and result.get("role") == "Human"
            and self._is_fresh(result, snapshot)
            and any(
                answer.get("request_id") == request_id
                for answer in result.get("payload", {}).get("answers", [])
            )
            for result in snapshot.results
        )

    def _decision_is_recorded(
        self,
        snapshot: ProjectSnapshot,
        request_id: str,
    ) -> bool:
        # Human回答だけでは規範にならない。回答を解決するaccepted Decisionと、
        # そのDecisionを根拠にするContract clauseの両方を要求する。
        decision_ids = {
            decision["id"]
            for decision in snapshot.decisions
            if decision.get("status") == "accepted"
            and request_id in decision.get("resolves", [])
        }
        if not decision_ids:
            return False
        return any(
            clause.get("authority_ref") in decision_ids
            for contract in snapshot.contracts
            for clause in contract.get("clauses", [])
        )

    def _batch_action(
        self,
        snapshot: ProjectSnapshot,
        candidates: tuple[RequirementInstance, ...],
    ) -> NextAction:
        # 順序を固定し、同じ入力から常に同じRole・Schemaのbatchを作る。
        first = sorted(
            candidates,
            key=lambda item: (
                {"Analyst": 0, "Builder": 1, "Challenger": 2}.get(item.role, 9),
                item.result_schema,
                item.instance_key,
            ),
        )[0]
        batch = tuple(
            item
            for item in candidates
            if item.role == first.role and item.result_schema == first.result_schema
        )
        action_name = {
            "Analyst": "analyze-requirements",
            "Builder": "record-evidence",
            "Challenger": "challenge-result",
        }.get(first.role, "complete-requirements")
        return self._make_action(
            snapshot,
            role=first.role,
            action=action_name,
            instances=batch,
            reason=f"{len(batch)}件のRequirementを満たす必要があります",
            expected_result_schema=first.result_schema,
        )

    def _make_action(
        self,
        snapshot: ProjectSnapshot,
        role: str,
        action: str,
        instances: tuple[RequirementInstance, ...],
        reason: str,
        expected_result_schema: str,
        extra: str = "",
        candidate_fingerprints: tuple[str, ...] = (),
    ) -> NextAction:
        # ランダムIDにせず、Actionの意味が同じなら同じIDになるようにする。
        # Context digestは別途source versionを含むため、入力変更はそこで検出する。
        action_id = "action." + canonical_digest(
            {
                "change_id": snapshot.change_id,
                "role": role,
                "action": action,
                "instances": [
                    (item.instance_key, item.definition_digest) for item in instances
                ],
                "extra": extra,
                "candidate_fingerprints": candidate_fingerprints,
            }
        ).removeprefix("sha256:")[:16]
        return NextAction(
            id=action_id,
            role=role,
            action=action,
            requirement_instances=instances,
            reason=reason,
            expected_result_schema=expected_result_schema,
            candidate_fingerprints=candidate_fingerprints,
        )
