<div align="center">

# ogt

<h3>Large command output stays out of agent context while complete output remains recoverable.</h3>

<p><strong>Output Gate Tool</strong>. It wraps noisy shell commands for developers, scripts, coding agents, and any repository.</p>
 
<p><a href="#what-is-ogt">What is ogt?</a> · <a href="#install">Install</a> · <a href="#quickstart">Quickstart</a> · <a href="#harnesses">Harnesses</a> · <a href="#measurement">Measurement</a> · <a href="#limits">Limits</a></p>

<p><a href="https://github.com/farhan-syah/ogt/blob/main/LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/github/license/farhan-syah/ogt"></a></p>

</div>

## What is ogt?

OGT (Output Gate Tool) is a size gate for command output. It gates large output before it enters the caller's context.

Large grep, git, cargo, file-listing, and diff output can fill agent context. Users often need only a compact result and a recovery path.

Use ogt directly with `ogt <program> [args...]`.

| Target programs                                               |
| ------------------------------------------------------------- |
| `grep`, `cat`, `find`, `rg`, `cargo`, `git`, `ls`, and `diff` |

Other commands pass through unchanged. The names `report`, `hook`, and `init` are reserved by ogt. Run a literal program with one of those names by path, such as `ogt ./init`.

ogt is not a command-specific output rewriter or a replacement for the wrapped tools.

## Why use it?

- Keep large command output out of the visible agent context.
- Keep the complete output available for later reading.
- Keep small command output unchanged.
- Measure estimated output tokens saved by handled commands.

## Install

```sh
cargo install ogt
ogt --version
ogt --help
```

`cargo install ogt` is the source-install fallback.

### Prebuilt binaries

GitHub releases provide archives for `linux-x64`, `linux-arm64`, `macos-arm64`, `macos-x64`, and `windows-x64`.

Each Unix archive contains `ogt`, `LICENSE`, and `NOTICE`. The Windows archive contains `ogt.exe`, `LICENSE`, and `NOTICE`.

`SHA256SUMS` detects transfer corruption but is not a signed build attestation. Prereleases are marked and are not the latest release.

Download and install the Linux x64 archive into a user-writable directory:

```sh
version=0.1.0
label=linux-x64
archive=ogt-${version}-${label}.tar.gz
base_url="https://github.com/farhan-syah/ogt/releases/download/v${version}"
curl -fsSLO "$base_url/$archive"
curl -fsSLO "$base_url/SHA256SUMS"
grep -F "  $archive" SHA256SUMS | sha256sum -c -
tmp=$(mktemp -d)
tar -xzf "$archive" -C "$tmp"
mkdir -p "$HOME/.local/bin"
install -m 0755 "$tmp/ogt" "$HOME/.local/bin/ogt"
rm -rf "$tmp"
```

On Windows, use PowerShell and add `$HOME\bin` to your user `PATH`:

```powershell
$version = "0.1.0"
$label = "windows-x64"
$archive = "ogt-$version-$label.zip"
$baseUrl = "https://github.com/farhan-syah/ogt/releases/download/v$version"
Invoke-WebRequest "$baseUrl/$archive" -OutFile $archive
Invoke-WebRequest "$baseUrl/SHA256SUMS" -OutFile SHA256SUMS
$line = Get-Content SHA256SUMS | Where-Object { $_ -like "*  $archive" }
$expected = (($line -split "\s+")[0]).ToLowerInvariant()
$actual = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "SHA256SUMS check failed" }
$destination = Join-Path $HOME "bin"
New-Item -ItemType Directory -Force $destination | Out-Null
Expand-Archive $archive -DestinationPath $destination -Force
```

## Quickstart

Run ogt inside any repository.

```sh
cd /path/to/your/repository
ogt rg "TODO" .
ogt cargo test
```

Output below the estimated 25,000-token threshold passes byte-for-byte unchanged.

At or above the threshold, ogt saves full output and returns a compact head/tail summary with an exact recovery command.

ogt waits until the child exits before returning output.

## Harnesses

The core wrapper is harness-agnostic.

Claude Code has the built-in adapter because it exposes a PreToolUse hook format.

Run `ogt init --yes` to install that Claude Code hook. Claude Code then invokes `ogt hook` for matching Bash calls.

A different harness can prefix supported commands in its command string.

```sh
ogt rg "TODO" .
ogt cargo test
```

