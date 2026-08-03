"""人が保守するRequirement・Ruleを、Kernel用Indexへ変換する。"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import yaml

from .model import (
    ActivationRule,
    RequirementDefinition,
    RuleIndex,
    canonical_digest,
)
from .schema import default_schema_registry


ALLOWED_PHASES = {"before-build", "before-merge"}
ALLOWED_ROLES = {"Analyst", "Human", "Builder", "Challenger"}
ALLOWED_CONTEXT_SELECTORS = {
    "change",
    "repository-artifacts",
    "affected-code",
    "matching-contracts",
    "matching-decisions",
    "dependency-results",
    "matching-evidence",
    "contracts",
    "decisions",
    "results",
    "evidence",
}


def load_rule_source(path: str | Path) -> dict[str, Any]:
    with Path(path).open(encoding="utf-8") as stream:
        value = yaml.safe_load(stream)
    if not isinstance(value, dict):
        raise ValueError("rule source must be a mapping")
    return value


def compile_rule_index(source: dict[str, Any]) -> RuleIndex:
    """Rule sourceを構造検査し、評価順に依存しないIndexを作る。

    ここではID・参照・cycleなど機械判定可能な誤りだけを拒否する。
    Requirementの意味が妥当かどうかは自動決定しない。
    """

    requirements: dict[str, RequirementDefinition] = {}
    schema_registry = default_schema_registry()
    for raw in source.get("requirements", []):
        requirement_id = raw["id"]
        if requirement_id in requirements:
            raise ValueError(f"duplicate requirement: {requirement_id}")
        if raw["phase"] not in ALLOWED_PHASES:
            raise ValueError(f"invalid phase: {raw['phase']}")
        if raw["role"] not in ALLOWED_ROLES:
            raise ValueError(f"invalid role: {raw['role']}")
        if not schema_registry.supports_result_schema(raw["result_schema"]):
            raise ValueError(
                f"{requirement_id} refers to unsupported Result schema: "
                f"{raw['result_schema']}"
            )
        if not schema_registry.supports_result_role(
            raw["result_schema"],
            raw["role"],
        ):
            raise ValueError(
                f"{requirement_id} cannot use role {raw['role']} with "
                f"{raw['result_schema']}"
            )
        unknown_context = set(raw.get("context", [])) - ALLOWED_CONTEXT_SELECTORS
        if unknown_context:
            raise ValueError(
                "unknown context selector: " + ", ".join(sorted(unknown_context))
            )
        # Resultが古いRequirement定義を満たしたことにしないため、意味に関わる
        # 全フィールドからdefinition digestを作る。
        definition_body = {
            "id": requirement_id,
            "phase": raw["phase"],
            "role": raw["role"],
            "result_schema": raw["result_schema"],
            "depends_on": sorted(raw.get("depends_on", [])),
            "context": sorted(raw.get("context", [])),
        }
        requirements[requirement_id] = RequirementDefinition(
            id=requirement_id,
            phase=raw["phase"],
            role=raw["role"],
            result_schema=raw["result_schema"],
            depends_on=tuple(definition_body["depends_on"]),
            context=tuple(definition_body["context"]),
            definition_digest=canonical_digest(definition_body),
        )

    for requirement in requirements.values():
        for dependency in requirement.depends_on:
            if dependency not in requirements:
                raise ValueError(
                    f"{requirement.id} refers to unknown dependency: {dependency}"
                )
    _assert_acyclic(requirements)

    rules: list[ActivationRule] = []
    rule_ids: set[str] = set()
    for raw in source.get("rules", []):
        rule_id = raw["id"]
        if rule_id in rule_ids:
            raise ValueError(f"duplicate rule: {rule_id}")
        rule_ids.add(rule_id)
        requirement_id = raw["requirement"]
        if requirement_id not in requirements:
            raise ValueError(f"{rule_id} refers to unknown requirement: {requirement_id}")
        condition = raw.get("when", "signal")
        if condition not in {"always", "signal"}:
            raise ValueError(f"unsupported rule condition: {condition}")
        if condition == "signal" and not raw.get("signal"):
            raise ValueError(f"{rule_id} requires signal")
        rules.append(
            ActivationRule(
                id=rule_id,
                requirement_id=requirement_id,
                condition=condition,
                signal=raw.get("signal"),
                repository_phase=raw.get("repository_phase"),
                subjects=tuple(raw.get("subjects", [])),
            )
        )

    # YAMLの記載順を変えてもRule Index digestが変わらないよう正規化する。
    normalized = {
        "requirements": [
            {
                "id": value.id,
                "phase": value.phase,
                "role": value.role,
                "result_schema": value.result_schema,
                "depends_on": value.depends_on,
                "context": value.context,
                "definition_digest": value.definition_digest,
            }
            for value in sorted(requirements.values(), key=lambda item: item.id)
        ],
        "rules": [
            {
                "id": value.id,
                "requirement": value.requirement_id,
                "condition": value.condition,
                "signal": value.signal,
                "repository_phase": value.repository_phase,
                "subjects": value.subjects,
            }
            for value in sorted(rules, key=lambda item: item.id)
        ],
    }
    return RuleIndex(
        requirements=requirements,
        rules=tuple(rules),
        digest=canonical_digest(normalized),
    )


def _assert_acyclic(requirements: dict[str, RequirementDefinition]) -> None:
    """depends_onの循環をDFSで検出する。"""

    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(requirement_id: str) -> None:
        if requirement_id in visiting:
            raise ValueError(f"dependency cycle at: {requirement_id}")
        if requirement_id in visited:
            return
        visiting.add(requirement_id)
        for dependency in requirements[requirement_id].depends_on:
            visit(dependency)
        visiting.remove(requirement_id)
        visited.add(requirement_id)

    for requirement_id in requirements:
        visit(requirement_id)
