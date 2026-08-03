"""Project内の正本を読み書きするAdapterのPrototype実装。"""

from __future__ import annotations

from copy import deepcopy
from pathlib import Path
import threading
from typing import Any, Protocol

import yaml

from .model import ProjectSnapshot, canonical_digest
from .schema import validate_record


def _by_change(items: list[dict[str, Any]], change_id: str) -> list[dict[str, Any]]:
    return [
        item
        for item in items
        if item.get("change_id") in (None, change_id)
    ]


class ProjectStore(Protocol):
    """Persistence boundary required by the Application service."""

    def snapshot(self, change_id: str) -> ProjectSnapshot:
        ...

    def append_result(self, result: dict[str, Any]) -> None:
        ...

    def upsert_contract(
        self,
        contract: dict[str, Any],
        expected_digest: str | None = None,
    ) -> None:
        ...


class InMemoryProjectStore:
    """Prototype用のProject Adapter。永続化は行わない。

    Kernelが保存形式を知らなくてよいことを検証するため、fixtureの辞書を
    Snapshotへ変換する責務をこのAdapterへ隔離している。
    """

    def __init__(self, project: dict[str, Any]):
        self._project = deepcopy(project)
        self._contract_lock = threading.Lock()

    def snapshot(self, change_id: str) -> ProjectSnapshot:
        """現在値をdeep copyし、一回の評価中に変化しないSnapshotを作る。"""

        return build_project_snapshot(self._project, change_id)

    def append_result(self, result: dict[str, Any]) -> None:
        """Resultを追記する。既存Resultは更新せず、判断履歴を保持する。"""

        validate_record("result", result)
        if any(item["id"] == result["id"] for item in self._project.get("results", [])):
            raise ValueError(f"duplicate result: {result['id']}")
        self._project.setdefault("results", []).append(deepcopy(result))

    def upsert_contract(
        self,
        contract: dict[str, Any],
        expected_digest: str | None = None,
    ) -> None:
        validate_record("contract", contract)
        with self._contract_lock:
            existing = self._find("contracts", contract["id"])
            validate_contract_update(existing, contract, expected_digest)
            self._upsert("contracts", contract)

    def upsert_decision(self, decision: dict[str, Any]) -> None:
        validate_record("decision", decision)
        self._upsert("decisions", decision)

    def update_repository(self, repository: dict[str, Any]) -> None:
        self._project["repository"] = deepcopy(repository)

    def add_evidence(self, evidence: dict[str, Any]) -> None:
        validate_record("evidence", evidence)
        self._upsert("evidence", evidence)

    def _upsert(self, collection: str, value: dict[str, Any]) -> None:
        items = self._project.setdefault(collection, [])
        for index, item in enumerate(items):
            if item["id"] == value["id"]:
                items[index] = deepcopy(value)
                return
        items.append(deepcopy(value))

    def _find(
        self,
        collection: str,
        record_id: str,
    ) -> dict[str, Any] | None:
        return next(
            (
                item
                for item in self._project.get(collection, [])
                if item["id"] == record_id
            ),
            None,
        )


def validate_contract_update(
    existing: dict[str, Any] | None,
    proposed: dict[str, Any],
    expected_digest: str | None,
) -> None:
    """Reject lost updates when a Shared Contract identity already exists."""

    contract_id = proposed["id"]
    if existing is None:
        if expected_digest is not None:
            raise ValueError(
                "stale Contract update: "
                f"{contract_id}: expected {expected_digest}, current record is missing"
            )
        return

    shared_update = (
        existing.get("change_id") is None
        or proposed.get("change_id") is None
    )
    if expected_digest is None:
        if shared_update:
            raise ValueError(
                f"Shared Contract update requires expected digest: {contract_id}"
            )
        return

    current_digest = canonical_digest(existing)
    if expected_digest != current_digest:
        raise ValueError(
            "stale Contract update: "
            f"{contract_id}: expected {expected_digest}, current {current_digest}"
        )


def load_project(path: str | Path) -> dict[str, Any]:
    with Path(path).open(encoding="utf-8") as stream:
        value = yaml.safe_load(stream)
    if not isinstance(value, dict):
        raise ValueError("project fixture must be a mapping")
    return value


def build_project_snapshot(
    project: dict[str, Any],
    change_id: str,
) -> ProjectSnapshot:
    """Normalize records from any Project Store into the Kernel input model."""

    # Validate at the Adapter boundary even when records came from an external
    # implementation. Kernel therefore receives one language-neutral shape.
    for collection, record_kind in (
        ("changes", "change"),
        ("contracts", "contract"),
        ("decisions", "decision"),
        ("results", "result"),
        ("evidence", "evidence"),
    ):
        for item in project.get(collection, []):
            validate_record(record_kind, item)

    changes = {
        item["id"]: item for item in project.get("changes", [])
    }
    if change_id not in changes:
        raise ValueError(f"unknown change: {change_id}")

    change = deepcopy(changes[change_id])
    # Physical file order must not affect Snapshot or Kernel output.
    contracts = tuple(
        sorted(
            deepcopy(_by_change(project.get("contracts", []), change_id)),
            key=lambda item: item["id"],
        )
    )
    decisions = tuple(
        sorted(
            deepcopy(_by_change(project.get("decisions", []), change_id)),
            key=lambda item: item["id"],
        )
    )
    results = tuple(
        sorted(
            deepcopy(_by_change(project.get("results", []), change_id)),
            key=lambda item: item["id"],
        )
    )
    evidence = tuple(
        sorted(
            deepcopy(_by_change(project.get("evidence", []), change_id)),
            key=lambda item: item["id"],
        )
    )
    repository = deepcopy(project.get("repository", {}))

    # Freshness uses content digests instead of filesystem timestamps.
    artifact_digests: dict[str, str] = {
        change["id"]: canonical_digest(change),
    }
    for contract in contracts:
        contract_id = contract["id"]
        artifact_digests[contract_id] = canonical_digest(contract)
        # Keep both document and clause digests so a future selector can narrow
        # invalidation without changing the Project Store interface.
        for clause in contract.get("clauses", []):
            artifact_digests[f"{contract_id}#{clause['id']}"] = canonical_digest(clause)
    for collection in (decisions, results, evidence):
        for item in collection:
            artifact_digests[item["id"]] = canonical_digest(item)
    for artifact in repository.get("artifacts", []):
        artifact_digests[artifact["ref"]] = (
            artifact.get("digest") or canonical_digest(artifact)
        )

    snapshot_body = {
        "change": change,
        "contracts": contracts,
        "decisions": decisions,
        "results": results,
        "evidence": evidence,
        "repository": repository,
        "artifact_digests": artifact_digests,
    }
    return ProjectSnapshot(
        change_id=change_id,
        change=change,
        contracts=contracts,
        decisions=decisions,
        results=results,
        evidence=evidence,
        repository=repository,
        artifact_digests=artifact_digests,
        digest=canonical_digest(snapshot_body),
    )
