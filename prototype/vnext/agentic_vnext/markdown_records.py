"""Codec for human-readable records with one machine-readable YAML block."""

from __future__ import annotations

import re
from typing import Any

import yaml


_LABELS = {
    "change": "agentic-change",
    "contract": "agentic-contract",
    "decision": "agentic-decision",
}


def parse_markdown_record(text: str, record_kind: str) -> dict[str, Any]:
    """Parse exactly one typed fenced block and ignore narrative prose."""

    match = _single_block(text, record_kind)
    value = yaml.safe_load(match.group("payload"))
    if not isinstance(value, dict):
        raise ValueError(
            f"{record_kind} structured block must contain a mapping"
        )
    if not isinstance(value.get("id"), str):
        raise ValueError(f"{record_kind} structured block requires string id")
    return value


def create_markdown_record(
    value: dict[str, Any],
    record_kind: str,
) -> str:
    """Create a readable shell without inventing domain-specific prose."""

    label = _label(record_kind)
    title = value.get("title") or value["id"]
    payload = _dump_yaml(value)
    return (
        f"# {title}\n\n"
        "This document is owned by the project. Human-readable rationale, "
        "examples, and diagrams may be added outside the structured block.\n\n"
        f"```{label}\n"
        f"{payload}"
        "```\n"
    )


def replace_markdown_record(
    text: str,
    value: dict[str, Any],
    record_kind: str,
) -> str:
    """Replace only the structured payload and preserve all surrounding prose."""

    match = _single_block(text, record_kind)
    return (
        text[: match.start("payload")]
        + _dump_yaml(value)
        + text[match.end("payload") :]
    )


def _single_block(text: str, record_kind: str) -> re.Match[str]:
    label = _label(record_kind)
    pattern = re.compile(
        rf"^```{re.escape(label)}[ \t]*\n"
        rf"(?P<payload>.*?)"
        rf"^```[ \t]*$",
        re.MULTILINE | re.DOTALL,
    )
    matches = list(pattern.finditer(text))
    if len(matches) != 1:
        raise ValueError(
            f"{record_kind} Markdown must contain exactly one {label} block"
        )
    return matches[0]


def _label(record_kind: str) -> str:
    try:
        return _LABELS[record_kind]
    except KeyError as error:
        raise ValueError(f"unsupported Markdown record kind: {record_kind}") from error


def _dump_yaml(value: dict[str, Any]) -> str:
    return yaml.safe_dump(
        value,
        allow_unicode=True,
        sort_keys=False,
    )
