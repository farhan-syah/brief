#!/usr/bin/env bash
set -euo pipefail

fail() {
  printf '::error::%s\n' "$1" >&2
  exit 1
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  fail "usage: $0 <base-version> [output]"
fi

version=$1
output=${2:-/tmp/changelog-section.md}
if [[ ! $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  fail "invalid base version '$version'"
fi
[[ -n $output ]] || fail 'output path must not be empty'
[[ -f CHANGELOG.md ]] || fail 'CHANGELOG.md is required to cut a release'
command -v python3 >/dev/null 2>&1 || fail 'python3 is required'

python3 - "$version" "$output" <<'PY'
from __future__ import annotations

import os
import re
import sys
import tempfile
from pathlib import Path

version, output_name = sys.argv[1:]


def fail(message: str) -> None:
    print(f"::error::{message}", file=sys.stderr)
    raise SystemExit(1)


changelog_path = Path("CHANGELOG.md")
try:
    content = changelog_path.read_text(encoding="utf-8")
except OSError as error:
    fail(f"cannot read CHANGELOG.md: {error}")

heading = f"## [{version}]"
lines = content.splitlines(keepends=True)


def is_version_heading(line: str) -> bool:
    text = line.rstrip("\r\n")
    if not text.startswith(heading):
        return False
    suffix = text[len(heading) :]
    return suffix == "" or suffix.startswith(" - ")


heading_indexes = [index for index, line in enumerate(lines) if is_version_heading(line)]
if not heading_indexes:
    fail(f"CHANGELOG.md has no non-empty '## [{version}]' section. Rename the Unreleased heading to [{version}] and describe the release.")
if len(heading_indexes) != 1:
    fail(f"CHANGELOG.md has duplicate '## [{version}]' headings")

start = heading_indexes[0] + 1
end = len(lines)
for index in range(start, len(lines)):
    text = lines[index].rstrip("\r\n")
    if text.startswith("## [") or re.match(r"^\s{0,3}\[[^\]\r\n]+\]:\s+\S", text):
        end = index
        break

section = lines[start:end]
while section and not section[0].strip():
    section.pop(0)
while section and not section[-1].strip():
    section.pop()
while section and section[-1].strip() == "---":
    section.pop()
    while section and not section[-1].strip():
        section.pop()
body = "".join(section)
if not body.strip():
    fail(f"CHANGELOG.md has no non-empty '## [{version}]' section. Rename the Unreleased heading to [{version}] and describe the release.")

output_path = Path(output_name)
try:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{output_path.name}.", dir=output_path.parent)
    temporary_path = Path(temporary)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="") as handle:
            handle.write(body)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary_path, output_path)
    finally:
        if temporary_path.exists():
            temporary_path.unlink()
except OSError as error:
    fail(f"cannot write {output_path}: {error}")

print(f"Changelog section for [{version}]:")
print(body, end="" if body.endswith("\n") else "\n")
PY
