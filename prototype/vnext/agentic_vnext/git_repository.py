"""Observe declared repository artifacts at a clean Git revision."""

from __future__ import annotations

import hashlib
from pathlib import Path
import subprocess
from typing import Any

import yaml

from .model import canonical_digest


OBSERVATION_SCHEMA_VERSION = "2"


class GitRepositoryAdapter:
    """Convert tracked files and typed fact declarations into Detector input."""

    def __init__(
        self,
        project_root: str | Path,
        manifest_path: str,
        require_clean: bool = True,
    ):
        self.root = Path(project_root).resolve()
        self.manifest_path = self._repository_path(manifest_path)
        self.require_clean = require_clean

    def observe(self) -> dict[str, Any]:
        top_level = Path(
            self._git("rev-parse", "--show-toplevel").strip()
        ).resolve()
        if top_level != self.root:
            raise ValueError(
                f"configured root is not Git top-level: {self.root}"
            )
        if self.require_clean:
            status = self._git(
                "status",
                "--porcelain",
                "--untracked-files=all",
            )
            if status:
                raise ValueError("Git working tree is not clean")

        revision = self._git("rev-parse", "HEAD").strip()
        manifest_relative = self.manifest_path.relative_to(self.root).as_posix()
        self._assert_tracked(manifest_relative)
        with self.manifest_path.open(encoding="utf-8") as stream:
            manifest = yaml.safe_load(stream)
        if not isinstance(manifest, dict):
            raise ValueError("repository observation must be a mapping")
        if str(manifest.get("schema_version")) != OBSERVATION_SCHEMA_VERSION:
            raise ValueError("unsupported repository observation schema")
        if manifest.get("phase") not in {"pre-build", "post-build"}:
            raise ValueError("repository observation phase is invalid")
        expected_fields = {
            "schema_version",
            "phase",
            "artifacts",
            "facts",
            "coverage",
        }
        if set(manifest) != expected_fields:
            raise ValueError(
                "repository observation must contain "
                "schema_version, phase, artifacts, facts, coverage"
            )

        artifacts: list[dict[str, Any]] = []
        artifact_refs: set[str] = set()
        for declaration in manifest.get("artifacts", []):
            if not isinstance(declaration, dict):
                raise ValueError("artifact declaration must be a mapping")
            required = {"ref", "path", "applies_to"}
            if set(declaration) != required:
                raise ValueError(
                    "artifact declaration must contain ref, path, applies_to"
                )
            ref = declaration["ref"]
            if not isinstance(ref, str) or ref in artifact_refs:
                raise ValueError(f"duplicate or invalid artifact ref: {ref!r}")
            artifact_refs.add(ref)
            artifact_path = self._repository_path(declaration["path"])
            artifact_relative = artifact_path.relative_to(self.root).as_posix()
            self._assert_tracked(artifact_relative)
            if not artifact_path.is_file():
                raise ValueError(f"artifact is not a file: {artifact_relative}")
            content_digest = "sha256:" + hashlib.sha256(
                artifact_path.read_bytes()
            ).hexdigest()
            # Include declaration metadata because applies_to changes Context
            # selection even when file bytes remain unchanged.
            digest = canonical_digest(
                {
                    "content_digest": content_digest,
                    "declaration": declaration,
                }
            )
            artifacts.append(
                {
                    "ref": ref,
                    "path": artifact_relative,
                    "applies_to": list(declaration["applies_to"]),
                    "content_digest": content_digest,
                    "digest": digest,
                }
            )

        facts = manifest.get("facts", [])
        if not isinstance(facts, list):
            raise ValueError("repository facts must be a list")
        for fact in facts:
            if not isinstance(fact, dict):
                raise ValueError("repository fact must be a mapping")
            unknown_refs = set(fact.get("evidence_refs", [])) - artifact_refs
            if unknown_refs:
                raise ValueError(
                    "repository fact refers to unknown artifacts: "
                    + ", ".join(sorted(unknown_refs))
                )
        coverage = self._coverage(manifest["coverage"], artifact_refs)
        return {
            "phase": manifest["phase"],
            "revision": revision,
            "artifacts": sorted(artifacts, key=lambda item: item["ref"]),
            "facts": facts,
            "coverage": coverage,
        }

    def _coverage(
        self,
        value: object,
        artifact_refs: set[str],
    ) -> dict[str, Any]:
        if not isinstance(value, dict):
            raise ValueError("repository coverage must be a mapping")
        if set(value) != {"scope", "analyzed_refs", "gaps"}:
            raise ValueError(
                "repository coverage must contain scope, analyzed_refs, gaps"
            )
        if value["scope"] != "declared-artifacts":
            raise ValueError(
                "repository coverage scope must be declared-artifacts"
            )
        analyzed_refs = value["analyzed_refs"]
        if (
            not isinstance(analyzed_refs, list)
            or any(not isinstance(item, str) for item in analyzed_refs)
            or len(analyzed_refs) != len(set(analyzed_refs))
        ):
            raise ValueError(
                "repository coverage analyzed_refs must be unique strings"
            )
        unknown_analyzed = set(analyzed_refs) - artifact_refs
        if unknown_analyzed:
            raise ValueError(
                "repository coverage refers to unknown artifacts: "
                + ", ".join(sorted(unknown_analyzed))
            )
        raw_gaps = value["gaps"]
        if not isinstance(raw_gaps, list):
            raise ValueError("repository coverage gaps must be a list")
        gaps: list[dict[str, str]] = []
        for gap_index, gap in enumerate(raw_gaps):
            if not isinstance(gap, dict):
                raise ValueError(
                    f"repository coverage gap {gap_index} must be a mapping"
                )
            allowed = {"kind", "ref", "reason"}
            if not {"kind", "reason"} <= set(gap) or not set(gap) <= allowed:
                raise ValueError(
                    f"repository coverage gap {gap_index} must contain "
                    "kind, optional ref, reason"
                )
            if any(not isinstance(item, str) or not item for item in gap.values()):
                raise ValueError(
                    f"repository coverage gap {gap_index} values "
                    "must be non-empty strings"
                )
            gaps.append(dict(gap))
        for ref in sorted(artifact_refs - set(analyzed_refs)):
            gaps.append(
                {
                    "kind": "unscanned-artifact",
                    "ref": ref,
                    "reason": "declared artifact was not analyzed",
                }
            )
        gaps.sort(
            key=lambda gap: (
                gap["kind"],
                gap.get("ref", ""),
                gap["reason"],
            )
        )
        return {
            "status": "incomplete" if gaps else "complete",
            "scope": "declared-artifacts",
            "analyzed_refs": sorted(analyzed_refs),
            "gaps": gaps,
        }

    def assert_tracked(self, paths: list[str | Path]) -> None:
        """Require every authoritative input path to exist in the Git index."""

        for path in paths:
            candidate = self._repository_path(str(path))
            relative = candidate.relative_to(self.root).as_posix()
            self._assert_tracked(relative)

    def _assert_tracked(self, relative_path: str) -> None:
        self._git("ls-files", "--error-unmatch", "--", relative_path)

    def _repository_path(self, relative: str) -> Path:
        if not isinstance(relative, str):
            raise ValueError("repository path must be a string")
        path = Path(relative)
        if path.is_absolute():
            raise ValueError(f"path must be repository-relative: {relative}")
        resolved = (self.root / path).resolve()
        try:
            resolved.relative_to(self.root)
        except ValueError as error:
            raise ValueError(f"path escapes repository: {relative}") from error
        return resolved

    def _git(self, *arguments: str) -> str:
        completed = subprocess.run(
            ["git", "-C", str(self.root), *arguments],
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            message = completed.stderr.strip() or completed.stdout.strip()
            raise ValueError(
                f"Git command failed ({' '.join(arguments)}): {message}"
            )
        return completed.stdout
