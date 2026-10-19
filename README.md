<div align="center">

# retake

**Record LLM API calls once. Replay them deterministically in tests.**

A single-binary, language-agnostic record/replay proxy for OpenAI, Anthropic and any OpenAI-compatible API.
Make tests for LLM apps and agents fast, free, offline and identical on every run.

[![CI](https://github.com/Zecrari/retake/actions/workflows/ci.yml/badge.svg)](https://github.com/Zecrari/retake/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Zecrari/retake?sort=semver)](https://github.com/Zecrari/retake/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Website](https://zecrari.github.io/retake/) · [Documentation](https://zecrari.github.io/retake/docs/) · [Releases](https://github.com/Zecrari/retake/releases)

</div>

---

## Why

Tests that call LLM APIs are **flaky** (answers change every run), **expensive** (every CI run burns credits),
**slow**, and **need a network and an API key**. Agents make it worse: one test can make dozens of calls,
each depending on the last.

`retake` sits between your code and the API. Record a real run once, commit the cassette, and every
later run replays it — offline, in milliseconds, byte-for-byte the same.

```console
$ retake record -- npm test        # real API calls, saved to ./cassettes
$ retake replay -- npm test        # no network, no key, no cost, same results
```

No SDK plugin and no code changes: `retake` sets `OPENAI_BASE_URL` and `ANTHROPIC_BASE_URL` for the
command it runs, and the official SDKs pick them up automatically — in Python, TypeScript, Go, Java,
.NET, Ruby or plain `curl`.

## Features

- **Language-agnostic.** It's an HTTP proxy, so it works with any SDK or HTTP client.
- **Streaming done right.** Server-sent event streams are stored as whole events and replayed byte-for-byte.
  Optional realistic pacing with `--timing realtime`.
- **Built for agents.** Repeated identical calls replay in recorded order, and parallel calls match independently.
- **Clear failures.** An unrecorded call fails with a `404` and a diff against the closest recording —
  never a silent real API call, and never a `5xx` that SDKs would retry.
- **Reviewable cassettes.** Pretty-printed JSON with stable formatting, so a prompt change shows up
  in the pull request next to the response change.
- **Safe to commit.** Request headers are never stored, and anything that looks like an API key,
  token or private key is redacted. `retake check` scans cassettes in CI or as a pre-commit hook.
- **One cassette per test.** Pick the cassette per request with the `x-retake-session` header.
- **Single static binary.** No Python, Node or Docker needed. Linux, macOS and Windows.

## Install

```sh
# macOS / Linux
curl -fsSL https://zecrari.github.io/retake/install.sh | sh
```

```powershell
# Windows (PowerShell)
irm https://zecrari.github.io/retake/install.ps1 | iex
```

Or download a binary from [Releases](https://github.com/Zecrari/retake/releases), or build from source:

```sh
cargo install --git https://github.com/Zecrari/retake --locked
```

## Quick start

```sh
# 1. Record. Calls go to the real API; responses are saved to ./cassettes/default.json
OPENAI_API_KEY=sk-... retake record -- pytest

# 2. Replay. Nothing leaves your machine. Unrecorded calls fail the run.
retake replay -- pytest

# 3. Day to day: replay what exists, record only what's new
retake auto -- pytest
```

Commit the `cassettes/` directory. In CI, run `retake replay`.

### One cassette per test

Send the `x-retake-session` header to pick a cassette. Session names may contain `/` to create folders.

```python
# conftest.py
import re, pytest
from openai import OpenAI

@pytest.fixture
def client(request):
    session = re.sub(r"[^A-Za-z0-9._/-]", "_", request.node.nodeid.replace("::", "/"))
    return OpenAI(default_headers={"x-retake-session": session})
```

```ts
// TypeScript (Vitest)
import OpenAI from "openai";
import { beforeEach, expect } from "vitest";

let client: OpenAI;
beforeEach(() => {
  const name = expect.getState().currentTestName ?? "default";
  client = new OpenAI({ defaultHeaders: { "x-retake-session": name.replace(/[^A-Za-z0-9._/-]+/g, "_") } });
});
```

See [`examples/`](examples) for complete Python and TypeScript projects.

### GitHub Actions

```yaml
- uses: Zecrari/retake@v0
- run: retake check            # fail if a cassette contains a secret
- run: retake replay -- npm test
```

## How it works

```
your tests ──► retake (localhost) ──► api.openai.com / api.anthropic.com / your upstream
                   │
                   └── cassettes/<session>.json
```

`retake <mode> -- <command>` starts a proxy on a free local port and runs your command with:

| Variable | Value |
|---|---|
| `OPENAI_BASE_URL` | `http://127.0.0.1:<port>/openai/v1` |
| `ANTHROPIC_BASE_URL` | `http://127.0.0.1:<port>/anthropic` |
| `RETAKE_<NAME>_URL` | `http://127.0.0.1:<port>/<name>` for every upstream |
| `RETAKE_URL`, `RETAKE_MODE` | the proxy address and mode |

In `replay` mode, `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` get placeholder values if unset, so SDKs
don't refuse to start in CI.

Requests match when method, upstream, path, query and body are equal. JSON bodies are compared
after sorting keys, so formatting never matters. Use `--ignore metadata.request_id` to skip
fields that change on every run.

### Modes

| Mode | Recorded call | New call |
|---|---|---|
| `record` | — (cassettes start empty) | forwarded and saved |
| `replay` | served from cassette | **fails** with a diff |
| `auto` | served from cassette | forwarded and saved |

`429` and `5xx` responses are passed through but not recorded (use `--record-errors` to keep them).

### Other upstreams

Any OpenAI-compatible server works — Ollama, vLLM, LM Studio, Groq, OpenRouter, Azure OpenAI:

```sh
retake record --upstream ollama=http://localhost:11434 -- python app.py
# app uses base URL $RETAKE_OLLAMA_URL/v1
```

## CLI reference

```text
retake record|replay|auto [OPTIONS] [-- COMMAND...]
  --cassettes <DIR>        Cassette directory [env: RETAKE_CASSETTES] [default: cassettes]
  -s, --session <NAME>     Default cassette name [env: RETAKE_SESSION] [default: default]
  -p, --port <PORT>        Port, 0 = any free port [env: RETAKE_PORT] [default: 0]
  --host <ADDR>            Listen address [default: 127.0.0.1]
  --upstream <NAME=URL>    Add or override an upstream (repeatable)
  --ignore <FIELD>         Dotted JSON path to ignore when matching (repeatable)
  --timing <fast|realtime> Replay pacing [default: fast]
  --record-timing          Store event timing (needed for --timing realtime)
  --record-errors          Also record 429 and 5xx responses

retake check [PATHS...] [--fix]   Validate cassettes and scan for secrets
```

Without a command, the proxy runs until Ctrl+C and prints the variables to set.
Exit codes: the command's own exit code; `1` if replay had unmatched calls; `2` for retake errors.

## Compared with other tools

| | retake | [reel](https://github.com/tathagat22/reel) | [llm-tracelab](https://github.com/kingfs/llm-tracelab) | LiteLLM caching |
|---|---|---|---|---|
| Works from any language | ✅ proxy | ✅ proxy | ✅ proxy | ✅ proxy |
| Install | single binary | Python package | Go binary | Python package |
| Streaming replay | ✅ byte-identical | ✅ | ✅ | partial |
| Diff on mismatch | ✅ | — | — | — |
| Focus | tests & CI | tests | tracing & observability | production gateway |

If you only use Python and want fuzzy matching, `reel` is worth a look too.

## Roadmap

- Ordered agent-session matching that reports the first step where a run diverged
- Gemini and Bedrock request normalization
- A local web UI to browse and diff cassettes
- `retake prune` to remove unused recordings

Ideas and feedback are very welcome in [issues](https://github.com/Zecrari/retake/issues).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: see [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE)
