# Security policy

## Reporting a vulnerability

Please **do not open a public issue** for security problems. Report them privately through
[GitHub security advisories](https://github.com/Zecrari/retake/security/advisories/new).

You can expect an acknowledgement within 3 working days. Once a fix is ready, we'll publish a
release and an advisory, and credit you unless you prefer otherwise.

## Supported versions

Security fixes go into the latest release.

## What retake protects

- Request headers (where API keys live) are never written to cassettes.
- Request and response bodies are scanned for common secret formats (OpenAI, Anthropic, Google,
  GitHub, AWS, Slack, bearer tokens, private keys), and matches are replaced with `[REDACTED]`.
- The proxy listens on `127.0.0.1` by default.

Redaction is pattern-based and can't catch everything — for example personal data in prompts or
custom token formats. Review cassettes before committing them, and run `retake check` in CI.
Don't expose the proxy on a public interface: anyone who can reach it in `record` mode can make
API calls with the keys your application sends.
