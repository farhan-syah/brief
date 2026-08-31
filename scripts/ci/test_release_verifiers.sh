#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 0 ]]; then
  printf 'usage: %s\n' "$0" >&2
  exit 1
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
stamp="$root/scripts/ci/stamp_version.sh"
changelog="$root/scripts/ci/changelog_section.sh"
prepare="$root/scripts/ci/verify_prepare_run.sh"
artifacts="$root/scripts/ci/verify_release_artifacts.sh"
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/brief-release-verifiers.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT

failures=0

ok_case() {
  printf 'ok   %s\n' "$1"
}

failed_case() {
  printf 'not ok %s\n' "$1" >&2
  failures=$((failures + 1))
}

expect_success() {
  local name=$1
  shift
  if "$@" >/dev/null 2>&1; then
    ok_case "$name"
  else
    failed_case "$name"
  fi
}

expect_failure() {
  local name=$1
  shift
  if "$@" >/dev/null 2>&1; then
    failed_case "$name"
  else
    ok_case "$name"
  fi
}

make_cargo_fixture() {
  local dir=$1
  mkdir -p "$dir"
  cat >"$dir/Cargo.toml" <<'EOF'
[package]
name = "brief"
version = "0.1.0"
edition = "2021"

[dependencies]
dep = "1"
EOF
  cat >"$dir/Cargo.lock" <<'EOF'
version = 3

[[package]]
name = "brief"
version = "0.1.0"
dependencies = [
 "dep",
]

[[package]]
name = "dep"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0000000000000000000000000000000000000000000000000000000000000000"
EOF
}

run_stamp() {
  local dir=$1
  local version=$2
  (cd "$dir" && "$stamp" "$version")
}

stable="$tmp_dir/stable"
make_cargo_fixture "$stable"
cp "$stable/Cargo.toml" "$stable/Cargo.toml.orig"
cp "$stable/Cargo.lock" "$stable/Cargo.lock.orig"
expect_success 'stamp stable no-op' run_stamp "$stable" 0.1.0
expect_success 'stamp stable source unchanged' cmp "$stable/Cargo.toml" "$stable/Cargo.toml.orig"
expect_success 'stamp stable lock unchanged' cmp "$stable/Cargo.lock" "$stable/Cargo.lock.orig"

prerelease="$tmp_dir/prerelease"
make_cargo_fixture "$prerelease"
cp "$prerelease/Cargo.toml" "$prerelease/Cargo.toml.orig"
cp "$prerelease/Cargo.lock" "$prerelease/Cargo.lock.orig"
expect_success 'stamp prerelease' run_stamp "$prerelease" 0.1.0-beta.1
expect_success 'stamp Cargo.toml prerelease version' grep -q '^version = "0.1.0-beta.1"$' "$prerelease/Cargo.toml"
expect_success 'stamp Cargo.lock prerelease version' grep -q '^version = "0.1.0-beta.1"$' "$prerelease/Cargo.lock"
expect_success 'stamp prerelease source unchanged' cmp "$stable/Cargo.toml.orig" "$prerelease/Cargo.toml.orig"
expect_success 'stamp prerelease lock dependencies unchanged' grep -q '^checksum = "0000000000000000000000000000000000000000000000000000000000000000"$' "$prerelease/Cargo.lock"
expect_success 'stamp prerelease idempotent' run_stamp "$prerelease" 0.1.0-beta.1

for invalid in 0.1 01.1.0 0.1.0-dev.1 '0.1.0-beta.1-extra'; do
  expect_failure "stamp rejects version $invalid" run_stamp "$stable" "$invalid"
done

renamed="$tmp_dir/renamed"
make_cargo_fixture "$renamed"
python3 - "$renamed/Cargo.toml" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
path.write_text(path.read_text(encoding="utf-8").replace('name = "brief"', 'name = "other"', 1), encoding="utf-8")
PY
expect_failure 'stamp rejects renamed manifest' run_stamp "$renamed" 0.1.0-beta.1

