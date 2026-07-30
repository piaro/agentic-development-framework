"""Disposable filesystem cache for deterministic evaluation artifacts."""

from __future__ import annotations

from dataclasses import asdict
import json
import os
from pathlib import Path
import re
import tempfile
from typing import Protocol

from .model import (
    DetectionReport,
    KernelDecision,
    ProjectSnapshot,
    RuleIndex,
    canonical_digest,
)


_SAFE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
CACHE_SCHEMA_VERSION = "1"


class EvaluationCache(Protocol):
    """Optional output boundary; cache data is never a Kernel input."""

    def write_evaluation(
        self,
        framework_lock_digest: str,
        snapshot: ProjectSnapshot,
        rule_index: RuleIndex,
        detection: DetectionReport,
        decision: KernelDecision,
    ) -> None:
        ...


class DerivedCache:
    """Write regenerated artifacts below `.agentic/cache/`.

    This class intentionally has no read API. Introducing cache reads later must
    validate every source digest before a cached value can influence execution.
    """

    def __init__(self, project_root: str | Path):
        self.project_root = Path(project_root).resolve()
        if not self.project_root.is_dir():
            raise ValueError(f"project root does not exist: {self.project_root}")
        self.cache_root = (
            self.project_root / ".agentic" / "cache"
        ).resolve()
        try:
            self.cache_root.relative_to(self.project_root)
        except ValueError as error:
            raise ValueError("cache root escapes repository") from error

    def write_evaluation(
        self,
        framework_lock_digest: str,
        snapshot: ProjectSnapshot,
        rule_index: RuleIndex,
        detection: DetectionReport,
        decision: KernelDecision,
    ) -> None:
        change_id = self._safe_id(snapshot.change_id)
        lock_key = framework_lock_digest.removeprefix("sha256:")[:24]
        if not re.fullmatch(r"[0-9a-f]{24}", lock_key):
            raise ValueError("invalid Framework lock digest")

        project_path = Path("project") / f"{change_id}.json"
        rules_path = Path("rules") / f"{lock_key}.json"
        detection_path = Path("detection") / f"{change_id}.json"
        state_path = Path("state") / f"{change_id}.json"
        manifest_path = Path("manifests") / f"{change_id}.json"

        project_body = asdict(snapshot)
        rules_body = asdict(rule_index)
        detection_body = asdict(detection)
        state_body = decision.as_dict()
        manifest = {
            "schema_version": CACHE_SCHEMA_VERSION,
            "change_id": snapshot.change_id,
            "framework_lock_digest": framework_lock_digest,
            "inputs": {
                "project_snapshot_digest": snapshot.digest,
                "rule_index_digest": rule_index.digest,
                "detection_report_digest": detection.digest,
            },
            "outputs": {
                "kernel_decision_digest": canonical_digest(state_body),
            },
            "files": {
                "project_snapshot": project_path.as_posix(),
                "rule_index": rules_path.as_posix(),
                "detection_report": detection_path.as_posix(),
                "kernel_decision": state_path.as_posix(),
            },
        }

        # Write the manifest last. Its presence then means all referenced cache
        # files were fully replaced, although none of them is authoritative.
        self._write_json(project_path, project_body)
        self._write_json(rules_path, rules_body)
        self._write_json(detection_path, detection_body)
        self._write_json(state_path, state_body)
        self._write_json(manifest_path, manifest)

    def _safe_id(self, value: str) -> str:
        if not isinstance(value, str) or not _SAFE_ID.fullmatch(value):
            raise ValueError(f"unsafe cache key: {value!r}")
        return value

    def _write_json(self, relative_path: Path, value: object) -> None:
        path = self.cache_root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        serialized = (
            json.dumps(
                value,
                ensure_ascii=False,
                indent=2,
                sort_keys=True,
            )
            + "\n"
        )
        descriptor, temporary_name = tempfile.mkstemp(
            prefix=f".{path.name}.",
            suffix=".tmp",
            dir=path.parent,
            text=True,
        )
        temporary = Path(temporary_name)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                stream.write(serialized)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, path)
        finally:
            temporary.unlink(missing_ok=True)
