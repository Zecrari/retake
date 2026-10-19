# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/Zecrari/retake/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Zecrari/retake/releases/tag/v0.1.0
