#!/usr/bin/env bash
set -euo pipefail

fail() {
  printf 'stamp version: %s\n' "$1" >&2
  exit 1
}

if [[ $# -ne 1 ]]; then
  fail "usage: $0 <version>"
fi

version=$1
if [[ ! $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.(0|[1-9][0-9]*))?$ ]]; then
  fail "invalid version '$version'"
fi

command -v python3 >/dev/null 2>&1 || fail 'python3 is required'

base=${version%%-*}
python3 - "$version" "$base" <<'PY'
from __future__ import annotations

import os
import re
import stat
import sys
import tempfile
from pathlib import Path

version, base = sys.argv[1:]


def fail(message: str) -> None:
    print(f"stamp version: {message}", file=sys.stderr)
    raise SystemExit(1)


def read_file(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except FileNotFoundError:
        fail(f"{path} is required")
    except OSError as error:
        fail(f"cannot read {path}: {error}")


def write_temporary(path: Path, content: bytes) -> Path:
    try:
        mode = stat.S_IMODE(path.stat().st_mode)
        fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    except OSError as error:
        fail(f"cannot prepare {path}: {error}")

    temporary_path = Path(temporary)
    try:
        with os.fdopen(fd, "wb") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary_path, mode)
        return temporary_path
    except OSError as error:
        if temporary_path.exists():
            temporary_path.unlink()
        fail(f"cannot prepare {path}: {error}")


def replace_pair(
    manifest_content: bytes,
    lockfile_content: bytes,
    original_manifest: bytes,
) -> None:
    manifest_temporary = write_temporary(manifest_path, manifest_content)
    lock_temporary = write_temporary(lock_path, lockfile_content)
    manifest_replaced = False
    try:
        os.replace(manifest_temporary, manifest_path)
        manifest_replaced = True
        os.replace(lock_temporary, lock_path)
    except OSError as error:
        if manifest_replaced:
            try:
                rollback_temporary = write_temporary(manifest_path, original_manifest)
                try:
                    os.replace(rollback_temporary, manifest_path)
                finally:
                    if rollback_temporary.exists():
                        rollback_temporary.unlink()
            except OSError as rollback_error:
                fail(f"cannot replace {lock_path}: {error}; rollback failed: {rollback_error}")
        fail(f"cannot replace release version files: {error}")
    finally:
        if manifest_temporary.exists():
            manifest_temporary.unlink()
        if lock_temporary.exists():
            lock_temporary.unlink()


manifest_path = Path("Cargo.toml")
lock_path = Path("Cargo.lock")
manifest = read_file(manifest_path)
lockfile = read_file(lock_path)

package_headers = list(re.finditer(rb"(?m)^\[package\]\r?$", manifest))
if len(package_headers) != 1:
    fail("Cargo.toml must contain exactly one [package] section")

package_start = package_headers[0].end()
next_section = re.search(rb"(?m)^\[[^\r\n]+\]\r?$", manifest[package_start:])
package_end = package_start + next_section.start() if next_section else len(manifest)
package_section = manifest[package_start:package_end]
manifest_names = list(re.finditer(rb'(?m)^name = "([^"\r\n]*)"\r?$', package_section))
if len(manifest_names) != 1 or manifest_names[0].group(1) != b"ogt":
    fail('Cargo.toml [package] must name exactly one package: ogt')
manifest_versions = list(re.finditer(rb'(?m)^version = "([^"\r\n]*)"\r?$', package_section))
if len(manifest_versions) != 1:
    fail("Cargo.toml [package] must contain exactly one version")
manifest_match = manifest_versions[0]
manifest_version = manifest_match.group(1).decode("utf-8")

package_blocks = list(re.finditer(rb"(?ms)^\[\[package\]\]\r?\n.*?(?=^\[\[package\]\]\r?\n|\Z)", lockfile))
ogt_blocks = [block for block in package_blocks if re.search(rb'(?m)^name = "ogt"\r?$', block.group(0))]
root_ogt_blocks = [
    block for block in ogt_blocks if not re.search(rb"(?m)^source = ", block.group(0))
]
if len(root_ogt_blocks) != 1:
    fail('Cargo.lock must contain exactly one root [[package]] block named ogt')

ogt_block = root_ogt_blocks[0]
lock_versions = list(re.finditer(rb'(?m)^version = "([^"\r\n]*)"\r?$', ogt_block.group(0)))
if len(lock_versions) != 1:
    fail('Cargo.lock ogt block must contain exactly one version')
lock_match = lock_versions[0]
lock_version = lock_match.group(1).decode("utf-8")

if manifest_version != lock_version:
    fail(f"Cargo.toml and Cargo.lock versions drift: {manifest_version} != {lock_version}")
if manifest_version not in {base, version}:
    fail(f"tracked version {manifest_version} does not match requested base {base}")
if lock_version not in {base, version}:
    fail(f"tracked version {lock_version} does not match requested base {base}")

if manifest_version == version:
    print(f"Version already {version} — nothing to stamp.")
    raise SystemExit(0)

manifest_offset = package_start + manifest_match.start(1)
manifest_end = package_start + manifest_match.end(1)
lock_offset = ogt_block.start() + lock_match.start(1)
lock_end = ogt_block.start() + lock_match.end(1)

stamped_manifest = manifest[:manifest_offset] + version.encode() + manifest[manifest_end:]
stamped_lockfile = lockfile[:lock_offset] + version.encode() + lockfile[lock_end:]
replace_pair(stamped_manifest, stamped_lockfile, manifest)
print(f"Stamped package version: {manifest_version} -> {version} (Cargo.toml + Cargo.lock)")
PY
