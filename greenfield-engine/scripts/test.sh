#!/usr/bin/env bash
# Run the greenfield engine test suite from a fixed entrypoint, so it can be permission-allow-listed
# (`bash greenfield-engine/scripts/test.sh`) without triggering a bash-expansion prompt every run.
# Optional args pass straight through to `cargo test` (e.g. a test-name filter):
#   bash scripts/test.sh                       # full suite
#   bash scripts/test.sh furrow                # only matching tests
# Prints just the result/error lines; the full output is saved to /tmp/gf-test.log.
set -uo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo test -p engine "$@" 2>&1 | tee /tmp/gf-test.log \
  | grep -E "test result:|error\[|^error:|FAILED|panicked|warning: unused" | tail -40
status="${PIPESTATUS[0]}"
echo "--- cargo test exit ${status} · full log: /tmp/gf-test.log ---"
exit "${status}"
