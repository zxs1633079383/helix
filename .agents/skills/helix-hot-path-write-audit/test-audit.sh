#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TMP_DIR="$(mktemp -d -t helix-hot-path-audit-test.XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

cat > "$TMP_DIR/clean.rs" <<'RS'
pub fn build_value(
    input: &str,
) -> String {
    input.to_string()
}
RS

cat > "$TMP_DIR/multiline.rs" <<'RS'
pub(crate) fn copy_twice(
    first: String,
    second: String,
) -> (String, String) {
    (first.clone(), second.clone())
}
RS

bash "$SCRIPT_DIR/audit.sh" "$TMP_DIR/clean.rs" >/dev/null

set +e
bash "$SCRIPT_DIR/audit.sh" "$TMP_DIR/multiline.rs" > "$TMP_DIR/output.txt"
rc=$?
set -e

if [ "$rc" -ne 1 ]; then
    echo "expected multiline fixture to exit 1, got $rc" >&2
    exit 1
fi

grep -q "multi-clone-per-fn" "$TMP_DIR/output.txt"
echo "helix-hot-path-write-audit fixtures PASS"
