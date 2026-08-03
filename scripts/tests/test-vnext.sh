#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
KIT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)

sh "$KIT_ROOT/scripts/tests/test-vnext-rust.sh"
