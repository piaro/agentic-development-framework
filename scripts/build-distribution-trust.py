#!/usr/bin/env python3
"""Create the public trust policy that is attested independently of a Release."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


SAFE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
KEY = re.compile(r"^[0-9a-f]{64}$")


def main(arguments: list[str]) -> None:
    if len(arguments) != 6:
        raise SystemExit(
            "usage: build-distribution-trust.py "
            "<release-id> <key-id> <public-key> <source-id> <output>"
        )
    release_id, key_id, public_key, source_id, output = arguments[1:]
    if not SAFE_ID.fullmatch(release_id):
        raise SystemExit("distribution trust release ID is invalid")
    if not key_id or not source_id:
        raise SystemExit("distribution trust key and source IDs must not be empty")
    if not KEY.fullmatch(public_key):
        raise SystemExit("distribution trust public key must be lowercase hexadecimal")
    path = Path(output)
    if path.exists():
        raise SystemExit(f"distribution trust output already exists: {path}")
    value = {
        "schema_version": "1",
        "release_id": release_id,
        "keys": [
            {
                "id": key_id,
                "algorithm": "ed25519",
                "public_key": public_key,
                "allowed_sources": [source_id],
                "status": "active",
            }
        ],
    }
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main(sys.argv)
