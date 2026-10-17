#!/usr/bin/env bash
#
# End-to-end test runner for bas.
#
# Builds the compiler, then compiles and runs every program in examples/ and
# compares its stdout against tests/expected/<name>.out.
#
# Usage: ./tests/run.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPILER="$ROOT/compiler"
BAS="$COMPILER/target/debug/bas"
EXPECTED="$ROOT/tests/expected"

echo "==> building compiler"
cargo build --manifest-path "$COMPILER/Cargo.toml" -p bas-cli --quiet

fail=0
count=0
for src in "$ROOT"/examples/*.ba; do
    name="$(basename "$src" .ba)"
    exp="$EXPECTED/$name.out"
    if [[ ! -f "$exp" ]]; then
        echo "SKIP $name (no expected output at tests/expected/$name.out)"
        continue
    fi
    actual="$("$BAS" run "$src" 2>/tmp/bas_stderr_$$ || true)"
    expected="$(cat "$exp")"
    count=$((count + 1))
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS $name"
    else
        echo "FAIL $name"
        diff <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") || true
        fail=$((fail + 1))
    fi
done
rm -f /tmp/bas_stderr_$$

# Negative tests: programs that must be rejected with a matching diagnostic.
ERRDIR="$ROOT/tests/errors"
if compgen -G "$ERRDIR/*.ba" > /dev/null; then
    for src in "$ERRDIR"/*.ba; do
        name="$(basename "$src" .ba)"
        exp="$ERRDIR/$name.err"
        count=$((count + 1))
        stderr="$("$BAS" check "$src" 2>&1 >/dev/null || true)"
        if "$BAS" check "$src" >/dev/null 2>&1; then
            echo "FAIL $name (expected an error, but check passed)"
            fail=$((fail + 1))
            continue
        fi
        if [[ -f "$exp" ]] && ! grep -qF "$(cat "$exp")" <<< "$stderr"; then
            echo "FAIL $name (diagnostic did not match)"
            echo "  expected to contain: $(cat "$exp")"
            echo "  got: $stderr"
            fail=$((fail + 1))
            continue
        fi
        echo "PASS $name (rejected)"
    done
fi

if [[ $fail -ne 0 ]]; then
    echo "==> $fail of $count example(s) failed"
    exit 1
fi
echo "==> all $count example(s) passed"
