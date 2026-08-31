# Changelog

All notable changes to brief are documented here.

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and [Semantic Versioning](https://semver.org/).

## [Unreleased]

No unreleased changes.

## [0.1.0] - 2026-08-31

### Added

- Fold large output from `grep`, `cat`, `find`, `rg`, `cargo`, `git`, `ls`, and `diff`.
- Pass output through byte-for-byte below the estimated token threshold.
- Save complete folded output with an exact recovery command.
- Report full output, returned output, and estimated saved tokens for handled calls.
- Scope folding to project roots with `BRIEF_ROOTS` or a roots file.
- Install the built-in Claude Code adapter with `brief init`.
- Use direct command prefixes with any shell or harness that supports them.
- Create Unix PATH shims with `brief init --shims <dir>` for harnesses that start programs by name.
- Add interactive initialization for terminal input.

### Known limitations

- The folding threshold is an estimate.
- brief buffers selected output until the child exits.
- Pipes can change terminal detection because brief wraps child streams.
- brief stores recovery files on disk and rotation removes older files.
- Commands outside the target list or configured roots pass through without folding.
- Re-read counts detect only reads that also go through brief, so they are a lower bound.

[Unreleased]: https://github.com/farhan-syah/brief/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/farhan-syah/brief/releases/tag/v0.1.0
