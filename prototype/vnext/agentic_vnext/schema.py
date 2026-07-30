"""Language-neutral JSON Schema bundle and a dependency-free validator subset."""

from __future__ import annotations

from functools import lru_cache
import json
from pathlib import Path
import re
from typing import Any

from .model import canonical_digest


SCHEMA_BUNDLE_VERSION = "1"
_RECORD_KINDS = {"change", "contract", "decision", "result", "evidence"}
_BUILT_IN_RESULT_SCHEMAS = {
    "result.analysis",
    "result.build",
    "result.challenge",
    "result.evidence",
    "result.human-answer",
    "result.risk-signal-review",
}


class SchemaValidationError(ValueError):
    """A stable path and reason suitable for CLI and CI diagnostics."""


class SchemaRegistry:
    def __init__(
        self,
        record_schemas: dict[str, dict[str, Any]],
        result_payload_schemas: dict[str, dict[str, Any]],
    ):
        missing = _RECORD_KINDS - set(record_schemas)
        unexpected = set(record_schemas) - _RECORD_KINDS
        if missing or unexpected:
            raise ValueError(
                "invalid Schema bundle: "
                f"missing={sorted(missing)}, unexpected={sorted(unexpected)}"
            )
        missing_results = _BUILT_IN_RESULT_SCHEMAS - set(result_payload_schemas)
        unexpected_results = set(result_payload_schemas) - _BUILT_IN_RESULT_SCHEMAS
        if missing_results or unexpected_results:
            raise ValueError(
                "invalid Result payload Schema bundle: "
                f"missing={sorted(missing_results)}, "
                f"unexpected={sorted(unexpected_results)}"
            )
        self.record_schemas = record_schemas
        self.result_payload_schemas = result_payload_schemas
        self.digest = canonical_digest(
            {
                "bundle_version": SCHEMA_BUNDLE_VERSION,
                "record_schemas": record_schemas,
                "result_payload_schemas": result_payload_schemas,
            }
        )

    def validate(self, record_kind: str, value: Any) -> None:
        if record_kind not in self.record_schemas:
            raise ValueError(f"unknown record kind: {record_kind}")
        _validate(value, self.record_schemas[record_kind], "$")
        if record_kind == "result":
            # The envelope only establishes that these are strings/objects.
            # Dispatch binds that pair to one concrete Result contract.
            if not self.supports_result_schema(value["result_schema"]):
                _fail(
                    "$.result_schema",
                    f"unsupported Result schema {value['result_schema']!r}",
                )
            if not self.supports_result_role(
                value["result_schema"],
                value["role"],
            ):
                _fail(
                    "$.role",
                    f"role {value['role']!r} is not allowed for "
                    f"{value['result_schema']!r}",
                )
            self.validate_result_payload(
                value["result_schema"],
                value["payload"],
                path="$.payload",
            )

    def supports_result_schema(self, result_schema: str) -> bool:
        return result_schema in self.result_payload_schemas

    def supports_result_role(self, result_schema: str, role: str) -> bool:
        schema = self.result_payload_schemas.get(result_schema)
        if schema is None:
            return False
        return role in schema["x-allowed-roles"]

    def validate_result_payload(
        self,
        result_schema: str,
        payload: Any,
        path: str = "$",
    ) -> None:
        schema = self.result_payload_schemas.get(result_schema)
        if schema is None:
            _fail(
                "$.result_schema",
                f"unsupported Result schema {result_schema!r}",
            )
        _validate(payload, schema, path)


