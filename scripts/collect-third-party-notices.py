#!/usr/bin/env python3
"""Collect third-party license notices for the distributed binary.

Reads Cargo.lock, locates each dependency in the local cargo registry, and
writes a single notices file containing every dependency's license expression
and license text. Binary distribution requires these notices: the BSD-3-Clause
and Apache-2.0 dependencies mandate that their terms travel with the binary.

Package sources are read from the extracted registry directory when present and
from the downloaded .crate archive otherwise, so fetching without building is
enough. All packages in Cargo.lock are included regardless of target platform,
which makes one notices file valid for every published binary. Windows and
WebAssembly packages only arrive when their target is fetched explicitly, so
run `cargo fetch --target <triple>` once per published target beforehand.

Usage:
  collect-third-party-notices.py --lock <Cargo.lock> --output <path> [--allow-missing]

Exits non-zero when a dependency has no discoverable license text, unless
--allow-missing is given.
"""

from __future__ import annotations

import argparse
import io
import os
import re
import sys
import tarfile
from pathlib import Path

PACKAGE_PATTERN = re.compile(
    r'\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"'
)
LICENSE_FIELD_PATTERN = re.compile(r'^license\s*=\s*"([^"]+)"', re.MULTILINE)
LICENSE_FILE_FIELD_PATTERN = re.compile(
    r'^license-file\s*=\s*"([^"]+)"', re.MULTILINE
)
LICENSE_TEXT_PATTERN = re.compile(
    r"^(LICEN[CS]E.*|COPYING.*|COPYRIGHT.*|NOTICE.*|UNLICENSE.*)$",
    re.IGNORECASE,
)
AUTHORS_PATTERN = re.compile(r"^authors\s*=\s*\[([^\]]*)\]", re.MULTILINE | re.DOTALL)

