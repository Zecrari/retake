# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-10-20

### Added
- **Dashboard.** `retake ui` opens a local web UI to browse cassettes as readable conversations
  (OpenAI Chat Completions, OpenAI Responses and Anthropic Messages, including tool calls,
  thinking, streaming and token usage), view raw requests, responses and stream events, and
  delete stale interactions or cassettes.
- **Live view.** Every proxy serves the dashboard at `/__retake/`, streaming each call as it is
  replayed, recorded, passed through or unmatched, with the diff for unmatched calls.
- `--open` on `record`, `replay` and `auto` opens the live dashboard in a browser.

### Changed
- Upstream names may no longer start with `__`, which is reserved for retake's own routes.

### Security
- The dashboard API only answers requests addressed to localhost and requires a custom header
  for changes, so other websites can't read or modify cassettes. Pages are served with a strict
  Content-Security-Policy.

## [0.1.0] - 2026-10-19

### Added
- `record`, `replay` and `auto` modes, with or without a wrapped command.
- Proxying for OpenAI and Anthropic out of the box, plus any upstream via `--upstream NAME=URL`.
- Byte-identical replay of server-sent event streams, with optional `--timing realtime`.
- Request matching that ignores JSON key order and formatting, with `--ignore` for volatile fields.
- Per-request cassettes via the `x-retake-session` header.
- Unified diff against the closest recording when a replayed request doesn't match.
- Secret redaction in cassettes; request headers are never stored.
- `retake check [--fix]` to validate cassettes and scan for leaked secrets.
- Install scripts, a GitHub Action, and prebuilt binaries for Linux, macOS and Windows.

[Unreleased]: https://github.com/Zecrari/retake/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Zecrari/retake/releases/tag/v0.2.0
[0.1.0]: https://github.com/Zecrari/retake/releases/tag/v0.1.0
