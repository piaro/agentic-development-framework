"""Create and validate the technical inputs pinned by a Framework lock."""

from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass
from pathlib import Path
import re
from typing import Any

import yaml

from .model import RuleIndex, canonical_digest
from .schema import SCHEMA_BUNDLE_VERSION, default_schema_registry
from .versions import (
    APPLICATION_PROTOCOL_VERSION,
    CANONICALIZATION_VERSION,
    CONTEXT_COMPILER_VERSION,
    DATA_MODEL_VERSION,
    DETECTOR_ID,
    DETECTOR_VERSION,
    EXPLANATION_VERSION,
    FRAMEWORK_LOCK_SCHEMA_VERSION,
    FRAMEWORK_RELEASE,
    KERNEL_VERSION,
    PROJECT_SNAPSHOT_PROTOCOL_VERSION,
    RULE_COMPILER_VERSION,
    SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION,
)


@dataclass(frozen=True)
class FrameworkLock:
    """Validated immutable view of the runtime and Rule set identity."""

    manifest: dict[str, Any]
    digest: str

    def as_dict(self) -> dict[str, Any]:
        return deepcopy(self.manifest)


def build_framework_lock(
    rule_source: dict[str, Any],
    rule_index: RuleIndex,
) -> dict[str, Any]:
    """Build the exact lock expected by this runtime.

    Generation is a technical operation: it records versions and content digests
    but never decides whether a Rule change is semantically acceptable.
    """

    return {
        "schema_version": FRAMEWORK_LOCK_SCHEMA_VERSION,
        "framework_release": FRAMEWORK_RELEASE,
        "protocols": {
            "application": APPLICATION_PROTOCOL_VERSION,
            "canonicalization": CANONICALIZATION_VERSION,
            "context_compiler": CONTEXT_COMPILER_VERSION,
            "data_model": DATA_MODEL_VERSION,
            "explanation": EXPLANATION_VERSION,
            "kernel": KERNEL_VERSION,
            "project_snapshot": PROJECT_SNAPSHOT_PROTOCOL_VERSION,
            "rule_compiler": RULE_COMPILER_VERSION,
        },
        "detectors": {
            DETECTOR_ID: DETECTOR_VERSION,
        },
        "schema_bundle": {
            "version": SCHEMA_BUNDLE_VERSION,
            "digest": default_schema_registry().digest,
        },
        "rule_set": {
            "source_digest": canonical_digest(rule_source),
            "index_digest": rule_index.digest,
        },
    }


def validate_framework_lock(
    lock_source: dict[str, Any],
    rule_source: dict[str, Any],
    rule_index: RuleIndex,
) -> FrameworkLock:
    """Reject any runtime or Rule input that differs from the pinned manifest."""

    expected = build_framework_lock(rule_source, rule_index)
    differences = _differences(expected, _comparable_core_lock(lock_source))
    if differences:
        raise ValueError(
            "framework lock mismatch:\n- " + "\n- ".join(differences)
        )
    manifest = deepcopy(lock_source)
    return FrameworkLock(
        manifest=manifest,
        digest=canonical_digest(manifest),
    )


def _comparable_core_lock(lock_source: dict[str, Any]) -> dict[str, Any]:
    """Remove only the delivery-owned v2 extension before runtime comparison."""

    version = lock_source.get("schema_version")
    if version == FRAMEWORK_LOCK_SCHEMA_VERSION:
        return deepcopy(lock_source)
    if version != SIGNED_FRAMEWORK_LOCK_SCHEMA_VERSION:
        raise ValueError(f"unsupported framework lock Schema: {version!r}")
    artifact = lock_source.get("release_artifact")
    expected_fields = {"artifact_digest", "source_id", "signer_key_id"}
    if not isinstance(artifact, dict) or set(artifact) != expected_fields:
        raise ValueError(
            "framework lock v2 release_artifact fields must be exactly "
            f"{sorted(expected_fields)!r}"
        )
    for field in expected_fields:
        if not isinstance(artifact[field], str) or not artifact[field]:
            raise ValueError(
                f"framework lock release_artifact.{field} "
                "must be a non-empty string"
            )
    if re.fullmatch(r"sha256:[0-9a-fA-F]{64}", artifact["artifact_digest"]) is None:
        raise ValueError(
            "framework lock release_artifact.artifact_digest "
            "must be a SHA-256 digest"
        )
    comparable = deepcopy(lock_source)
    comparable["schema_version"] = FRAMEWORK_LOCK_SCHEMA_VERSION
    del comparable["release_artifact"]
    return comparable


def load_framework_lock(path: str | Path) -> dict[str, Any]:
    with Path(path).open(encoding="utf-8") as stream:
        value = yaml.safe_load(stream)
    if not isinstance(value, dict):
        raise ValueError("framework lock must be a mapping")
    return value


def _differences(
    expected: Any,
    actual: Any,
    path: str = "",
) -> list[str]:
    """Return stable, field-level differences for actionable startup errors."""

    if isinstance(expected, dict) and isinstance(actual, dict):
        differences: list[str] = []
        for key in sorted(set(expected) | set(actual)):
            child_path = f"{path}.{key}" if path else str(key)
            if key not in expected:
                differences.append(f"{child_path}: unexpected field")
            elif key not in actual:
                differences.append(f"{child_path}: missing field")
            else:
                differences.extend(
                    _differences(expected[key], actual[key], child_path)
                )
        return differences
    if expected != actual:
        return [f"{path}: expected {expected!r}, got {actual!r}"]
    return []
