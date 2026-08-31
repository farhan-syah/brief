<div align="center">

# brief

<h3>Large command output stays out of agent context while complete output remains recoverable.</h3>

<p><strong>brief</strong> wraps noisy shell commands for developers, scripts, coding agents, and any repository.</p>

<p><a href="#what-is-brief">What is brief?</a> · <a href="#install">Install</a> · <a href="#quickstart">Quickstart</a> · <a href="#harnesses">Harnesses</a> · <a href="#measurement">Measurement</a> · <a href="#limits">Limits</a></p>

<p><a href="https://github.com/farhan-syah/brief/blob/main/LICENSE"><img alt="License: Apache-2.0" src="https://img.shields.io/github/license/farhan-syah/brief"></a></p>

</div>
 
## What is brief?

brief is a size gate for command output.

Large grep, git, cargo, file-listing, and diff output can fill agent context. Users often need only a compact result and a recovery path.

Use brief directly with `brief <program> [args...]`.

| Target programs                                               |
| ------------------------------------------------------------- |
| `grep`, `cat`, `find`, `rg`, `cargo`, `git`, `ls`, and `diff` |

Other commands pass through unchanged. The names `report`, `hook`, and `init` are reserved by brief. Run a literal program with one of those names by path, such as `brief ./init`.

brief is not a command-specific output rewriter or a replacement for the wrapped tools.

## Why use it?

- Keep large command output out of the visible agent context.
- Keep the complete output available for later reading.
- Keep small command output unchanged.
- Measure estimated output tokens saved by handled commands.

## Install

```sh
cargo install brief
brief --version
brief --help
```

## Quickstart

Run brief inside any repository.

```sh
cd /path/to/your/repository
brief rg "TODO" .
brief cargo test
```

Output below the estimated 25,000-token threshold passes byte-for-byte unchanged.

At or above the threshold, brief saves full output and returns a compact head/tail summary with an exact recovery command.

brief waits until the child exits before returning output.

## Harnesses

The core wrapper is harness-agnostic.

Claude Code has the built-in adapter because it exposes a PreToolUse hook format.

Run `brief init --yes` to install that Claude Code hook. Claude Code then invokes `brief hook` for matching Bash calls.

A different harness can prefix supported commands in its command string.

```sh
brief rg "TODO" .
brief cargo test
```

Automatic hook rewriting for another harness requires that harness to provide its own adapter or command-prefix support. Unix PATH shims avoid that requirement on supported Unix systems.

### Unix PATH shims

Use `brief init --shims <dir>` when a harness starts programs by name instead of providing a command string.

```sh
brief init --shims "$HOME/.local/bin/brief-shims"
export PATH="$HOME/.local/bin/brief-shims:$PATH"
rg "TODO" .
cargo test
```

The shim directory must come first on `PATH`.

The shims work with Unix shells and the target programs listed above.

## Repository scope

brief works in any repository and folds output everywhere by default.

Limit folding to absolute roots with `BRIEF_ROOTS`.

```sh
BRIEF_ROOTS="/work/app:/work/lib" brief rg "TODO" .
```

Without `BRIEF_ROOTS`, brief reads `<config-dir>/brief/roots` when that file exists.

List one absolute repository root per line.

Outside the configured roots, commands pass through without folding.

## Measurement

A token is an estimated text unit used by a language model.

| Term         | Meaning                                                     |
| ------------ | ----------------------------------------------------------- |
| Raw tokens   | Estimated tokens in the complete command output.            |
| Kept tokens  | Estimated tokens in the output brief returns to the caller. |
| Saved tokens | Raw tokens minus kept tokens.                               |

When brief folds output, it saves the complete output to disk instead of returning it all.

Run the report with its default window or inspect all retained tracking rows.

```sh
brief report --since all
brief report --since all --format json
```

The report covers only commands brief handled, not all terminal output, model usage, or billing.

Re-read counts only include reads that also go through brief, so they are a lower bound.

### One-PC snapshot

This snapshot uses all retained rows from `brief report --since all` on one PC, captured on 2026-08-31.

| Tool  |             Tracked work |                   Before |                  Returned | Saved tokens |  Rate |
| ----- | -----------------------: | -----------------------: | ------------------------: | -----------: | ----: |
| brief |      4,756 handled calls |   105,198,542 raw tokens |     3,816,975 kept tokens |  101,381,567 | 96.4% |
| RTK   | 234,623 tracked commands | 838,589,245 input tokens | 144,425,960 output tokens |  704,537,499 | 84.0% |

The brief values come from its global report.

The RTK values are RTK estimates from its global savings report.

RTK's saved-token field uses its own accounting and does not equal input minus output in this snapshot.

These snapshots use different tracking stores, definitions, and command populations.

They are not an apples-to-apples benchmark, and neither proves billing savings.

## Config

| Variable                 | Effect                                                                         |
| ------------------------ | ------------------------------------------------------------------------------ |
| `BRIEF_THRESHOLD_TOKENS` | Set the estimated token threshold. Default: `25000`.                           |
| `BRIEF_ENABLED`          | Set `0` or `false` to disable folding.                                         |
| `BRIEF`                  | Short enable or disable alias. `BRIEF_ENABLED` wins when both exist.           |
| `BRIEF_FOLD_DIR`         | Set the directory for complete folded output.                                  |
| `BRIEF_ROOTS`            | Set colon-separated absolute roots for folding. This overrides the roots file. |

## Report flags

| Flag                              | Effect                                       |
| --------------------------------- | -------------------------------------------- |
| `--since <Nd\|Nh\|all\|epoch_ms>` | Select a time window. Default: `30d`.        |
| `--project`                       | Restrict rows to the current directory.      |
| `--format text\|json`             | Select text or JSON output. Default: `text`. |
| `--help`, `-h`                    | Print report help.                           |

## Limits

- brief buffers selected output until the child exits.
- The threshold is an estimate, not an exact token count.
- brief stores complete folded output on disk.
- Rotation removes older recovery files after 20 logs per directory by default.
- Pipes can change terminal detection because brief wraps child streams.
- Commands outside the target list or configured roots pass through without folding.
- Reports do not represent total terminal output, model usage, or billing.
- Re-read counts only include reads that go through brief, so they are a lower bound.

## Uninstall

Remove the Claude Code hook.

```sh
brief init --uninstall
```

Remove only brief-owned Unix PATH shims.

```sh
brief init --shims "$HOME/.local/bin/brief-shims" --uninstall
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
