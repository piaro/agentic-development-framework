#!/usr/bin/env python3
"""Validate the complete cross-platform binary candidate set."""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path


TARGETS = (
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
)
# The binaries statically link their dependencies, so the license terms have to
# be published with them rather than left in the repository.
LICENSE_FILES = (
    "LICENSE-APACHE",
    "LICENSE-MIT",
    "THIRD-PARTY-NOTICES.md",
)
REVISION = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")


def binary_name(target: str) -> str:
    suffix = ".exe" if target.endswith("-windows-msvc") else ""
    return f"agentic-{target}{suffix}"


def fail(message: str) -> "NoReturn":
    raise SystemExit(message)


def main(arguments: list[str]) -> None:
    if len(arguments) not in {3, 4} or (
        len(arguments) == 4 and arguments[3] != "--write-checksums"
    ):
        fail(
            "usage: verify-release-binaries.py "
            "<binary-dir> <source-revision> [--write-checksums]"
        )
    root = Path(arguments[1])
    revision = arguments[2]
    write_checksums = len(arguments) == 4
    if not REVISION.fullmatch(revision):
        fail("Source revision must be a 40-character lowercase hexadecimal Git SHA")
    if not root.is_dir():
        fail(f"Release binary candidate is not a directory: {root}")

    binaries = {binary_name(target) for target in TARGETS}
    records = {name + ".build.json" for name in binaries}
    expected = binaries | records | set(LICENSE_FILES)
    if not write_checksums or (root / "SHA256SUMS").exists():
        expected.add("SHA256SUMS")
    actual: set[str] = set()
    for entry in root.iterdir():
        if entry.is_symlink() or not entry.is_file():
            fail(f"Release binary candidate entry is not a regular file: {entry.name}")
        actual.add(entry.name)
    if actual != expected:
        fail(
            "Release binary candidate file set mismatch: "
            f"missing={sorted(expected - actual)}, "
            f"unexpected={sorted(actual - expected)}"
        )

    checksums: list[str] = []
    for target in TARGETS:
        name = binary_name(target)
        path = root / name
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        checksums.append(f"{digest}  {name}")
        try:
            record = json.loads(
                (root / f"{name}.build.json").read_text(encoding="utf-8")
            )
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            fail(f"Invalid binary build record for {name}: {error}")
        fields = {
            "schema_version",
            "binary_name",
            "target",
            "source_revision",
            "sha256",
            "size",
            "rustc_version",
        }
        if not isinstance(record, dict) or set(record) != fields:
            fail(f"Binary build record fields do not match Schema version 1: {name}")
        if record["schema_version"] != "1":
            fail(f"Unsupported binary build record Schema: {name}")
        if record["binary_name"] != name or record["target"] != target:
            fail(f"Binary build record target mismatch: {name}")
        if record["source_revision"] != revision:
            fail(f"Binary build record source revision mismatch: {name}")
        if record["sha256"] != f"sha256:{digest}" or not DIGEST.fullmatch(
            str(record["sha256"])
        ):
            fail(f"Binary build record digest mismatch: {name}")
        if record["size"] != path.stat().st_size or not isinstance(
            record["size"], int
        ):
            fail(f"Binary build record size mismatch: {name}")
        if (
            not isinstance(record["rustc_version"], str)
            or not record["rustc_version"].startswith("rustc 1.89.0 ")
        ):
            fail(f"Binary build record Rust version mismatch: {name}")

    for name in LICENSE_FILES:
        path = root / name
        if path.stat().st_size == 0:
            fail(f"Published license file is empty: {name}")
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        checksums.append(f"{digest}  {name}")

    checksum_text = "\n".join(checksums) + "\n"
    checksum_path = root / "SHA256SUMS"
    if write_checksums:
        checksum_path.write_text(checksum_text, encoding="utf-8")
    elif checksum_path.read_text(encoding="utf-8") != checksum_text:
        fail("SHA256SUMS does not match the Release binaries")

    print(
        json.dumps(
            {
                "schema_version": "1",
                "source_revision": revision,
                "binary_count": len(TARGETS),
                "binaries": [
                    {
                        "name": binary_name(target),
                        "target": target,
                        "sha256": checksums[index].split(" ", 1)[0],
                    }
                    for index, target in enumerate(TARGETS)
                ],
            },
            sort_keys=True,
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    main(sys.argv)