MIT_TEMPLATE = """MIT License

Copyright (c) {holder}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""


def registry_roots() -> list[Path]:
    roots: list[Path] = []
    for base in (os.environ.get("CARGO_HOME"), "~/.cargo"):
        if not base:
            continue
        expanded = Path(base).expanduser()
        roots.extend(sorted((expanded / "registry" / "src").glob("*")))
    return [root for root in roots if root.is_dir()]


def cache_roots() -> list[Path]:
    roots: list[Path] = []
    for base in (os.environ.get("CARGO_HOME"), "~/.cargo"):
        if not base:
            continue
        expanded = Path(base).expanduser()
        roots.extend(sorted((expanded / "registry" / "cache").glob("*")))
    return [root for root in roots if root.is_dir()]


def read_from_directory(package_dir: Path) -> tuple[str, list[tuple[str, str]]]:
    manifest = (package_dir / "Cargo.toml").read_text(
        encoding="utf-8", errors="replace"
    )
    texts: list[tuple[str, str]] = []
    for entry in sorted(package_dir.iterdir()):
        if entry.is_file() and LICENSE_TEXT_PATTERN.match(entry.name):
            texts.append(
                (entry.name, entry.read_text(encoding="utf-8", errors="replace"))
            )
    return manifest, texts


def read_from_archive(archive: Path) -> tuple[str, list[tuple[str, str]]]:
    manifest = ""
    texts: list[tuple[str, str]] = []
    with tarfile.open(archive, "r:gz") as tar:
        for member in tar.getmembers():
            if not member.isfile():
                continue
            parts = Path(member.name).parts
            if len(parts) != 2:
                continue
            filename = parts[1]
            handle = tar.extractfile(member)
            if handle is None:
                continue
            content = io.TextIOWrapper(
                handle, encoding="utf-8", errors="replace"
            ).read()
            if filename == "Cargo.toml":
                manifest = content
            elif LICENSE_TEXT_PATTERN.match(filename):
                texts.append((filename, content))
    return manifest, sorted(texts)


def locate_package(
    name: str, version: str
) -> tuple[str, list[tuple[str, str]], str] | None:
    for root in registry_roots():
        package_dir = root / f"{name}-{version}"
        if (package_dir / "Cargo.toml").is_file():
            manifest, texts = read_from_directory(package_dir)
            return manifest, texts, str(package_dir)
    for root in cache_roots():
        archive = root / f"{name}-{version}.crate"
        if archive.is_file():
            manifest, texts = read_from_archive(archive)
            return manifest, texts, str(archive)
    return None


def license_expression(manifest: str) -> str:
    match = LICENSE_FIELD_PATTERN.search(manifest)
    if match:
        return match.group(1)
    match = LICENSE_FILE_FIELD_PATTERN.search(manifest)
    if match:
        return f"see {match.group(1)}"
    return "not declared"


def package_authors(manifest: str) -> str:
    match = AUTHORS_PATTERN.search(manifest)
    if not match:
        return "the package authors"
    names = re.findall(r'"([^"]+)"', match.group(1))
    return ", ".join(names) if names else "the package authors"


def substitute_license_text(
    expression: str, manifest: str, canonical_dir: Path
) -> list[tuple[str, str]]:
    """Supply license text for packages that declare a license but ship none."""
    supplied: list[tuple[str, str]] = []
    if "Apache-2.0" in expression:
        apache = canonical_dir / "LICENSE-APACHE"
        if apache.is_file():
            supplied.append(
                (
                    "Apache-2.0 (canonical text; the package ships none)",
                    apache.read_text(encoding="utf-8").rstrip("\n"),
                )
            )
    if "MIT" in expression:
        supplied.append(
            (
                "MIT (canonical text; the package ships none)",
                MIT_TEMPLATE.format(holder=package_authors(manifest)),
            )
        )
    return supplied


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lock", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--canonical-dir",
        type=Path,
        default=Path(__file__).resolve().parents[3],
        help="directory holding LICENSE-APACHE, used when a package ships no text",
    )
    parser.add_argument("--allow-missing", action="store_true")
    arguments = parser.parse_args()

    lock_text = arguments.lock.read_text(encoding="utf-8")
    packages = PACKAGE_PATTERN.findall(lock_text)
    if not packages:
        print(f"no packages found in {arguments.lock}", file=sys.stderr)
        return 1

    root_package = arguments.lock.parent / "Cargo.toml"
    root_name = ""
    if root_package.is_file():
        match = re.search(
            r'^name\s*=\s*"([^"]+)"',
            root_package.read_text(encoding="utf-8"),
            re.MULTILINE,
        )
        if match:
            root_name = match.group(1)

    sections: list[str] = []
    unresolved: list[str] = []
    without_text: list[str] = []

    for name, version in sorted(set(packages)):
        if name == root_name:
            continue
        located = locate_package(name, version)
        if located is None:
            unresolved.append(f"{name} {version}")
            continue
        manifest, texts, _ = located
        expression = license_expression(manifest)
        if not texts:
            texts = substitute_license_text(
                expression, manifest, arguments.canonical_dir
            )
            if not texts:
                without_text.append(f"{name} {version} ({expression})")
        body = [f"## {name} {version}", "", f"License: {expression}", ""]
        if texts:
            for filename, content in texts:
                body.append(f"### {filename}")
                body.append("")
                body.append("```")
                body.append(content.rstrip("\n"))
                body.append("```")
                body.append("")
        else:
            body.append(
                "No license text ships with this package and no canonical text "
                "matches its license expression. Review this package by hand."
            )
            body.append("")
        sections.append("\n".join(body))

    header = [
        "# Third-party notices",
        "",
        "The distributed binary statically links the packages listed below.",
        "Their license terms are reproduced here in full.",
        "",
        f"Package count: {len(sections)}",
        "",
    ]
    arguments.output.write_text(
        "\n".join(header) + "\n".join(sections), encoding="utf-8"
    )
    print(f"wrote {arguments.output} covering {len(sections)} packages")

    failed = False
    if unresolved:
        print(
            "could not locate these packages in the local cargo registry; "
            "run `cargo fetch` first:",
            file=sys.stderr,
        )
        for entry in unresolved:
            print(f"  {entry}", file=sys.stderr)
        failed = True
    if without_text:
        print("these packages ship no license text:", file=sys.stderr)
        for entry in without_text:
            print(f"  {entry}", file=sys.stderr)
        failed = True
    if failed and not arguments.allow_missing:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