@lru_cache(maxsize=1)
def default_schema_registry() -> SchemaRegistry:
    schema_root = Path(__file__).resolve().parents[1] / "schemas" / "v1"
    record_schemas = {}
    for record_kind in sorted(_RECORD_KINDS):
        path = schema_root / f"{record_kind}.schema.json"
        with path.open(encoding="utf-8") as stream:
            schema = json.load(stream)
        if not isinstance(schema, dict):
            raise ValueError(f"Schema must be an object: {path}")
        record_schemas[record_kind] = schema

    result_payload_schemas = {}
    for path in sorted((schema_root / "result-payloads").glob("*.schema.json")):
        with path.open(encoding="utf-8") as stream:
            schema = json.load(stream)
        if not isinstance(schema, dict):
            raise ValueError(f"Schema must be an object: {path}")
        result_schema = schema.get("x-result-schema")
        if not isinstance(result_schema, str):
            raise ValueError(f"Result Schema identity is missing: {path}")
        allowed_roles = schema.get("x-allowed-roles")
        if (
            not isinstance(allowed_roles, list)
            or not allowed_roles
            or any(not isinstance(role, str) for role in allowed_roles)
        ):
            raise ValueError(f"Result Schema allowed roles are invalid: {path}")
        if result_schema in result_payload_schemas:
            raise ValueError(f"duplicate Result Schema: {result_schema}")
        result_payload_schemas[result_schema] = schema
    return SchemaRegistry(record_schemas, result_payload_schemas)


def validate_record(record_kind: str, value: Any) -> None:
    default_schema_registry().validate(record_kind, value)


def validate_result_payload(result_schema: str, payload: Any) -> None:
    default_schema_registry().validate_result_payload(result_schema, payload)


def validate_json_document(value: Any, schema: dict[str, Any]) -> None:
    """Validate a non-Record output with the shared JSON Schema subset."""

    _validate(value, schema, "$")


def _validate(value: Any, schema: dict[str, Any], path: str) -> None:
    alternatives = schema.get("anyOf")
    if isinstance(alternatives, list):
        for alternative in alternatives:
            try:
                _validate(value, alternative, path)
            except SchemaValidationError:
                continue
            break
        else:
            _fail(path, "must match at least one anyOf alternative")

    if "const" in schema and value != schema["const"]:
        _fail(path, f"must equal {schema['const']!r}")
    if "enum" in schema and value not in schema["enum"]:
        _fail(path, f"must be one of {schema['enum']!r}")

    expected_type = schema.get("type")
    if expected_type is not None and not _matches_type(value, expected_type):
        _fail(path, f"must be {expected_type}, got {type(value).__name__}")

    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0):
            _fail(path, f"must contain at least {schema['minLength']} characters")
        pattern = schema.get("pattern")
        if pattern is not None and re.search(pattern, value) is None:
            _fail(path, f"must match pattern {pattern!r}")

    if isinstance(value, list):
        if len(value) < schema.get("minItems", 0):
            _fail(path, f"must contain at least {schema['minItems']} items")
        if schema.get("uniqueItems"):
            normalized = [
                json.dumps(item, ensure_ascii=False, sort_keys=True)
                for item in value
            ]
            if len(normalized) != len(set(normalized)):
                _fail(path, "must contain unique items")
        item_schema = schema.get("items")
        if isinstance(item_schema, dict):
            for index, item in enumerate(value):
                _validate(item, item_schema, f"{path}[{index}]")

    if isinstance(value, dict):
        required = schema.get("required", [])
        for field in required:
            if field not in value:
                _fail(path, f"missing required field {field!r}")
        properties = schema.get("properties", {})
        for field, item in value.items():
            child_path = f"{path}.{field}"
            if field in properties:
                _validate(item, properties[field], child_path)
                continue
            additional = schema.get("additionalProperties", True)
            if additional is False:
                _fail(child_path, "unexpected field")
            if isinstance(additional, dict):
                _validate(item, additional, child_path)


def _matches_type(value: Any, expected: str) -> bool:
    return {
        "object": isinstance(value, dict),
        "array": isinstance(value, list),
        "string": isinstance(value, str),
        "integer": isinstance(value, int) and not isinstance(value, bool),
        "number": (
            isinstance(value, (int, float))
            and not isinstance(value, bool)
        ),
        "boolean": isinstance(value, bool),
        "null": value is None,
    }.get(expected, False)


def _fail(path: str, reason: str) -> None:
    raise SchemaValidationError(f"Schema validation failed at {path}: {reason}")
