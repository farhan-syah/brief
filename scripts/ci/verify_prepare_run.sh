#!/usr/bin/env bash
set -euo pipefail

fail() {
  printf 'prepare run: %s\n' "$1" >&2
  exit 1
}

if [[ $# -ne 2 ]]; then
  fail "usage: $0 <run.json> <commit>"
fi

run_json=$1
commit=$2

[[ -f "$run_json" && ! -L "$run_json" ]] || fail 'missing workflow-run metadata'
[[ $commit =~ ^[0-9a-f]{40}$ ]] || fail 'invalid release commit'
command -v jq >/dev/null 2>&1 || fail 'jq is required'

if ! jq -e 'type == "object"' "$run_json" >/dev/null 2>&1; then
  fail 'workflow-run metadata is not valid JSON'
fi

check_field() {
  local field=$1
  local expected=$2
  local actual

  if ! actual=$(jq -er --arg field "$field" \
    'if has($field) and .[$field] != null and (.[$field] | type) == "string" then .[$field] else empty end' \
    "$run_json" 2>/dev/null); then
    fail "run $field is '<missing>', expected '$expected'"
  fi
  if [[ $actual != "$expected" ]]; then
    fail "run $field is '$actual', expected '$expected'"
  fi
}

check_field head_sha "$commit"
check_field path '.github/workflows/release-prepare.yml'
check_field event push
check_field status completed
check_field conclusion success

printf 'prepare run: ok\n'
