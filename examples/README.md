# Examples

| Example | Shows |
|---|---|
| [`python/`](python) | pytest, OpenAI SDK, one cassette per test, streaming |
| [`typescript/`](typescript) | Vitest, Anthropic SDK, a multi-step tool-calling agent |

Each file's header shows the record and replay commands. Record once with a real API key,
commit `cassettes/`, then run `retake replay` everywhere else — including CI, with no key at all.
