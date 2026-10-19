// Example: testing an Anthropic tool-calling loop with Vitest and retake.
//
//   npm install @anthropic-ai/sdk vitest
//   ANTHROPIC_API_KEY=sk-ant-... retake record -- npx vitest run   # once
//   retake replay -- npx vitest run                                # offline, deterministic
import Anthropic from "@anthropic-ai/sdk";
import { beforeEach, expect, test } from "vitest";

let client: Anthropic;

beforeEach(() => {
  // One cassette per test. ANTHROPIC_BASE_URL is set by retake.
  const name = expect.getState().currentTestName ?? "default";
  client = new Anthropic({
    defaultHeaders: { "x-retake-session": `agent/${name.replace(/[^A-Za-z0-9._/-]+/g, "_")}` },
  });
});

const tools: Anthropic.Tool[] = [
  {
    name: "lookup_order",
    description: "Look up an order by id",
    input_schema: { type: "object", properties: { order_id: { type: "string" } }, required: ["order_id"] },
  },
];

function lookupOrder(orderId: string) {
  return { order_id: orderId, status: "delivered", total: 49.0 };
}

test("agent looks up the order before answering", async () => {
  const messages: Anthropic.MessageParam[] = [{ role: "user", content: "What is the status of order 1042?" }];

  // Every call in this loop is recorded once and replayed in order.
  for (let step = 0; step < 5; step++) {
    const response = await client.messages.create({ model: "claude-sonnet-5-5", max_tokens: 512, tools, messages });
    messages.push({ role: "assistant", content: response.content });
    if (response.stop_reason !== "tool_use") break;

    const results = response.content
      .filter((block): block is Anthropic.ToolUseBlock => block.type === "tool_use")
      .map((block) => ({
        type: "tool_result" as const,
        tool_use_id: block.id,
        content: JSON.stringify(lookupOrder((block.input as { order_id: string }).order_id)),
      }));
    messages.push({ role: "user", content: results });
  }

  const last = messages[messages.length - 1];
  expect(JSON.stringify(last.content)).toMatch(/delivered/i);
});
