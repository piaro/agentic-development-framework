"""Module間で受け渡す、永続形式に依存しない内部データ型。"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
import hashlib
import json
from typing import Any


def canonical_json(value: Any) -> str:
    """Serialize one value using the cross-language canonical JSON contract.

    The v1 contract uses UTF-8, unescaped Unicode, recursively sorted object
    keys, no insignificant whitespace, and the standard JSON literals. Golden
    vectors intentionally exclude floating-point values until number
    normalization is specified independently of a runtime.
    """

    if _contains_float(value):
        raise ValueError(
            "canonical-json-v1: floating-point-not-supported"
        )
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )


def canonical_digest(value: Any) -> str:
    """Return the SHA-256 identity of the canonical UTF-8 JSON bytes."""

    encoded = canonical_json(value).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def _contains_float(value: Any) -> bool:
    if isinstance(value, float):
        return True
    if isinstance(value, (list, tuple)):
        return any(_contains_float(item) for item in value)
    if isinstance(value, dict):
        return any(_contains_float(item) for item in value.values())
    return False


@dataclass(frozen=True)
class SignalCandidate:
    """Detectorが見つけた、まだ規範適用を確定していない候補。"""

    signal: str
    bindings: dict[str, str]
    evidence_refs: tuple[str, ...]
    detector_id: str
    detector_version: str
    fingerprint: str


@dataclass(frozen=True)
class DetectionCoverage:
    """Detectorが解析できた範囲と、未解決のgap。"""

    status: str
    scope: str
    analyzed_refs: tuple[str, ...]
    gaps: tuple[dict[str, str], ...]


@dataclass(frozen=True)
class DetectionReport:
    change_id: str
    coverage: DetectionCoverage
    candidates: tuple[SignalCandidate, ...]
    digest: str


@dataclass(frozen=True)
class RequirementDefinition:
    id: str
    phase: str
    role: str
    result_schema: str
    depends_on: tuple[str, ...] = ()
    context: tuple[str, ...] = ()
    definition_digest: str = ""


@dataclass(frozen=True)
class ActivationRule:
    id: str
    requirement_id: str
    condition: str
    signal: str | None = None
    repository_phase: str | None = None
    subjects: tuple[str, ...] = ()


@dataclass(frozen=True)
class RuleIndex:
    requirements: dict[str, RequirementDefinition]
    rules: tuple[ActivationRule, ...]
    digest: str


@dataclass(frozen=True)
class RequirementInstance:
    """Requirement定義を具体的な対象へ適用した実行時の単位。"""

    requirement_id: str
    subject_refs: tuple[str, ...]
    instance_key: str
    selected_by: tuple[str, ...]
    definition_digest: str
    phase: str
    role: str
    result_schema: str
    depends_on: tuple[str, ...]
    context: tuple[str, ...]
    status: str = "unsatisfied"


@dataclass(frozen=True)
class ProjectSnapshot:
    """一回のKernel評価で参照するProject正本の不変Snapshot。"""

    change_id: str
    change: dict[str, Any]
    contracts: tuple[dict[str, Any], ...]
    decisions: tuple[dict[str, Any], ...]
    results: tuple[dict[str, Any], ...]
    evidence: tuple[dict[str, Any], ...]
    repository: dict[str, Any]
    artifact_digests: dict[str, str]
    digest: str


@dataclass(frozen=True)
class NextAction:
    id: str
    role: str
    action: str
    requirement_instances: tuple[RequirementInstance, ...]
    reason: str
    expected_result_schema: str
    # Populated only for risk review actions so the Context Compiler can expose
    # the exact candidate delta selected by the Kernel.
    candidate_fingerprints: tuple[str, ...] = ()


@dataclass(frozen=True)
class KernelDecision:
    """現在から導出したState、NextAction、判定根拠の集合。"""

    state: str
    action: NextAction | None
    requirement_instances: tuple[RequirementInstance, ...]
    diagnostics: tuple[str, ...] = ()

    def as_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class GeneratedContext:
    """一つのNextActionを実行するAgentまたはHuman向けの中間生成物。"""

    action_id: str
    role: str
    source_refs: tuple[str, ...]
    source_digests: dict[str, str]
    # Each outcome carries only the sources selected for its Requirement Instance.
    # The action-level source_digests above remains their union for submit validation.
    instance_source_digests: dict[str, dict[str, str]]
    payload: dict[str, Any] = field(compare=True)
    digest: str = ""
