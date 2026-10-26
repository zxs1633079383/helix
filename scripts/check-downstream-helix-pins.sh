#!/usr/bin/env bash
set -euo pipefail

HELIX_REPO="${HELIX_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
LOOPFORGE_REPO="${LOOPFORGE_REPO:-/System/Volumes/Data/workspace/rust/loopforge-tauri-im}"
MOBILE_REPO="${MOBILE_REPO:-/System/Volumes/Data/workspace/c/mobile-qucik-c++}"
EXPECTED_REV="${HELIX_EXPECTED_REV:-$(git -C "$HELIX_REPO" rev-parse HEAD)}"

failures=0

note() {
  printf '%s\n' "$*"
}

fail() {
  failures=$((failures + 1))
  printf 'FAIL: %s\n' "$*" >&2
}

check_file_exists() {
  local file="$1"
  if [ ! -f "$file" ]; then
    fail "missing file: $file"
    return 1
  fi
  return 0
}

check_loopforge_cargo_file() {
  local rel="$1"
  local file="$LOOPFORGE_REPO/$rel"
  check_file_exists "$file" || return 0

  local revs
  revs="$(awk '
    /git *= *"file:\/\/\/System\/Volumes\/Data\/workspace\/rust\/helix"/ {
      if (match($0, /rev *= *"[0-9a-fA-F]+"/)) {
        rev = substr($0, RSTART, RLENGTH)
        gsub(/rev *= *"/, "", rev)
        gsub(/"/, "", rev)
        print rev
      }
    }
  ' "$file" | sort -u)"

  if [ -z "$revs" ]; then
    fail "$rel has no pinned local helix git rev"
    return 0
  fi

  while IFS= read -r rev; do
    [ -n "$rev" ] || continue
    if [ "$rev" != "$EXPECTED_REV" ]; then
      fail "$rel pins helix $rev, expected $EXPECTED_REV"
    else
      note "OK: $rel pins $rev"
    fi
  done <<< "$revs"
}

check_mobile_assignment() {
  local rel="$1"
  local key="$2"
  local file="$MOBILE_REPO/$rel"
  check_file_exists "$file" || return 0

  local revs
  revs="$(sed -n -E "s/^[[:space:]]*${key}=\\\"?([0-9a-fA-F]{7,40})\\\"?.*$/\\1/p" "$file" | sort -u)"
  if [ -z "$revs" ]; then
    fail "$rel has no $key pin"
    return 0
  fi

  while IFS= read -r rev; do
    [ -n "$rev" ] || continue
    if [ "$rev" != "$EXPECTED_REV" ]; then
      fail "$rel $key pins helix $rev, expected $EXPECTED_REV"
    else
      note "OK: $rel $key pins $rev"
    fi
  done <<< "$revs"
}

note "checking downstream helix pins"
note "helix repo: $HELIX_REPO"
note "expected rev: $EXPECTED_REV"

check_loopforge_cargo_file "src-tauri/Cargo.toml"
check_loopforge_cargo_file "crates/helix-driver-instrument/Cargo.toml"
check_mobile_assignment "scripts/build-helix-ffi.sh" "DEFAULT_HELIX_REPO_REV"
check_mobile_assignment "config/helix.env.example" "HELIX_REPO_REV"

if [ "$failures" -ne 0 ]; then
  fail "$failures downstream helix pin check(s) failed"
  exit 1
fi

note "all downstream helix pins match $EXPECTED_REV"
