"""Load the small Git-managed configuration required by Project adapters."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml


CONFIG_SCHEMA_VERSION = "1"


@dataclass(frozen=True)
class ProjectConfig:
    contract_root: str
    decision_root: str
    repository_observation: str


def load_project_config(
    project_root: str | Path,
    relative_path: str = ".agentic/config.yaml",
) -> ProjectConfig:
    root = Path(project_root).resolve()
    path = _repository_path(root, relative_path)
    with path.open(encoding="utf-8") as stream:
        value = yaml.safe_load(stream)
    if not isinstance(value, dict):
        raise ValueError("project config must be a mapping")

    expected_fields = {
        "schema_version",
        "project_sources",
        "repository_observation",
    }
    unexpected = set(value) - expected_fields
    missing = expected_fields - set(value)
    if unexpected or missing:
        details = [
            *(f"unexpected field: {field}" for field in sorted(unexpected)),
            *(f"missing field: {field}" for field in sorted(missing)),
        ]
        raise ValueError("invalid project config: " + ", ".join(details))
    if str(value["schema_version"]) != CONFIG_SCHEMA_VERSION:
        raise ValueError(
            "unsupported project config schema: "
            f"{value['schema_version']}"
        )

    sources = value["project_sources"]
    if not isinstance(sources, dict) or set(sources) != {
        "contracts",
        "decisions",
    }:
        raise ValueError(
            "project_sources must contain only contracts and decisions"
        )
    for field in ("contracts", "decisions"):
        if not isinstance(sources[field], str):
            raise ValueError(f"project_sources.{field} must be a string")
    if not isinstance(value["repository_observation"], str):
        raise ValueError("repository_observation must be a string")

    # Resolve once here to reject absolute and escaping paths before any adapter
    # reads from the filesystem. Individual adapters apply stricter path policies.
    for configured_path in (
        sources["contracts"],
        sources["decisions"],
        value["repository_observation"],
    ):
        _repository_path(root, configured_path)
    return ProjectConfig(
        contract_root=sources["contracts"],
        decision_root=sources["decisions"],
        repository_observation=value["repository_observation"],
    )


def _repository_path(root: Path, relative: str) -> Path:
    path = Path(relative)
    if path.is_absolute():
        raise ValueError(f"configured path must be repository-relative: {relative}")
    resolved = (root / path).resolve()
    try:
        resolved.relative_to(root)
    except ValueError as error:
        raise ValueError(
            f"configured path escapes repository: {relative}"
        ) from error
    return resolved
