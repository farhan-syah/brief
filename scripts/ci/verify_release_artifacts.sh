#!/usr/bin/env bash
set -euo pipefail

fail() {
  printf 'release artifacts: %s\n' "$1" >&2
  exit 1
}

if [[ $# -ne 2 ]]; then
  fail "usage: $0 <artifact-dir> <version>"
fi

artifact_dir=$1
version=$2

[[ -d "$artifact_dir" && ! -L "$artifact_dir" ]] || fail 'artifact directory is missing'
[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.(0|[1-9][0-9]*))?$ ]] || fail "invalid version '$version'"

expected=(
  "ogt-${version}-linux-x64.tar.gz"
  "ogt-${version}-linux-arm64.tar.gz"
  "ogt-${version}-macos-arm64.tar.gz"
  "ogt-${version}-macos-x64.tar.gz"
  "ogt-${version}-windows-x64.zip"
)
is_expected() {
  local candidate
  for candidate in "${expected[@]}"; do
    [[ $candidate == "$1" ]] && return 0
  done
  return 1
}

for path in "$artifact_dir"/* "$artifact_dir"/.[!.]* "$artifact_dir"/..?*; do
  [[ -e "$path" || -L "$path" ]] || continue
  name=${path##*/}
  is_expected "$name" || fail "unexpected entry '$name'"
  [[ -f "$path" && ! -L "$path" ]] || fail "artifact '$name' is not a regular file"
done

for name in "${expected[@]}"; do
  [[ -f "$artifact_dir/$name" && ! -L "$artifact_dir/$name" ]] || fail "missing artifact '$name'"
done

printf 'release artifacts: ok\n'
