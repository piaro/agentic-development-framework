"""コード観測結果を、Rule適用前のSignal候補へ変換するDetector。"""

from __future__ import annotations

from .model import (
    DetectionCoverage,
    DetectionReport,
    ProjectSnapshot,
    SignalCandidate,
    canonical_digest,
)
from .versions import DETECTOR_ID, DETECTOR_VERSION


class TypedFactDetector:
    """明示されたfixture factをSignal候補へ変換する決定的Detector。

    Detectorは「該当する可能性」を報告するだけで、Requirementを直接選ばない。
    Signalの確定はResult Recordに残るreviewを経由させる。
    """

    detector_id = DETECTOR_ID
    detector_version = DETECTOR_VERSION

    def detect(self, snapshot: ProjectSnapshot) -> DetectionReport:
        coverage = self._coverage(snapshot.repository.get("coverage"))
        candidates: list[SignalCandidate] = []
        for fact_index, fact in enumerate(snapshot.repository.get("facts", [])):
            if not isinstance(fact, dict):
                raise ValueError(
                    f"repository fact {fact_index} must be an object"
                )
            for signal, bindings in self._signals_for(fact, fact_index):
                evidence_refs = self._evidence_refs(fact, fact_index)
                # 根拠コードまたはDetector versionが変われば別候補として扱う。
                # 逆に同じ根拠なら、過去のreview dispositionを再利用できる。
                fingerprint_body = {
                    "detector_id": self.detector_id,
                    "detector_version": self.detector_version,
                    "signal": signal,
                    "bindings": bindings,
                    "evidence": {
                        ref: snapshot.artifact_digests.get(ref, "missing")
                        for ref in evidence_refs
                    },
                }
                candidates.append(
                    SignalCandidate(
                        signal=signal,
                        bindings=bindings,
                        evidence_refs=evidence_refs,
                        detector_id=self.detector_id,
                        detector_version=self.detector_version,
                        fingerprint=canonical_digest(fingerprint_body),
                    )
                )
        candidates.sort(key=lambda item: (item.signal, item.fingerprint))
        # 入力順に依存しないReport digestを作るため、候補を先にsortする。
        report_body = {
            "coverage": {
                "status": coverage.status,
                "scope": coverage.scope,
                "analyzed_refs": coverage.analyzed_refs,
                "gaps": coverage.gaps,
            },
            "candidates": [
                {
                    "signal": item.signal,
                    "bindings": item.bindings,
                    "evidence_refs": item.evidence_refs,
                    "fingerprint": item.fingerprint,
                }
                for item in candidates
            ],
        }
        return DetectionReport(
            change_id=snapshot.change_id,
            coverage=coverage,
            candidates=tuple(candidates),
            digest=canonical_digest(report_body),
        )

    def _signals_for(
        self,
        fact: dict[str, object],
        fact_index: int,
    ) -> list[tuple[str, dict[str, str]]]:
        kind = fact.get("kind")
        if not isinstance(kind, str):
            raise ValueError(
                f"repository fact {fact_index} field kind must be a string"
            )
        if kind == "db_write":
            return [
                (
                    "persistent-data-write",
                    {
                        "operation": self._required_string(
                            fact,
                            "operation",
                            fact_index,
                        ),
                        "data": self._required_string(
                            fact,
                            "data",
                            fact_index,
                        ),
                    },
                )
            ]
        if kind == "message_publish":
            bindings = {
                "operation": self._required_string(
                    fact,
                    "operation",
                    fact_index,
                ),
                "integration": self._required_string(
                    fact,
                    "integration",
                    fact_index,
                ),
            }
            return [
                ("distributed-effect", bindings),
                ("message-or-event-publish", bindings),
            ]
        raise ValueError(
            f"repository fact {fact_index} has unsupported kind: {kind}"
        )

    def _coverage(self, raw: object) -> DetectionCoverage:
        if raw is None:
            return DetectionCoverage(
                status="incomplete",
                scope="unknown",
                analyzed_refs=(),
                gaps=(
                    {
                        "kind": "coverage-not-reported",
                        "reason": "Detector coverage was not reported",
                    },
                ),
            )
        if not isinstance(raw, dict):
            raise ValueError("repository coverage must be an object")
        expected = {"status", "scope", "analyzed_refs", "gaps"}
        if set(raw) != expected:
            raise ValueError(
                "repository coverage must contain "
                "status, scope, analyzed_refs, gaps"
            )
        status = raw["status"]
        if status not in {"complete", "incomplete"}:
            raise ValueError(
                "repository coverage status must be complete or incomplete"
            )
        scope = raw["scope"]
        if not isinstance(scope, str) or not scope:
            raise ValueError(
                "repository coverage scope must be a non-empty string"
            )
        analyzed_refs = raw["analyzed_refs"]
        if (
            not isinstance(analyzed_refs, list)
            or any(not isinstance(item, str) for item in analyzed_refs)
            or len(analyzed_refs) != len(set(analyzed_refs))
        ):
            raise ValueError(
                "repository coverage analyzed_refs must be unique strings"
            )
        raw_gaps = raw["gaps"]
        if not isinstance(raw_gaps, list):
            raise ValueError("repository coverage gaps must be an array")
        gaps: list[dict[str, str]] = []
        for gap_index, gap in enumerate(raw_gaps):
            if not isinstance(gap, dict):
                raise ValueError(
                    f"repository coverage gap {gap_index} must be an object"
                )
            allowed = {"kind", "ref", "reason"}
            if not {"kind", "reason"} <= set(gap) or not set(gap) <= allowed:
                raise ValueError(
                    f"repository coverage gap {gap_index} must contain "
                    "kind, optional ref, reason"
                )
            if any(not isinstance(value, str) or not value for value in gap.values()):
                raise ValueError(
                    f"repository coverage gap {gap_index} values "
                    "must be non-empty strings"
                )
            gaps.append(dict(gap))
        if status == "complete" and gaps:
            raise ValueError(
                "complete repository coverage cannot contain gaps"
            )
        if status == "incomplete" and not gaps:
            raise ValueError(
                "incomplete repository coverage must contain a gap"
            )
        return DetectionCoverage(
            status=status,
            scope=scope,
            analyzed_refs=tuple(sorted(analyzed_refs)),
            gaps=tuple(
                sorted(
                    gaps,
                    key=lambda gap: (
                        gap["kind"],
                        gap.get("ref", ""),
                        gap["reason"],
                    ),
                )
            ),
        )

    def _required_string(
        self,
        fact: dict[str, object],
        field: str,
        fact_index: int,
    ) -> str:
        value = fact.get(field)
        if not isinstance(value, str):
            raise ValueError(
                f"repository fact {fact_index} field {field} must be a string"
            )
        return value

    def _evidence_refs(
        self,
        fact: dict[str, object],
        fact_index: int,
    ) -> tuple[str, ...]:
        value = fact.get("evidence_refs", [])
        if not isinstance(value, list):
            raise ValueError(
                f"repository fact {fact_index} field evidence_refs must be an array"
            )
        if any(not isinstance(item, str) for item in value):
            raise ValueError(
                f"repository fact {fact_index} field evidence_refs items "
                "must be strings"
            )
        return tuple(sorted(value))
