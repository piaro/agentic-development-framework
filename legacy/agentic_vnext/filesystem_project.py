"""Filesystem-backed Project Store for Git-managed records."""

from __future__ import annotations

from contextlib import contextmanager
from copy import deepcopy
import json
import os
from pathlib import Path
import re
import tempfile
from typing import Any, Iterator

import yaml

from .markdown_records import (
    create_markdown_record,
    parse_markdown_record,
    replace_markdown_record,
)
from .model import ProjectSnapshot
from .project import build_project_snapshot, validate_contract_update
from .schema import validate_record


_SAFE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
_DISALLOWED_SOURCE_ROOTS = {
    ".agentic/cache",
    ".agentic/bundles",
    ".agentic/local",
    ".agentic/logs",
    ".agentic/tmp",
}
FILESYSTEM_PROJECT_PROTOCOL_VERSION = "2"


class FileProjectStore:
    """Read Git-managed records and append validated Application results.

    Repository code observations are supplied separately because a production
    Detector derives them from the current Git revision; they are not duplicated
    as Project records by this Store.
    """

    def __init__(
        self,
        root: str | Path,
        repository: dict[str, Any],
        contract_root: str = ".agentic/contracts",
        decision_root: str = ".agentic/decisions",
        default_document_format: str = "auto",
    ):
        self.root = Path(root).resolve()
        if not self.root.is_dir():
            raise ValueError(f"project root does not exist: {self.root}")
        self.contract_root = self._source_root(contract_root)
        self.decision_root = self._source_root(decision_root)
        self.change_root = self.root / ".agentic" / "changes"
        self._repository = deepcopy(repository)
        if default_document_format not in {"auto", "yaml", "markdown"}:
            raise ValueError(
                "default_document_format must be auto, yaml, or markdown"
            )
        self.default_document_format = default_document_format

    @classmethod
    def initialize(
        cls,
        root: str | Path,
        project: dict[str, Any],
        contract_root: str = ".agentic/contracts",
        decision_root: str = ".agentic/decisions",
        document_format: str = "yaml",
    ) -> "FileProjectStore":
        """Create fixture-compatible records without overwriting existing files."""

        project_root = Path(root).resolve()
        project_root.mkdir(parents=True, exist_ok=True)
        store = cls(
            project_root,
            project.get("repository", {}),
            contract_root,
            decision_root,
            default_document_format=document_format,
        )
        if document_format not in {"yaml", "markdown"}:
            raise ValueError("document_format must be yaml or markdown")
        # Reject the complete input before creating the first file. This keeps
        # initialization all-or-nothing for schema errors.
        for collection, record_kind in (
            ("changes", "change"),
            ("contracts", "contract"),
            ("decisions", "decision"),
            ("results", "result"),
            ("evidence", "evidence"),
        ):
            for value in project.get(collection, []):
                validate_record(record_kind, value)
        document_extension = "md" if document_format == "markdown" else "yaml"
        targets: list[tuple[Path, dict[str, Any], str]] = []
        for change in project.get("changes", []):
            change_id = store._safe_id(change["id"])
            change_name = (
                "change.md"
                if document_format == "markdown"
                else "change.yaml"
            )
            targets.append(
                (
                    store.change_root / change_id / change_name,
                    change,
                    "change-markdown"
                    if document_format == "markdown"
                    else "yaml",
                )
            )
        for contract in project.get("contracts", []):
            targets.append(
                (
                    store.contract_root
                    / (
                        f"{store._safe_id(contract['id'])}."
                        f"{document_extension}"
                    ),
                    contract,
                    "contract-markdown"
                    if document_format == "markdown"
                    else "yaml",
                )
            )
        for decision in project.get("decisions", []):
            targets.append(
                (
                    store.decision_root
                    / (
                        f"{store._safe_id(decision['id'])}."
                        f"{document_extension}"
                    ),
                    decision,
                    "decision-markdown"
                    if document_format == "markdown"
                    else "yaml",
                )
            )
        for result in project.get("results", []):
            change_id = store._safe_id(result["change_id"])
            targets.append(
                (
                    store.change_root
                    / change_id
                    / "results"
                    / store._result_filename(result),
                    result,
                    "json",
                )
            )
        for evidence in project.get("evidence", []):
            change_id = store._safe_id(evidence["change_id"])
            evidence_id = store._safe_id(evidence["id"])
            targets.append(
                (
                    store.change_root
                    / change_id
                    / "evidence"
                    / f"{evidence_id}.json",
                    evidence,
                    "json",
                )
            )

        target_paths = [path for path, _, _ in targets]
        duplicate_targets = sorted(
            str(path)
            for path in set(target_paths)
            if target_paths.count(path) > 1
        )
        if duplicate_targets:
            raise ValueError(
                "project records resolve to duplicate files: "
                + ", ".join(duplicate_targets)
            )
        conflicts = sorted(str(path) for path in target_paths if path.exists())
        if conflicts:
            raise ValueError(
                "project initialization would overwrite existing files: "
                + ", ".join(conflicts)
            )
        for path, value, file_format in targets:
            store._write_new(path, value, file_format)
        return store

    def snapshot(self, change_id: str) -> ProjectSnapshot:
        safe_change_id = self._safe_id(change_id)
        change_path = self._change_path(safe_change_id)

        project = {
            "changes": [self._load_document(change_path, "change")],
            "contracts": self._load_document_records(
                self.contract_root,
                "contract",
            ),
            "decisions": self._load_document_records(
                self.decision_root,
                "decision",
            ),
            "results": self._load_json_records(
                self.change_root / safe_change_id / "results"
            ),
            "evidence": self._load_json_records(
                self.change_root / safe_change_id / "evidence"
            ),
            "repository": deepcopy(self._repository),
        }
        return build_project_snapshot(project, change_id)

    def record_paths(self, change_id: str) -> tuple[Path, ...]:
        """Return every Project record used to build the requested Snapshot."""

        safe_change_id = self._safe_id(change_id)
        change_directory = self.change_root / safe_change_id
        change_path = self._change_path(safe_change_id)
        paths = [
            change_path,
            *(
                self._document_paths(self.contract_root)
                if self.contract_root.is_dir()
                else []
            ),
            *(
                self._document_paths(self.decision_root)
                if self.decision_root.is_dir()
                else []
            ),
            *sorted((change_directory / "results").glob("*.json")),
            *sorted((change_directory / "evidence").glob("*.json")),
        ]
        return tuple(path for path in paths if path.is_file())

    def append_result(self, result: dict[str, Any]) -> None:
        """Append one Result per Action using exclusive file creation."""

        validate_record("result", result)
        change_id = self._safe_id(result["change_id"])
        self._change_path(change_id)
        path = (
            self.change_root
            / change_id
            / "results"
            / self._result_filename(result)
        )
        self._write_new(path, result, "json")

    def upsert_contract(
        self,
        contract: dict[str, Any],
        expected_digest: str | None = None,
    ) -> None:
        validate_record("contract", contract)
        with self._contract_update_lock():
            existing_path = self._record_path(
                self.contract_root,
                self._safe_id(contract["id"]),
                "contract",
            )
            existing = (
                self._load_document(existing_path, "contract")
                if existing_path is not None
                else None
            )
            validate_contract_update(existing, contract, expected_digest)
            self._upsert_document_record(
                self.contract_root,
                contract,
                "contract",
                existing_path=existing_path,
            )

    def upsert_decision(self, decision: dict[str, Any]) -> None:
        validate_record("decision", decision)
        self._upsert_document_record(
            self.decision_root,
            decision,
            "decision",
        )

    def add_evidence(self, evidence: dict[str, Any]) -> None:
        validate_record("evidence", evidence)
        change_id = self._safe_id(evidence["change_id"])
        evidence_id = self._safe_id(evidence["id"])
        self._change_path(change_id)
        path = (
            self.change_root
            / change_id
            / "evidence"
            / f"{evidence_id}.json"
        )
        self._write_atomic(path, evidence, "json")

    def update_repository(self, repository: dict[str, Any]) -> None:
        """Replace the current Git observation supplied by the Detector adapter."""

        self._repository = deepcopy(repository)

    def _upsert_document_record(
        self,
        source_root: Path,
        value: dict[str, Any],
        record_kind: str,
        existing_path: Path | None = None,
    ) -> None:
        record_id = self._safe_id(value["id"])
        existing = existing_path or self._record_path(
            source_root,
            record_id,
            record_kind,
        )
        if existing is not None:
            file_format = (
                f"{record_kind}-markdown"
                if existing.suffix == ".md"
                else "yaml"
            )
            path = existing
        else:
            preferred = self._preferred_document_format()
            path = source_root / (
                f"{record_id}.md"
                if preferred == "markdown"
                else f"{record_id}.yaml"
            )
            file_format = (
                f"{record_kind}-markdown"
                if preferred == "markdown"
                else "yaml"
            )
        self._write_atomic(path, value, file_format)

    @contextmanager
    def _contract_update_lock(self) -> Iterator[None]:
        """Serialize compare-and-replace without making the lock authoritative."""

        path = (
            self.root
            / ".agentic"
            / "cache"
            / "locks"
            / "contract-updates.lock"
        )
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("a+b") as stream:
            if os.name == "nt":
                import msvcrt

                if stream.seek(0, os.SEEK_END) == 0:
                    stream.write(b"\0")
                    stream.flush()
                stream.seek(0)
                msvcrt.locking(stream.fileno(), msvcrt.LK_LOCK, 1)
                try:
                    yield
                finally:
                    stream.seek(0)
                    msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
                try:
                    yield
                finally:
                    fcntl.flock(stream.fileno(), fcntl.LOCK_UN)

    def _record_path(
        self,
        source_root: Path,
        record_id: str,
        record_kind: str,
    ) -> Path | None:
        matches = [
            path
            for path in self._document_paths(source_root)
            if self._load_document(path, record_kind).get("id") == record_id
        ] if source_root.is_dir() else []
        if len(matches) > 1:
            raise ValueError(f"duplicate record id: {record_id}")
        return matches[0] if matches else None

    def _load_document_records(
        self,
        source_root: Path,
        record_kind: str,
    ) -> list[dict[str, Any]]:
        if not source_root.is_dir():
            return []
        values = [
            self._load_document(path, record_kind)
            for path in self._document_paths(source_root)
        ]
        self._assert_unique_ids(values)
        return values

    def _load_json_records(self, source_root: Path) -> list[dict[str, Any]]:
        if not source_root.is_dir():
            return []
        values = [
            self._load_json(path)
            for path in sorted(source_root.glob("*.json"))
        ]
        self._assert_unique_ids(values)
        return values

    def _load_yaml(self, path: Path) -> dict[str, Any]:
        with path.open(encoding="utf-8") as stream:
            value = yaml.safe_load(stream)
        return self._mapping(value, path)

    def _load_document(
        self,
        path: Path,
        record_kind: str,
    ) -> dict[str, Any]:
        if path.suffix == ".md":
            return parse_markdown_record(
                path.read_text(encoding="utf-8"),
                record_kind,
            )
        return self._load_yaml(path)

    def _load_json(self, path: Path) -> dict[str, Any]:
        with path.open(encoding="utf-8") as stream:
            value = json.load(stream)
        return self._mapping(value, path)

    def _mapping(self, value: Any, path: Path) -> dict[str, Any]:
        if not isinstance(value, dict):
            raise ValueError(f"record must be a mapping: {path}")
        return value

    def _assert_unique_ids(self, values: list[dict[str, Any]]) -> None:
        seen: set[str] = set()
        for value in values:
            record_id = value.get("id")
            if not isinstance(record_id, str):
                raise ValueError("record id must be a string")
            if record_id in seen:
                raise ValueError(f"duplicate record id: {record_id}")
            seen.add(record_id)

    def _source_root(self, relative: str) -> Path:
        relative_path = Path(relative)
        if relative_path.is_absolute():
            raise ValueError("project source root must be repository-relative")
        normalized = relative_path.as_posix().rstrip("/")
        if any(
            normalized == blocked or normalized.startswith(blocked + "/")
            for blocked in _DISALLOWED_SOURCE_ROOTS
        ):
            raise ValueError(
                f"project source root cannot use generated/local path: {relative}"
            )
        candidate = (self.root / relative_path).resolve()
        try:
            candidate.relative_to(self.root)
        except ValueError as error:
            raise ValueError(
                f"project source root escapes repository: {relative}"
            ) from error
        return candidate

    def _safe_id(self, value: str) -> str:
        if not isinstance(value, str) or not _SAFE_ID.fullmatch(value):
            raise ValueError(f"unsafe record id: {value!r}")
        return value

    def _result_filename(self, result: dict[str, Any]) -> str:
        """Identify one issued Action version, not only its reusable Action ID."""

        action_id = self._safe_id(result["action_id"])
        context_digest = result["context_digest"]
        if (
            not isinstance(context_digest, str)
            or not re.fullmatch(r"sha256:[0-9a-f]{64}", context_digest)
        ):
            raise ValueError("unsafe Result context digest")
        return f"{action_id}.{context_digest.removeprefix('sha256:')}.json"

    def _write_new(
        self,
        path: Path,
        value: dict[str, Any],
        file_format: str,
    ) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        serialized = self._serialize(value, file_format)
        try:
            with path.open("x", encoding="utf-8") as stream:
                stream.write(serialized)
                stream.flush()
                os.fsync(stream.fileno())
        except FileExistsError as error:
            raise ValueError(f"record already exists: {path}") from error

    def _write_atomic(
        self,
        path: Path,
        value: dict[str, Any],
        file_format: str,
    ) -> None:
        """Replace one explicitly targeted record without exposing partial data."""

        path.parent.mkdir(parents=True, exist_ok=True)
        existing_text = (
            path.read_text(encoding="utf-8")
            if path.is_file() and file_format.endswith("-markdown")
            else None
        )
        serialized = self._serialize(
            value,
            file_format,
            existing_text=existing_text,
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

    def _serialize(
        self,
        value: dict[str, Any],
        file_format: str,
        existing_text: str | None = None,
    ) -> str:
        if file_format == "json":
            return (
                json.dumps(
                    value,
                    ensure_ascii=False,
                    indent=2,
                    sort_keys=True,
                )
                + "\n"
            )
        if file_format.endswith("-markdown"):
            record_kind = file_format.removesuffix("-markdown")
            if existing_text is not None:
                return replace_markdown_record(
                    existing_text,
                    value,
                    record_kind,
                )
            return create_markdown_record(value, record_kind)
        return yaml.safe_dump(
            value,
            allow_unicode=True,
            sort_keys=False,
        )

    def _change_path(self, change_id: str) -> Path:
        change_directory = self.change_root / change_id
        candidates = [
            path
            for path in (
                change_directory / "change.md",
                change_directory / "change.yaml",
            )
            if path.is_file()
        ]
        if not candidates:
            raise ValueError(f"unknown change: {change_id}")
        if len(candidates) > 1:
            raise ValueError(f"multiple Change records: {change_id}")
        return candidates[0]

    def _document_paths(self, source_root: Path) -> list[Path]:
        return sorted(
            [
                *source_root.rglob("*.md"),
                *source_root.rglob("*.yaml"),
            ]
        )

    def _preferred_document_format(self) -> str:
        if self.default_document_format != "auto":
            return self.default_document_format
        if (
            any(self.contract_root.rglob("*.md"))
            or any(self.decision_root.rglob("*.md"))
            or any(self.change_root.glob("*/change.md"))
        ):
            return "markdown"
        return "yaml"
