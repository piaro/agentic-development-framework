#!/usr/bin/env python3
"""Validate the portable metadata in a Release candidate Artifact.

Cryptographic Release verification remains in the Rust CLI. This helper only
checks the CI transfer envelope: exact file names, regular files, receipt
shape, the raw archive digest, the signer public-key pin, and tag naming.
It uses only the Python standard library and never reads the signing secret.
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path


EXPECTED_FILES = {
    "candidate-framework.lock",
    "distribution-trust.json",
    "framework-release.tar",
    "publish-receipt.json",
}
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
HEX_KEY = re.compile(r"^[0-9a-f]{64}$")
RELEASE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


def fail(message: str) -> "NoReturn":
    raise SystemExit(message)


def regular_files(root: Path) -> set[str]:
    if not root.is_dir():
        fail(f"Release candidate is not a directory: {root}")
    names: set[str] = set()
    for entry in root.iterdir():
        if entry.is_symlink() or not entry.is_file():
            fail(f"Release candidate entry is not a regular file: {entry.name}")
        names.add(entry.name)
    return names


def required_string(value: object, label: str, pattern: re.Pattern[str]) -> str:
    if not isinstance(value, str) or not pattern.fullmatch(value):
        fail(f"{label} has an invalid value")
    return value


def main(arguments: list[str]) -> None:
    if len(arguments) not in {5, 6}:
        fail(
            "usage: verify-release-candidate.py "
            "<candidate-dir> <expected-public-key> <expected-source-id> "
            "<expected-key-id> [expected-tag]"
        )
    root = Path(arguments[1])
    expected_public_key = required_string(
        arguments[2], "expected public key", HEX_KEY
    )
    expected_source_id = arguments[3]
    expected_key_id = arguments[4]
    expected_tag = arguments[5] if len(arguments) == 6 else None
    if not expected_source_id or not expected_key_id:
        fail("expected source and key IDs must not be empty")

    names = regular_files(root)
    if names != EXPECTED_FILES:
        missing = sorted(EXPECTED_FILES - names)
        unexpected = sorted(names - EXPECTED_FILES)
        fail(
            "Release candidate file set mismatch: "
            f"missing={missing}, unexpected={unexpected}"
        )

    receipt_path = root / "publish-receipt.json"
    try:
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"Invalid Publish Receipt: {error}")
    if not isinstance(receipt, dict):
        fail("Publish Receipt must be an object")
    expected_fields = {
        "schema_version",
        "release_id",
        "artifact_digest",
        "archive_digest",
        "signer_public_key",
        "outputs",
    }
    if set(receipt) != expected_fields:
        fail("Publish Receipt fields do not match Schema version 1")
    if receipt.get("schema_version") != "1":
        fail("Unsupported Publish Receipt Schema")

    release_id = required_string(receipt.get("release_id"), "release_id", RELEASE_ID)
    artifact_digest = required_string(
        receipt.get("artifact_digest"), "artifact_digest", DIGEST
    )
    archive_digest = required_string(
        receipt.get("archive_digest"), "archive_digest", DIGEST
    )
    signer_public_key = required_string(
        receipt.get("signer_public_key"), "signer_public_key", HEX_KEY
    )
    if signer_public_key != expected_public_key:
        fail("Publish Receipt signer public key does not match the trusted pin")

    trust_path = root / "distribution-trust.json"
    try:
        trust = json.loads(trust_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"Invalid Distribution Trust: {error}")
    if (
        not isinstance(trust, dict)
        or set(trust) != {"schema_version", "release_id", "keys"}
        or trust.get("schema_version") != "1"
        or trust.get("release_id") != release_id
    ):
        fail("Distribution Trust identity is invalid")
    keys = trust.get("keys")
    if not isinstance(keys, list) or not keys:
        fail("Distribution Trust keys are invalid")
    expected_key = None
    seen_key_ids: set[str] = set()
    for key in keys:
        if not isinstance(key, dict) or set(key) != {
            "id",
            "algorithm",
            "public_key",
            "allowed_sources",
            "status",
        }:
            fail("Distribution Trust key fields are invalid")
        key_id = key.get("id")
        public_key = key.get("public_key")
        allowed_sources = key.get("allowed_sources")
        if (
            not isinstance(key_id, str)
            or not key_id
            or key_id in seen_key_ids
            or key.get("algorithm") != "ed25519"
            or not isinstance(public_key, str)
            or not HEX_KEY.fullmatch(public_key)
            or not isinstance(allowed_sources, list)
            or not allowed_sources
            or not all(isinstance(source, str) and source for source in allowed_sources)
            or len(set(allowed_sources)) != len(allowed_sources)
            or key.get("status") not in {"active", "retired", "revoked"}
        ):
            fail("Distribution Trust key policy is invalid")
        seen_key_ids.add(key_id)
        if key.get("id") == expected_key_id:
            expected_key = key
    if expected_key is None:
        fail("Distribution Trust does not contain the expected key")
    if (
        expected_key.get("algorithm") != "ed25519"
        or expected_key.get("public_key") != expected_public_key
        or expected_key.get("status") != "active"
        or expected_source_id not in expected_key.get("allowed_sources", [])
    ):
        fail("Distribution Trust key policy does not match the trusted pin")

    outputs = receipt.get("outputs")
    if not isinstance(outputs, dict) or set(outputs) != {
        "archive",
        "framework_lock",
    }:
        fail("Publish Receipt outputs do not match Schema version 1")
    output_names = {
        "archive": "framework-release.tar",
        "framework_lock": "candidate-framework.lock",
    }
    for field, expected_name in output_names.items():
        value = outputs.get(field)
        if not isinstance(value, str) or Path(value).name != expected_name:
            fail(f"Publish Receipt outputs.{field} does not name {expected_name}")

    archive_hash = hashlib.sha256(
        (root / "framework-release.tar").read_bytes()
    ).hexdigest()
    if archive_digest != f"sha256:{archive_hash}":
        fail("Framework Release archive digest does not match Publish Receipt")

    canonical_tag = f"framework-{release_id}"
    if expected_tag is not None and expected_tag != canonical_tag:
        fail(
            f"Release tag must be {canonical_tag!r} for Release {release_id!r}"
        )

    # Stable JSON allows the publication script to generate provenance without
    # reparsing YAML or trusting filesystem paths recorded on another runner.
    print(
        json.dumps(
            {
                "schema_version": "1",
                "release_id": release_id,
                "release_tag": canonical_tag,
                "artifact_digest": artifact_digest,
                "archive_digest": archive_digest,
                "signer_public_key": signer_public_key,
            },
            sort_keys=True,
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    main(sys.argv)