source_bearing="$tmp_dir/source-bearing"
make_cargo_fixture "$source_bearing"
python3 - "$source_bearing/Cargo.lock" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
text = text.replace('name = "brief"\nversion = "0.1.0"', 'name = "brief"\nversion = "0.1.0"\nsource = "registry+https://example.invalid"', 1)
path.write_text(text, encoding="utf-8")
PY
expect_failure 'stamp rejects source-bearing brief lock block' run_stamp "$source_bearing" 0.1.0-beta.1

make_changelog_fixture() {
  local dir=$1
  mkdir -p "$dir"
  cat >"$dir/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

## [0.1.0] - 2026-08-31

### Added

- A release note.

---

[Unreleased]: https://example.invalid/unreleased
[0.1.0]: https://example.invalid/0.1.0
EOF
}

run_changelog() {
  local dir=$1
  local version=$2
  local output=$3
  (cd "$dir" && "$changelog" "$version" "$output")
}

changelog_fixture="$tmp_dir/changelog"
make_changelog_fixture "$changelog_fixture"
expect_success 'changelog extracts dated section' run_changelog "$changelog_fixture" 0.1.0 "$changelog_fixture/notes.md"
assert_footer_links_excluded() {
  ! grep -q '^\[Unreleased\]:' "$changelog_fixture/notes.md" &&
    ! grep -q '^\[0.1.0\]:' "$changelog_fixture/notes.md"
}
expect_success 'changelog excludes footer links' assert_footer_links_excluded
expect_success 'changelog body has notes' grep -q '^### Added$' "$changelog_fixture/notes.md"
expect_failure 'changelog rejects missing section' run_changelog "$changelog_fixture" 0.2.0 "$changelog_fixture/missing.md"
empty_changelog="$tmp_dir/empty-changelog"
mkdir -p "$empty_changelog"
printf '# Changelog\n\n## [0.2.0]\n\n[0.2.0]: https://example.invalid/0.2.0\n' >"$empty_changelog/CHANGELOG.md"
expect_failure 'changelog rejects empty section' run_changelog "$empty_changelog" 0.2.0 "$empty_changelog/notes.md"

commit=0123456789abcdef0123456789abcdef01234567
valid_run="$tmp_dir/run.json"
cat >"$valid_run" <<EOF
{"head_sha":"$commit","path":".github/workflows/release-prepare.yml","event":"push","status":"completed","conclusion":"success"}
EOF
run_prepare() {
  local json=$1
  local sha=$2
  "$prepare" "$json" "$sha"
}
expect_success 'prepare run accepts valid metadata' run_prepare "$valid_run" "$commit"
expect_failure 'prepare run rejects wrong commit' run_prepare "$valid_run"  fedcba9876543210fedcba9876543210fedcba98
printf '{"head_sha":' >"$tmp_dir/malformed.json"
expect_failure 'prepare run rejects malformed JSON' run_prepare "$tmp_dir/malformed.json" "$commit"
wrong_workflow="$tmp_dir/wrong-workflow.json"
sed 's#release-prepare.yml#other.yml#' "$valid_run" >"$wrong_workflow"
expect_failure 'prepare run rejects wrong workflow' run_prepare "$wrong_workflow" "$commit"

for field in head_sha path event status conclusion; do
  missing="$tmp_dir/missing-$field.json"
  null="$tmp_dir/null-$field.json"
  wrong_type="$tmp_dir/wrong-type-$field.json"
  jq --arg field "$field" 'del(.[$field])' "$valid_run" >"$missing"
  jq --arg field "$field" '.[$field] = null' "$valid_run" >"$null"
  jq --arg field "$field" '.[$field] = 42' "$valid_run" >"$wrong_type"
  expect_failure "prepare run rejects missing $field" run_prepare "$missing" "$commit"
  expect_failure "prepare run rejects null $field" run_prepare "$null" "$commit"
  expect_failure "prepare run rejects non-string $field" run_prepare "$wrong_type" "$commit"
