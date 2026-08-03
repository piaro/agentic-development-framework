#!/usr/bin/env python3
"""Small stateful GitHub CLI fake for Release publication boundary tests."""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path


STATE = Path(os.environ["FAKE_GH_STATE"])


def option(arguments: list[str], name: str) -> str:
    try:
        return arguments[arguments.index(name) + 1]
    except (ValueError, IndexError):
        raise SystemExit(f"missing fake gh option: {name}")


def release_root(tag: str) -> Path:
    return STATE / "releases" / tag


def api(arguments: list[str]) -> None:
    endpoint = arguments[0]
    if "/git/matching-refs/tags/" in endpoint:
        tag = endpoint.rsplit("/", 1)[-1]
        print("1" if release_root(tag).exists() else "0")
        return
    if "/actions/runs/" in endpoint:
        print(os.environ["FAKE_GH_RUN_JSON"])
        return
    raise SystemExit(f"unsupported fake gh api endpoint: {endpoint}")


def create(arguments: list[str]) -> None:
    tag = arguments[0]
    root = release_root(tag)
    if root.exists():
        raise SystemExit("fake release already exists")
    assets = root / "assets"
    assets.mkdir(parents=True)
    for value in arguments[1 : arguments.index("--repo")]:
        source = Path(value)
        target = assets / source.name
        shutil.copyfile(source, target)
        if os.environ.get("FAKE_GH_TAMPER_ASSET") == source.name:
            target.write_bytes(target.read_bytes() + b"tampered")
    (root / "state").write_text("draft\n", encoding="utf-8")
    (root / "target").write_text(option(arguments, "--target") + "\n", encoding="utf-8")


def download(arguments: list[str]) -> None:
    tag = arguments[0]
    source = release_root(tag) / "assets"
    destination = Path(option(arguments, "--dir"))
    destination.mkdir(parents=True, exist_ok=True)
    index = 0
    while index < len(arguments):
        if arguments[index] == "--pattern":
            name = arguments[index + 1]
            shutil.copyfile(source / name, destination / name)
            index += 2
        else:
            index += 1


def edit(arguments: list[str]) -> None:
    tag = arguments[0]
    if "--draft=false" not in arguments:
        raise SystemExit("fake release edit must publish the draft")
    (release_root(tag) / "state").write_text("published\n", encoding="utf-8")


def view_release(arguments: list[str]) -> None:
    tag = arguments[0]
    if option(arguments, "--jq") == ".isDraft":
        state = (release_root(tag) / "state").read_text(encoding="utf-8").strip()
        print("true" if state == "draft" else "false")
    else:
        print((release_root(tag) / "target").read_text(encoding="utf-8").strip())


def view_repository(arguments: list[str]) -> None:
    print(os.environ.get("FAKE_GH_DEFAULT_BRANCH", "main"))


def main(arguments: list[str]) -> None:
    STATE.mkdir(parents=True, exist_ok=True)
    if not arguments:
        raise SystemExit("missing fake gh command")
    if arguments[0] == "api":
        api(arguments[1:])
    elif arguments[:2] == ["attestation", "verify"]:
        artifact = Path(arguments[2]).name
        if os.environ.get("FAKE_GH_FAIL_ATTESTATION") == artifact:
            raise SystemExit("fake attestation verification failed")
        log = STATE / "attestation-calls.jsonl"
        with log.open("a", encoding="utf-8") as output:
            output.write(json.dumps(arguments[2:]) + "\n")
    elif arguments[:2] == ["release", "create"]:
        create(arguments[2:])
    elif arguments[:2] == ["release", "download"]:
        download(arguments[2:])
    elif arguments[:2] == ["release", "edit"]:
        edit(arguments[2:])
    elif arguments[:2] == ["release", "view"]:
        view_release(arguments[2:])
    elif arguments[:2] == ["repo", "view"]:
        view_repository(arguments[2:])
    else:
        raise SystemExit(f"unsupported fake gh command: {arguments}")


if __name__ == "__main__":
    main(sys.argv[1:])
