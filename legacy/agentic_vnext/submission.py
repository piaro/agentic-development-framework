"""Validate one issued Action response and build its immutable Result Record."""

from __future__ import annotations

from copy import deepcopy
from typing import Any

from .model import GeneratedContext, ProjectSnapshot, canonical_digest
from .schema import SchemaRegistry, default_schema_registry
from .versions import APPLICATION_PROTOCOL_VERSION


RESULT_SUBMISSION_PROTOCOL_VERSION = APPLICATION_PROTOCOL_VERSION


def prepare_result(
    *,
    context: GeneratedContext,
    current: ProjectSnapshot,
    change_id: str,
    action_id: str,
    context_digest: str,
    role: str,
    result_schema: str,
    payload: dict[str, Any],
    output_refs: tuple[str, ...] = (),
    schema_registry: SchemaRegistry | None = None,
) -> dict[str, Any]:
    """Return the Result Record that may be appended by a Project Store.

    This function has no persistence side effects. The Application is responsible
    for resolving an issued Action, and the Store remains responsible for an
    exclusive append so retries cannot overwrite an existing Result.
    """

    registry = schema_registry or default_schema_registry()
    if context.action_id != action_id:
        raise ValueError("action does not match issued context")
    if context.digest != context_digest:
        raise ValueError("context digest does not match issued action")
    if context.role != role:
        raise ValueError("role does not match issued action")
    expected_schema = context.payload["action"]["expected_result_schema"]
    if expected_schema != result_schema:
        raise ValueError(
            f"result schema mismatch: expected {expected_schema}, got {result_schema}"
        )
    if current.change_id != change_id:
        raise ValueError(
            f"change does not match current Snapshot: expected "
            f"{current.change_id}, got {change_id}"
        )

    output_ref_set = set(output_refs)
    if len(output_ref_set) != len(output_refs):
        raise ValueError("output refs must be unique")
    # An Action cannot be accepted against inputs that changed after issuance.
    # A source intentionally changed by this Action must be named as an output.
    for ref, issued_digest in context.source_digests.items():
        current_digest = current.artifact_digests.get(ref)
        if current_digest != issued_digest and ref not in output_ref_set:
            raise ValueError(f"issued context is stale: {ref}")
    missing_outputs = [
        ref for ref in output_refs if ref not in current.artifact_digests
    ]
    if missing_outputs:
        raise ValueError(
            "output ref does not exist: " + ", ".join(sorted(missing_outputs))
        )
    freshness_refs = dict(context.source_digests)
    freshness_refs.update(
        {ref: current.artifact_digests[ref] for ref in output_refs}
    )

    result_payload = deepcopy(payload)
    registry.validate_result_payload(result_schema, result_payload)
    if result_schema == "result.risk-signal-review":
        _validate_candidate_reviews(context, result_payload)
    _enrich_outcomes(context, current, result_payload, output_refs)

    result_body = {
        "schema_version": RESULT_SUBMISSION_PROTOCOL_VERSION,
        "change_id": change_id,
        "action_id": action_id,
        "role": role,
        "result_schema": result_schema,
        "context_digest": context_digest,
        "input_refs": context.source_digests,
        "output_refs": sorted(output_refs),
        "freshness_refs": freshness_refs,
        "payload": result_payload,
    }
    result = {
        "id": "result."
        + canonical_digest(result_body).removeprefix("sha256:")[:20],
        **result_body,
    }
    # Validate the complete envelope before handing it to a persistence Adapter.
    registry.validate("result", result)
    return result


def _validate_candidate_reviews(
    context: GeneratedContext,
    payload: dict[str, Any],
) -> None:
    offered_candidates = {
        candidate["fingerprint"]: candidate
        for candidate in context.payload["signal_candidates"]
    }
    reviewed_fingerprints: set[str] = set()
    for review in payload.get("reviewed_candidates", []):
        fingerprint = review.get("fingerprint")
        if fingerprint not in offered_candidates:
            raise ValueError(
                f"candidate was not offered by issued action: {fingerprint}"
            )
        if fingerprint in reviewed_fingerprints:
            raise ValueError(
                f"candidate was reviewed more than once: {fingerprint}"
            )
        unknown_basis_refs = (
            set(review["basis_refs"])
            - set(offered_candidates[fingerprint]["evidence_refs"])
        )
        if unknown_basis_refs:
            raise ValueError(
                "candidate review refers to evidence not offered by "
                "issued action: "
                + ", ".join(sorted(unknown_basis_refs))
            )
        reviewed_fingerprints.add(fingerprint)


def _enrich_outcomes(
    context: GeneratedContext,
    current: ProjectSnapshot,
    payload: dict[str, Any],
    output_refs: tuple[str, ...],
) -> None:
    known_instance_keys = set(context.instance_source_digests)
    selectors_by_instance = {
        item["instance_key"]: set(item["context_selectors"])
        for item in context.payload["requirement_instances"]
    }
    subjects_by_instance = {
        item["instance_key"]: set(item["subject_refs"])
        for item in context.payload["requirement_instances"]
    }
    for outcome in payload.get("outcomes", []):
        instance_key = outcome.get("instance_key")
        if instance_key not in known_instance_keys:
            raise ValueError(
                f"outcome does not belong to issued action: {instance_key}"
            )
        instance_inputs = context.instance_source_digests[instance_key]
        instance_freshness = dict(instance_inputs)
        instance_freshness.update(
            {
                ref: current.artifact_digests[ref]
                for ref in output_refs
                if _output_is_relevant(
                    ref,
                    instance_inputs,
                    selectors_by_instance[instance_key],
                    subjects_by_instance[instance_key],
                    current,
                )
            }
        )
        unknown_basis_refs = set(outcome["basis_refs"]) - set(instance_freshness)
        if unknown_basis_refs:
            raise ValueError(
                "outcome basis ref does not belong to issued action: "
                + ", ".join(sorted(unknown_basis_refs))
            )
        outcome["input_refs"] = instance_inputs
        outcome["freshness_refs"] = instance_freshness


def _output_is_relevant(
    ref: str,
    instance_inputs: dict[str, str],
    selectors: set[str],
    subjects: set[str],
    snapshot: ProjectSnapshot,
) -> bool:
    """Limit Action outputs to outcomes that select and match that source."""

    if ref in instance_inputs:
        return True
    source_kind = ref.split(".", 1)[0]
    if source_kind == "contract":
        if "contracts" in selectors:
            return True
        return "matching-contracts" in selectors and any(
            contract["id"] == ref
            and bool(set(contract.get("applies_to", [])) & subjects)
            for contract in snapshot.contracts
        )
    if source_kind == "decision":
        if "decisions" in selectors:
            return True
        matching_authorities = {
            clause["authority_ref"]
            for contract in snapshot.contracts
            if set(contract.get("applies_to", [])) & subjects
            for clause in contract.get("clauses", [])
            if clause.get("authority_ref")
        }
        return "matching-decisions" in selectors and ref in matching_authorities
    if source_kind == "evidence":
        if "evidence" in selectors:
            return True
        return "matching-evidence" in selectors and any(
            evidence["id"] == ref
            and bool(set(evidence.get("applies_to", [])) & subjects)
            for evidence in snapshot.evidence
        )
    if source_kind == "result":
        return bool({"dependency-results", "results"} & selectors)
    return False