done
non_object="$tmp_dir/non-object.json"
printf '[]\n' >"$non_object"
expect_failure 'prepare run rejects non-object JSON' run_prepare "$non_object" "$commit"

artifact_names=(
  brief-0.1.0-beta.1-linux-x64.tar.gz
  brief-0.1.0-beta.1-linux-arm64.tar.gz
  brief-0.1.0-beta.1-macos-arm64.tar.gz
  brief-0.1.0-beta.1-macos-x64.tar.gz
  brief-0.1.0-beta.1-windows-x64.zip
)
make_artifacts() {
  local dir=$1
  mkdir -p "$dir"
  local name
  for name in "${artifact_names[@]}"; do
    : >"$dir/$name"
  done
}
run_artifacts() {
  local dir=$1
  local version=$2
  "$artifacts" "$dir" "$version"
}

exact_artifacts="$tmp_dir/exact-artifacts"
make_artifacts "$exact_artifacts"
expect_success 'artifacts accept exact five files' run_artifacts "$exact_artifacts" 0.1.0-beta.1
hidden_artifacts="$tmp_dir/hidden-artifacts"
make_artifacts "$hidden_artifacts"
: >"$hidden_artifacts/.hidden"
expect_failure 'artifacts reject hidden entry' run_artifacts "$hidden_artifacts" 0.1.0-beta.1
symlink_artifacts="$tmp_dir/symlink-artifacts"
make_artifacts "$symlink_artifacts"
rm "$symlink_artifacts/${artifact_names[0]}"
ln -s "${artifact_names[1]}" "$symlink_artifacts/${artifact_names[0]}"
expect_failure 'artifacts reject symlink' run_artifacts "$symlink_artifacts" 0.1.0-beta.1
directory_artifacts="$tmp_dir/directory-artifacts"
make_artifacts "$directory_artifacts"
rm "$directory_artifacts/${artifact_names[0]}"
mkdir "$directory_artifacts/${artifact_names[0]}"
expect_failure 'artifacts reject directory' run_artifacts "$directory_artifacts" 0.1.0-beta.1
missing_artifacts="$tmp_dir/missing-artifacts"
make_artifacts "$missing_artifacts"
rm "$missing_artifacts/${artifact_names[0]}"
expect_failure 'artifacts reject missing file' run_artifacts "$missing_artifacts" 0.1.0-beta.1
mismatch_artifacts="$tmp_dir/mismatch-artifacts"
make_artifacts "$mismatch_artifacts"
expect_failure 'artifacts reject version mismatch' run_artifacts "$mismatch_artifacts" 0.1.0

checksum_dir="$tmp_dir/checksums"
make_artifacts "$checksum_dir"
checksum_line() {
  local name=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$name"
  else
    shasum -a 256 "$name"
  fi
}
(
  cd "$checksum_dir"
  for name in "${artifact_names[@]}"; do
    checksum_line "$name"
  done | sort -k2 >SHA256SUMS
)
printf '%s\n' "${artifact_names[@]}" | sort >"$tmp_dir/expected-checksum-names"
awk '{print $2}' "$checksum_dir/SHA256SUMS" | sort >"$tmp_dir/actual-checksum-names"
expect_success 'checksum manifest has exact sorted names' cmp "$tmp_dir/expected-checksum-names" "$tmp_dir/actual-checksum-names"
checksum_check() {
  local dir=$1
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$dir" && sha256sum -c SHA256SUMS)
  else
    (cd "$dir" && shasum -a 256 -c SHA256SUMS)
  fi
}
expect_success 'checksum manifest verifies' checksum_check "$checksum_dir"
printf x >>"$checksum_dir/${artifact_names[0]}"
expect_failure 'checksum rejects mutation' checksum_check "$checksum_dir"

if (( failures > 0 )); then
  printf 'release helper verification: %d test(s) failed\n' "$failures" >&2
  exit 1
fi
printf 'release helper verification: tests passed\n'
