"""Example: testing an LLM-backed function with pytest and retake.

    pip install openai pytest
    OPENAI_API_KEY=sk-... retake record -- pytest   # once, records cassettes/
    retake replay -- pytest                         # offline, free, deterministic
"""

import re

import pytest
from openai import OpenAI


def summarize(client: OpenAI, text: str) -> str:
    response = client.chat.completions.create(
        model="gpt-4.1-mini",
        messages=[
            {"role": "system", "content": "Summarize in one short sentence."},
            {"role": "user", "content": text},
        ],
    )
    return response.choices[0].message.content


@pytest.fixture
def client(request):
    # One cassette per test, e.g. cassettes/test_summarize.py/test_summary_is_short.json
    session = re.sub(r"[^A-Za-z0-9._/-]", "_", request.node.nodeid.replace("::", "/"))
    # OPENAI_BASE_URL is set by retake, so the client talks to the proxy.
    return OpenAI(default_headers={"x-retake-session": session})


def test_summary_is_short(client):
    summary = summarize(client, "Retake records LLM API calls once and replays them in tests. " * 5)
    assert 0 < len(summary) < 200


def test_streaming(client):
    stream = client.chat.completions.create(
        model="gpt-4.1-mini",
        messages=[{"role": "user", "content": "Count from 1 to 5."}],
        stream=True,
    )
    text = "".join(chunk.choices[0].delta.content or "" for chunk in stream if chunk.choices)
    assert "5" in text
