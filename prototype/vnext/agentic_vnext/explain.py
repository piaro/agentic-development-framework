"""Build an inspectable trace from the same inputs used by the Thin Kernel."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .model import DetectionReport, KernelDecision, ProjectSnapshot, RuleIndex

EXPLAIN_REPORT_SCHEMA_VERSION = "1"


@dataclass(frozen=True)
class ExplainReport:
    """Machine-readable trace with a small human-readable renderer."""

    change_id: str
    state: str
    candidates: tuple[dict[str, Any], ...]
    requirements: tuple[dict[str, Any], ...]
    authority: tuple[dict[str, Any], ...]
    next_action: dict[str, Any] | None
    diagnostics: tuple[str, ...]

    def as_dict(self) -> dict[str, Any]:
        return {
            "schema_version": EXPLAIN_REPORT_SCHEMA_VERSION,
            "change_id": self.change_id,
            "state": self.state,
            "candidates": list(self.candidates),
            "requirements": list(self.requirements),
            "authority": list(self.authority),
            "next_action": self.next_action,
            "diagnostics": list(self.diagnostics),
        }

    def render_text(self) -> str:
        """Render a stable summary without exposing the full Generated Context."""

        lines = [f"change: {self.change_id}", f"state: {self.state}"]
        if self.next_action is None:
            lines.append("next: none")
        else:
            lines.append(
                "next: "
                f"{self.next_action['role']}/{self.next_action['action']} "
                f"({self.next_action['reason']})"
            )

        lines.append("candidates:")
        for candidate in self.candidates:
            rules = ",".join(candidate["applied_rules"]) or "-"
            lines.append(
                f"  - {candidate['signal']} [{candidate['disposition']}] "
                f"{candidate['fingerprint']} rules={rules}"
            )

        lines.append("requirements:")
        for requirement in self.requirements:
            selected_by = ",".join(requirement["selected_by"]) or "-"
            blocked_by = ",".join(requirement["blocked_by"]) or "-"
            lines.append(
                f"  - {requirement['instance_key']} [{requirement['status']}] "
                f"rules={selected_by} blocked_by={blocked_by}"
            )
            for result in requirement["result_checks"]:
                stale_refs = ",".join(result["stale_refs"]) or "-"
                lines.append(
                    f"      result={result['result_id']} "
                    f"accepted={str(result['accepted']).lower()} "
                    f"stale_refs={stale_refs}"
                )

        if self.authority:
            lines.append("authority:")
            for request in self.authority:
                lines.append(
                    f"  - {request['request_id']} [{request['status']}] "
                    f"answers={len(request['answer_result_ids'])} "
                    f"decisions={len(request['decision_ids'])}"
                )
        return "\n".join(lines) + "\n"


class ExplanationBuilder:
    """Explain a decision without changing Project state or rerunning agents."""

    def build(
        self,
        snapshot: ProjectSnapshot,
        rule_index: RuleIndex,
        detection: DetectionReport,
        decision: KernelDecision,
    ) -> ExplainReport:
        dispositions = self._candidate_dispositions(snapshot)
        candidates = tuple(
            self._candidate_trace(
                candidate,
                dispositions.get(candidate.fingerprint, ("unreviewed", None)),
                rule_index,
                decision,
                snapshot,
            )
            for candidate in detection.candidates
        )
        requirements = tuple(
            self._requirement_trace(instance, decision, snapshot)
            for instance in decision.requirement_instances
        )
        authority = self._authority_trace(snapshot)
        action = decision.action
        next_action = (
            {
                "id": action.id,
                "role": action.role,
                "action": action.action,
                "reason": action.reason,
                "requirement_instances": [
                    instance.instance_key
                    for instance in action.requirement_instances
                ],
                "candidate_fingerprints": list(action.candidate_fingerprints),
            }
            if action is not None
            else None
        )
        return ExplainReport(
            change_id=snapshot.change_id,
            state=decision.state,
            candidates=candidates,
            requirements=requirements,
            authority=authority,
            next_action=next_action,
            diagnostics=decision.diagnostics,
        )

    def _candidate_dispositions(
        self,
        snapshot: ProjectSnapshot,
    ) -> dict[str, tuple[str, str]]:
        dispositions: dict[str, tuple[str, str]] = {}
        for result in snapshot.results:
            if (
                result.get("result_schema") != "result.risk-signal-review"
                or result.get("role") != "Analyst"
            ):
                continue
            for review in result.get("payload", {}).get("reviewed_candidates", []):
                dispositions[review["fingerprint"]] = (
                    review["status"],
                    result["id"],
                )
        return dispositions

    def _candidate_trace(
        self,
        candidate,
        disposition: tuple[str, str | None],
        rule_index: RuleIndex,
        decision: KernelDecision,
        snapshot: ProjectSnapshot,
    ) -> dict[str, Any]:
        status, result_id = disposition
        applied_rules = tuple(
            sorted(
                rule.id
                for rule in rule_index.rules
                if status == "confirmed"
                and rule.condition == "signal"
                and rule.signal == candidate.signal
                and (
                    rule.repository_phase is None
                    or rule.repository_phase == snapshot.repository.get("phase")
                )
            )
        )
        instance_keys = tuple(
            sorted(
                instance.instance_key
                for instance in decision.requirement_instances
                if set(instance.selected_by).intersection(applied_rules)
            )
        )
        return {
            "fingerprint": candidate.fingerprint,
            "signal": candidate.signal,
            "bindings": dict(sorted(candidate.bindings.items())),
            "evidence_refs": list(candidate.evidence_refs),
            "disposition": status,
            "disposition_result_id": result_id,
            "applied_rules": list(applied_rules),
            "requirement_instances": list(instance_keys),
        }

    def _requirement_trace(
        self,
        instance,
        decision: KernelDecision,
        snapshot: ProjectSnapshot,
    ) -> dict[str, Any]:
        result_checks: list[dict[str, Any]] = []
        for result in snapshot.results:
            for outcome in result.get("payload", {}).get("outcomes", []):
                if outcome.get("instance_key") != instance.instance_key:
                    continue
                refs = outcome.get(
                    "freshness_refs",
                    result.get("freshness_refs", result.get("input_refs", {})),
                )
                stale_refs = (
                    sorted(
                        str(ref)
                        for ref, digest in refs.items()
                        if snapshot.artifact_digests.get(str(ref)) != digest
                    )
                    if isinstance(refs, dict)
                    else ["<invalid-freshness-refs>"]
                )
                definition_matches = (
                    outcome.get("definition_digest")
                    == instance.definition_digest
                )
                result_schema_matches = (
                    result.get("result_schema") == instance.result_schema
                )
                role_matches = result.get("role") == instance.role
                result_checks.append(
                    {
                        "result_id": result["id"],
                        "outcome_status": outcome.get("status"),
                        "definition_matches": definition_matches,
                        "result_schema_matches": result_schema_matches,
                        "role_matches": role_matches,
                        "stale_refs": stale_refs,
                        "accepted": (
                            outcome.get("status") == "satisfied"
                            and definition_matches
                            and result_schema_matches
                            and role_matches
                            and not stale_refs
                        ),
                    }
                )

        blocked_by = tuple(
            sorted(
                candidate.instance_key
                for dependency in instance.depends_on
                for candidate in decision.requirement_instances
                if candidate.requirement_id == dependency
                and candidate.status != "satisfied"
            )
        )
        satisfaction_basis = (
            "all-current-candidates-reviewed"
            if instance.requirement_id == "risk-signals-reviewed"
            and instance.status == "satisfied"
            else "accepted-result-outcome"
            if instance.status == "satisfied"
            else "no-fresh-satisfying-result"
        )
        return {
            "instance_key": instance.instance_key,
            "requirement_id": instance.requirement_id,
            "subject_refs": list(instance.subject_refs),
            "selected_by": list(instance.selected_by),
            "status": instance.status,
            "satisfaction_basis": satisfaction_basis,
            "blocked_by": list(blocked_by),
            "result_checks": result_checks,
        }

    def _authority_trace(
        self,
        snapshot: ProjectSnapshot,
    ) -> tuple[dict[str, Any], ...]:
        requests: dict[str, dict[str, Any]] = {}
        for result in snapshot.results:
            for request in result.get("payload", {}).get("decision_requests", []):
                requests[request["id"]] = {
                    "request_id": request["id"],
                    "request_result_id": result["id"],
                    "request_stale_refs": self._record_stale_refs(
                        result,
                        snapshot,
                    ),
                }

        traces: list[dict[str, Any]] = []
        for request_id, request in sorted(requests.items()):
            answer_result_ids = sorted(
                result["id"]
                for result in snapshot.results
                if result.get("result_schema") == "result.human-answer"
                and result.get("role") == "Human"
                and not self._record_stale_refs(result, snapshot)
                and any(
                    answer.get("request_id") == request_id
                    for answer in result.get("payload", {}).get("answers", [])
                )
            )
            decision_ids = sorted(
                decision["id"]
                for decision in snapshot.decisions
                if decision.get("status") == "accepted"
                and request_id in decision.get("resolves", [])
            )
            contract_refs = sorted(
                f"{contract['id']}#{clause['id']}"
                for contract in snapshot.contracts
                for clause in contract.get("clauses", [])
                if clause.get("authority_ref") in decision_ids
            )
            # An accepted Decision embodied in a Contract is the terminal
            # authority state. Keep stale request refs visible, but do not make
            # the completed recording look open again after that Contract edit.
            if contract_refs:
                status = "recorded"
            elif request["request_stale_refs"]:
                status = "stale-request"
            elif answer_result_ids:
                status = "answered-not-recorded"
            else:
                status = "open"
            traces.append(
                {
                    **request,
                    "status": status,
                    "answer_result_ids": answer_result_ids,
                    "decision_ids": decision_ids,
                    "contract_clause_refs": contract_refs,
                }
            )
        return tuple(traces)

    def _record_stale_refs(
        self,
        result: dict[str, Any],
        snapshot: ProjectSnapshot,
    ) -> list[str]:
        refs = result.get("freshness_refs", result.get("input_refs", {}))
        if not isinstance(refs, dict):
            return ["<invalid-freshness-refs>"]
        return sorted(
            str(ref)
            for ref, digest in refs.items()
            if snapshot.artifact_digests.get(str(ref)) != digest
        )
