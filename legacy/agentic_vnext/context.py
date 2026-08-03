"""NextActionに必要な正本だけを、Agent向けContextへ投影する。"""

from __future__ import annotations

from .model import (
    DetectionReport,
    GeneratedContext,
    KernelDecision,
    ProjectSnapshot,
    RequirementInstance,
    canonical_digest,
)


class ContextCompiler:
    """NextActionに必要な正本参照を、決定的なContextへまとめる。

    Context全文は作業用の中間生成物であり、正本ではない。Resultには全文ではなく、
    参照IDとdigestを残し、後から何を根拠に判断したかを検証可能にする。
    """

    def compile(
        self,
        decision: KernelDecision,
        snapshot: ProjectSnapshot,
        detection: DetectionReport,
    ) -> GeneratedContext | None:
        action = decision.action
        if action is None:
            return None

        # 最初のrisk reviewに既存Contractを混ぜると、Contract更新のたびに
        # Signal確認までstaleになるため、Changeとコード根拠だけへ絞る。
        risk_review_only = (
            action.action == "review-risk-signals"
            and {
                instance.requirement_id
                for instance in action.requirement_instances
            }
            == {"risk-signals-reviewed"}
        )
        # A batched Action has a union manifest for submission validation and a
        # separate manifest for each Requirement outcome. The latter prevents a
        # source needed by one Requirement from invalidating all sibling outcomes.
        instance_source_refs = {
            instance.instance_key: self._instance_source_refs(
                instance,
                decision.requirement_instances,
                snapshot,
            )
            for instance in action.requirement_instances
        }
        if instance_source_refs:
            source_refs = tuple(
                sorted(
                    {
                        ref
                        for refs in instance_source_refs.values()
                        for ref in refs
                    }
                )
            )
        else:
            source_refs = self._action_source_refs(action.action, snapshot)
        source_digests = {
            ref: snapshot.artifact_digests[ref]
            for ref in source_refs
            if ref in snapshot.artifact_digests
        }
        instance_source_digests = {
            instance_key: {
                ref: snapshot.artifact_digests[ref]
                for ref in refs
                if ref in snapshot.artifact_digests
            }
            for instance_key, refs in instance_source_refs.items()
        }
        payload = {
            "change": snapshot.change,
            "action": {
                "id": action.id,
                "role": action.role,
                "action": action.action,
                "reason": action.reason,
                "expected_result_schema": action.expected_result_schema,
                "candidate_fingerprints": action.candidate_fingerprints,
            },
            "requirement_instances": [
                {
                    "requirement_id": item.requirement_id,
                    "instance_key": item.instance_key,
                    "subject_refs": item.subject_refs,
                    "definition_digest": item.definition_digest,
                    "selected_by": item.selected_by,
                    "context_selectors": item.context,
                    "sources": instance_source_digests[item.instance_key],
                }
                for item in action.requirement_instances
            ],
            "signal_candidates": [
                {
                    "signal": candidate.signal,
                    "bindings": candidate.bindings,
                    "evidence_refs": candidate.evidence_refs,
                    "fingerprint": candidate.fingerprint,
                }
                for candidate in detection.candidates
                if candidate.fingerprint in action.candidate_fingerprints
            ]
            if risk_review_only
            else [],
            "sources": source_digests,
        }
        digest = canonical_digest(payload)
        return GeneratedContext(
            action_id=action.id,
            role=action.role,
            source_refs=source_refs,
            source_digests=source_digests,
            instance_source_digests=instance_source_digests,
            payload=payload,
            digest=digest,
        )

    def _instance_source_refs(
        self,
        instance: RequirementInstance,
        all_instances: tuple[RequirementInstance, ...],
        snapshot: ProjectSnapshot,
    ) -> tuple[str, ...]:
        """Resolve declarative context selectors for one Requirement Instance.

        Selectors name information categories, while this compiler decides which
        concrete source IDs match the Instance subjects. This keeps file-layout
        knowledge out of Requirement definitions.
        """

        refs: set[str] = set()
        selectors = set(instance.context)
        if "change" in selectors:
            refs.add(snapshot.change_id)
        if "repository-artifacts" in selectors:
            refs.update(
                artifact["ref"] for artifact in snapshot.repository.get("artifacts", [])
            )
        if "affected-code" in selectors:
            refs.update(self._matching_artifact_refs(instance, snapshot))
        matching_contracts = self._matching_contracts(instance, snapshot)
        if "matching-contracts" in selectors:
            refs.update(contract["id"] for contract in matching_contracts)
        if "matching-decisions" in selectors:
            authority_refs = {
                clause["authority_ref"]
                for contract in matching_contracts
                for clause in contract.get("clauses", [])
                if clause.get("authority_ref")
            }
            refs.update(
                decision["id"]
                for decision in snapshot.decisions
                if decision["id"] in authority_refs
            )
        if "dependency-results" in selectors:
            dependency_keys = {
                candidate.instance_key
                for candidate in all_instances
                if candidate.requirement_id in instance.depends_on
            }
            refs.update(
                result["id"]
                for result in snapshot.results
                if any(
                    outcome.get("instance_key") in dependency_keys
                    for outcome in result.get("payload", {}).get("outcomes", [])
                )
            )
        if "matching-evidence" in selectors:
            refs.update(
                evidence["id"]
                for evidence in snapshot.evidence
                if self._matches_subjects(evidence, instance.subject_refs)
                or instance.instance_key
                in evidence.get("requirement_instances", [])
            )
        # These broad selectors are retained for experiments that explicitly need
        # a full category. Standard Requirements should prefer matching selectors.
        if "contracts" in selectors:
            refs.update(contract["id"] for contract in snapshot.contracts)
        if "decisions" in selectors:
            refs.update(decision["id"] for decision in snapshot.decisions)
        if "results" in selectors:
            refs.update(result["id"] for result in snapshot.results)
        if "evidence" in selectors:
            refs.update(evidence["id"] for evidence in snapshot.evidence)
        return tuple(sorted(refs))

    def _action_source_refs(
        self,
        action: str,
        snapshot: ProjectSnapshot,
    ) -> tuple[str, ...]:
        """Select sources for actions that do not represent Requirement outcomes."""

        refs = {snapshot.change_id}
        if action == "answer-decision-request":
            refs.update(result["id"] for result in snapshot.results)
        elif action == "implement-change":
            refs.update(contract["id"] for contract in snapshot.contracts)
            refs.update(decision["id"] for decision in snapshot.decisions)
            refs.update(
                artifact["ref"] for artifact in snapshot.repository.get("artifacts", [])
            )
        return tuple(sorted(refs))

    def _matching_contracts(
        self,
        instance: RequirementInstance,
        snapshot: ProjectSnapshot,
    ) -> tuple[dict[str, object], ...]:
        return tuple(
            contract
            for contract in snapshot.contracts
            if self._matches_subjects(contract, instance.subject_refs)
        )

    def _matching_artifact_refs(
        self,
        instance: RequirementInstance,
        snapshot: ProjectSnapshot,
    ) -> tuple[str, ...]:
        return tuple(
            artifact["ref"]
            for artifact in snapshot.repository.get("artifacts", [])
            if self._matches_subjects(artifact, instance.subject_refs)
        )

    def _matches_subjects(
        self,
        source: dict[str, object],
        subject_refs: tuple[str, ...],
    ) -> bool:
        applies_to = set(source.get("applies_to", []))
        return bool(applies_to.intersection(subject_refs))
