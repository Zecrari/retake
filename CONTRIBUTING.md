# Contributing to retake

Thanks for helping! Bug reports, ideas and pull requests are all welcome.

## Reporting bugs and ideas

Open an [issue](https://github.com/Zecrari/retake/issues/new/choose). For bugs, include your
`retake --version`, OS, the SDK and language you use, and the smallest steps that reproduce it.
**Never paste API keys or unredacted cassettes.**

## Development

You need Rust 1.88 or newer.

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

`tests/proxy.rs` runs the real proxy against a fake LLM API, so you never need an API key to test.

### Project layout

| Path | What it does |
|---|---|
| `src/main.rs` | Command-line interface |
| `src/runner.rs` | Starts the proxy, runs the wrapped command, prints the summary |
| `src/proxy.rs` | HTTP proxy: replays from cassettes or forwards and records |
| `src/store.rs` | Sessions, matching state and divergence diffs |
| `src/matcher.rs` | Decides whether two requests are the same |
| `src/cassette.rs` | On-disk format |
| `src/sse.rs` | Splits event streams into events |
| `src/redact.rs` | Secret detection and redaction |
| `src/check.rs` | `retake check` |
| `site/` | Website, deployed to GitHub Pages |

## Pull requests

- Keep each pull request focused on one change, and add tests for new behavior.
- Make sure `cargo fmt`, `cargo clippy` and `cargo test` pass. CI runs them on Linux, macOS and Windows.
- Add a line to the `Unreleased` section of `CHANGELOG.md` for anything users will notice.
- Changes to the cassette format need a format version bump and a migration path.

## Releasing (maintainers)

1. Update the version in `Cargo.toml` and move `Unreleased` entries in `CHANGELOG.md` under the new version.
2. Commit, then tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. The release workflow builds binaries for every platform and publishes the GitHub release.
4. Move the `v0` tag to the new release so `uses: Zecrari/retake@v0` picks it up:
   `git tag -f v0 && git push -f origin v0`.

By contributing, you agree that your contributions are licensed under the MIT License.