Automatic hook rewriting for another harness requires that harness to provide its own adapter or command-prefix support. Unix PATH shims avoid that requirement on supported Unix systems.

### Unix PATH shims

Use `ogt init --shims <dir>` when a harness starts programs by name instead of providing a command string.

```sh
ogt init --shims "$HOME/.local/bin/ogt-shims"
export PATH="$HOME/.local/bin/ogt-shims:$PATH"
rg "TODO" .
cargo test
```

The shim directory must come first on `PATH`.

The shims work with Unix shells and the target programs listed above.

## Repository scope

ogt works in any repository and folds output everywhere by default.

Limit folding to absolute roots with `OGT_ROOTS`.

Use the platform path-list separator: `:` on Unix and `;` on Windows.

```sh
OGT_ROOTS="/work/app:/work/lib" ogt rg "TODO" .
```

Without `OGT_ROOTS`, ogt reads `<config-dir>/ogt/roots` when that file exists.

List one absolute repository root per line.

Outside the configured roots, commands pass through without folding.

## Measurement

A token is an estimated text unit used by a language model.

| Term         | Meaning                                                   |
| ------------ | --------------------------------------------------------- |
| Raw tokens   | Estimated tokens in the complete command output.          |
| Shown tokens | Estimated tokens in the output ogt returns to the caller. |
| Saved tokens | Raw tokens minus shown tokens.                            |

When ogt folds output, it saves the complete output to disk instead of returning it all.

Run the report with its default window or inspect all retained tracking rows.

```sh
ogt report --since all
ogt report --since all --format json
```

The report covers only commands ogt handled, not all terminal output, model usage, or billing.

Re-read counts only include reads that also go through ogt, so they are a lower bound.

### One-PC snapshot

This snapshot uses all retained rows from `ogt report --since all` on one PC, captured on 2026-08-31.

| Tool |             Tracked work |  Raw tokens | Shown tokens | Saved tokens |  Rate |
| ---- | -----------------------: | ----------: | -----------: | -----------: | ----: |
| ogt  |      4,756 handled calls | 105,198,542 |    3,816,975 |  101,381,567 | 96.4% |
| RTK  | 234,623 tracked commands | 838,589,245 |  144,425,960 |  704,537,499 | 84.0% |

The ogt values come from its global report.

The RTK values are RTK estimates from its global savings report.

RTK's saved-token field uses its own accounting and does not equal input minus output in this snapshot.

These snapshots use different tracking stores, definitions, and command populations.

They are not an apples-to-apples benchmark, and neither proves billing savings.

## Config

| Variable               | Effect                                                                            |
| ---------------------- | --------------------------------------------------------------------------------- |
| `OGT_THRESHOLD_TOKENS` | Set the estimated token threshold. Default: `25000`.                              |
| `OGT_ENABLED`          | Set `0` or `false` to disable folding.                                            |
| `OGT`                  | Short enable or disable alias. `OGT_ENABLED` wins when both exist.                |
| `OGT_FOLD_DIR`         | Set the directory for complete folded output.                                     |
| `OGT_ROOTS`            | Set platform-separated absolute roots for folding. This overrides the roots file. |

## Report flags

| Flag                              | Effect                                       |
| --------------------------------- | -------------------------------------------- |
| `--since <Nd\|Nh\|all\|epoch_ms>` | Select a time window. Default: `30d`.        |
| `--project`                       | Restrict rows to the current directory.      |
| `--format text\|json`             | Select text or JSON output. Default: `text`. |
| `--help`, `-h`                    | Print report help.                           |

## Limits

- ogt buffers selected output until the child exits.
- The threshold is an estimate, not an exact token count.
- ogt stores complete folded output on disk.
- Rotation removes older recovery files after 20 logs per directory by default.
- Pipes can change terminal detection because ogt wraps child streams.
- Commands outside the target list or configured roots pass through without folding.
- Reports do not represent total terminal output, model usage, or billing.
- Re-read counts only include reads that go through ogt, so they are a lower bound.

## Uninstall

Remove the Claude Code hook.

```sh
ogt init --uninstall
```

Remove only ogt-owned Unix PATH shims.

```sh
ogt init --shims "$HOME/.local/bin/ogt-shims" --uninstall
```

## Contributing

Run the test suite and formatter before submitting changes.

```sh
cargo test
cargo fmt --all -- --check
```

Read [CHANGELOG.md](CHANGELOG.md) for release notes.

## License

[Apache-2.0](LICENSE)
