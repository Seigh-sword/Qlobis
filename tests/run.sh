#!/bin/sh
# Run every tests/cases/*.qbs against its *.expected on the python target.
#
# Usage: tests/run.sh [path-to-qlobis-binary]
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
QLOBIS="${1:-$ROOT/target/debug/qlobis}"
if [ ! -x "$QLOBIS" ]; then
    echo "building qlobis..." >&2
    (cd "$ROOT" && cargo build -p qlobis)
fi
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
PASS=0
FAIL=0
for case in "$ROOT"/tests/cases/*.qbs; do
    name="$(basename "$case" .qbs)"
    expected="$ROOT/tests/cases/$name.expected"
    proj="$WORK/$name"
    mkdir -p "$proj/src" "$proj/stdlib"
    cp "$ROOT"/stdlib/*.qbs "$proj/stdlib/"
    cp "$case" "$proj/src/main.qbs"
    cat > "$proj/qlobconfig.toml" <<EOF
[package]
name = "case_$name"
version = "0.0.0"

[targets]
python = true

[build]
sources = ["src"]
EOF
    if (cd "$proj" && "$QLOBIS" run > out.txt 2> err.txt); then
        if diff -u "$expected" "$proj/out.txt" > "$proj/diff.txt"; then
            echo "ok   $name"
            PASS=$((PASS + 1))
        else
            echo "FAIL $name (output mismatch)"
            cat "$proj/diff.txt"
            FAIL=$((FAIL + 1))
        fi
    else
        echo "FAIL $name (run failed)"
        cat "$proj/err.txt"
        FAIL=$((FAIL + 1))
    fi
done
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
