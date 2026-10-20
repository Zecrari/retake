// retake dashboard. Everything is built with DOM APIs (never innerHTML), because
// cassettes contain arbitrary text recorded from APIs.
(function () {
  "use strict";

  var API = "api/";
  var state = {
    info: null,
    sessions: [],
    filter: "",
    session: null, // { name, cassette }
    selected: -1,
    tab: "conversation",
    events: [],
    eventId: null, // selected live event
    live: { connected: false, replayed: 0, recorded: 0, missed: 0 },
  };

  // ---------- DOM helpers ----------

  function h(tag, attrs) {
    var el = document.createElement(tag);
    if (attrs) {
      Object.keys(attrs).forEach(function (key) {
        var value = attrs[key];
        if (value === null || value === undefined || value === false) return;
        if (key === "class") el.className = value;
        else if (key === "text") el.textContent = value;
        else if (key.slice(0, 2) === "on") el.addEventListener(key.slice(2), value);
        else el.setAttribute(key, value === true ? "" : value);
      });
    }
    for (var i = 2; i < arguments.length; i++) append(el, arguments[i]);
    return el;
  }

  function append(el, child) {
    if (child === null || child === undefined || child === false) return;
    if (Array.isArray(child)) child.forEach(function (c) { append(el, c); });
    else el.appendChild(typeof child === "string" || typeof child === "number" ? document.createTextNode(String(child)) : child);
  }

  function svg(paths) {
    var ns = "http://www.w3.org/2000/svg";
    var el = document.createElementNS(ns, "svg");
    el.setAttribute("viewBox", "0 0 24 24");
    el.setAttribute("aria-hidden", "true");
    paths.forEach(function (d) {
      var p = document.createElementNS(ns, "path");
      p.setAttribute("d", d);
      el.appendChild(p);
    });
    return el;
  }

  var ICONS = {
    back: ["M15 18l-6-6 6-6"],
    trash: ["M3 6h18", "M8 6V4h8v2", "M19 6l-1 14H6L5 6", "M10 11v6", "M14 11v6"],
    copy: ["M9 9h11v11H9z", "M5 15H4V4h11v1"],
    tape: ["M3 6h18v12H3z", "M7.5 14.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z", "M16.5 14.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z", "M7.5 14.5h9"],
    pulse: ["M3 12h4l3-8 4 16 3-8h4"],
    click: ["M9 3v4", "M3 9h4", "M5 5l2.5 2.5", "M10 10l10 4-4 2-2 4z"],
  };

  function clear(el) {
    while (el.firstChild) el.removeChild(el.firstChild);
    return el;
  }

  var $ = function (id) { return document.getElementById(id); };

  // ---------- Formatting ----------

  function fmtBytes(n) {
    if (n < 1024) return n + " B";
    if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
    return (n / 1024 / 1024).toFixed(1) + " MB";
  }

  function fmtAgo(ms) {
    if (!ms) return "";
    var s = Math.max(0, (Date.now() - ms) / 1000);
    if (s < 60) return "just now";
    if (s < 3600) return Math.floor(s / 60) + " min ago";
    if (s < 86400) return Math.floor(s / 3600) + " h ago";
    if (s < 86400 * 30) return Math.floor(s / 86400) + " d ago";
    return new Date(ms).toLocaleDateString();
  }

  function fmtTime(ms) {
    var d = new Date(ms);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }

  function fmtNum(n) {
    return n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1) + "k" : String(n);
  }

  function statusPill(status) {
    if (status === undefined || status === null) return h("span", { class: "pill warn", text: "—" });
    var cls = status < 300 ? "ok" : status < 500 ? "warn" : "rec";
    return h("span", { class: "pill " + cls, text: String(status) });
  }

  // ---------- JSON rendering ----------

  function jsonView(value) {
    var pre = h("pre");
    var code = h("code");
    pre.appendChild(code);
    writeJson(code, value, 0);
    return pre;
  }

  function writeJson(out, value, depth) {
    var pad = "  ";
    var indent = new Array(depth + 1).join(pad);
    if (value === null) return out.appendChild(h("span", { class: "j-lit", text: "null" }));
    if (typeof value === "boolean") return out.appendChild(h("span", { class: "j-lit", text: String(value) }));
    if (typeof value === "number") return out.appendChild(h("span", { class: "j-num", text: String(value) }));
    if (typeof value === "string") return out.appendChild(h("span", { class: "j-str", text: JSON.stringify(value) }));
    if (Array.isArray(value)) {
      if (!value.length) return out.appendChild(h("span", { class: "j-punct", text: "[]" }));
      out.appendChild(h("span", { class: "j-punct", text: "[" }));
      value.forEach(function (item, i) {
        out.appendChild(document.createTextNode("\n" + indent + pad));
        writeJson(out, item, depth + 1);
        if (i < value.length - 1) out.appendChild(h("span", { class: "j-punct", text: "," }));
      });
      out.appendChild(document.createTextNode("\n" + indent));
      return out.appendChild(h("span", { class: "j-punct", text: "]" }));
    }
    var keys = Object.keys(value);
    if (!keys.length) return out.appendChild(h("span", { class: "j-punct", text: "{}" }));
    out.appendChild(h("span", { class: "j-punct", text: "{" }));
    keys.forEach(function (key, i) {
      out.appendChild(document.createTextNode("\n" + indent + pad));
      out.appendChild(h("span", { class: "j-key", text: JSON.stringify(key) }));
      out.appendChild(h("span", { class: "j-punct", text: ": " }));
      writeJson(out, value[key], depth + 1);
      if (i < keys.length - 1) out.appendChild(h("span", { class: "j-punct", text: "," }));
    });
    out.appendChild(document.createTextNode("\n" + indent));
    out.appendChild(h("span", { class: "j-punct", text: "}" }));
  }

  function codeBlock(content, copyText) {
    var block = h("div", { class: "code" }, content);
    if (copyText !== undefined) {
      block.appendChild(
        h("button", {
          class: "btn copy",
          type: "button",
          title: "Copy",
          onclick: function () { copy(copyText); },
        }, svg(ICONS.copy), "Copy")
      );
    }
    return block;
  }

  function jsonBlock(value) {
    return codeBlock(jsonView(value), JSON.stringify(value, null, 2));
  }

  function textBlock(text) {
    return codeBlock(h("pre", null, h("code", { text: text })), text);
  }

  function diffBlock(text) {
    var code = h("code", { class: "diff" });
    text.split("\n").forEach(function (line) {
      var cls = line.indexOf("@@") === 0 ? "hunk" : line[0] === "+" && line.indexOf("+++") !== 0 ? "add" : line[0] === "-" && line.indexOf("---") !== 0 ? "del" : null;
      if (cls) code.appendChild(h("span", { class: cls, text: line }));
      else code.appendChild(document.createTextNode(line + "\n"));
    });
    return codeBlock(h("pre", null, code), text);
  }

  function copy(text) {
    if (!navigator.clipboard) return toast("Clipboard not available", true);
    navigator.clipboard.writeText(text).then(function () { toast("Copied"); }, function () { toast("Copy failed", true); });
  }

  var toastTimer;
  function toast(message, isError) {
    var el = $("toast");
    el.textContent = message;
    el.className = "toast" + (isError ? " error" : "");
    el.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { el.hidden = true; }, 2200);
  }

  // ---------- API ----------

  function api(path, options) {
    options = options || {};
    var headers = { Accept: "application/json" };
    if (options.method && options.method !== "GET") headers["x-retake-ui"] = "1";
    return fetch(API + path, { method: options.method || "GET", headers: headers }).then(function (res) {
      if (res.status === 204) return null;
      return res.json().then(function (body) {
        if (!res.ok) throw new Error(body && body.error ? body.error : "HTTP " + res.status);
        return body;
      });
    });
  }

  // ---------- Understanding LLM traffic ----------

  function parseSse(events) {
    var out = [];
    (events || []).forEach(function (ev) {
      var name = null;
      var data = [];
      ev.data.split(/\r?\n/).forEach(function (line) {
        if (line.indexOf("event:") === 0) name = line.slice(6).trim();
        else if (line.indexOf("data:") === 0) data.push(line.slice(5).replace(/^ /, ""));
      });
      if (!data.length && !name) return;
      var raw = data.join("\n");
      var parsed = raw;
      try { parsed = JSON.parse(raw); } catch (e) { /* plain text data */ }
      out.push({ event: name, data: parsed, raw: raw, t: ev.t_ms });
    });
    return out;
  }

  function parseArgs(value) {
    if (typeof value !== "string") return value;
    try { return JSON.parse(value); } catch (e) { return value; }
  }

  function textOf(content) {
    if (typeof content === "string") return content;
    if (Array.isArray(content)) return content.map(function (c) { return typeof c === "string" ? c : c.text || ""; }).join("\n");
    return content === undefined || content === null ? "" : JSON.stringify(content);
  }

  function kindOf(request) {
    var p = request.path || "";
    if (/\/messages$/.test(p)) return "anthropic";
    if (/\/chat\/completions$/.test(p)) return "openai-chat";
    if (/\/responses$/.test(p)) return "openai-responses";
    if (/\/embeddings$/.test(p)) return "embeddings";
    return null;
  }

  // Normalizes a request/response pair into { model, system, tools, input, output, usage, stop, streamed }.
  function understand(interaction) {
    var req = interaction.request;
    var res = interaction.response;
    var body = req.json || {};
    var kind = kindOf(req);
    var view = {
      kind: kind,
      model: body.model || null,
      system: null,
      tools: (body.tools || []).map(function (t) { return t.name || (t.function && t.function.name) || t.type; }).filter(Boolean),
      input: [],
      output: null,
      usage: null,
      stop: null,
      streamed: res.events && res.events.length > 0,
    };
    try {
      if (kind === "anthropic") anthropic(view, body, res);
      else if (kind === "openai-chat") openaiChat(view, body, res);
      else if (kind === "openai-responses") openaiResponses(view, body, res);
      else if (kind === "embeddings") embeddings(view, body, res);
    } catch (e) {
      view.kind = null; // unexpected shape: fall back to raw JSON
    }
    return view;
  }

  function anthropicParts(content) {
    if (typeof content === "string") return [{ kind: "text", text: content }];
    return (content || []).map(function (b) {
      switch (b.type) {
        case "text": return { kind: "text", text: b.text };
        case "tool_use": case "server_tool_use": return { kind: "call", name: b.name, args: b.input, id: b.id };
        case "tool_result": return { kind: "result", id: b.tool_use_id, text: textOf(b.content), error: b.is_error };
        case "thinking": return { kind: "thinking", text: b.thinking };
        case "redacted_thinking": return { kind: "thinking", text: "(redacted thinking)" };
        default: return { kind: "other", label: b.type, value: b };
      }
    });
  }

  function anthropic(view, body, res) {
    view.system = body.system ? textOf(body.system) : null;
    view.input = (body.messages || []).map(function (m) { return { role: m.role, parts: anthropicParts(m.content) }; });
    var msg = res.json;
    if (view.streamed) {
      var blocks = [];
      msg = { content: blocks, usage: {} };
      parseSse(res.events).forEach(function (e) {
        var d = e.data;
        if (!d || typeof d !== "object") return;
        if (d.type === "message_start") {
          msg.model = d.message.model;
          msg.usage.input_tokens = d.message.usage && d.message.usage.input_tokens;
        } else if (d.type === "content_block_start") {
          blocks[d.index] = Object.assign({}, d.content_block, { _json: "" });
        } else if (d.type === "content_block_delta" && blocks[d.index]) {
          var b = blocks[d.index];
          if (d.delta.type === "text_delta") b.text = (b.text || "") + d.delta.text;
          else if (d.delta.type === "input_json_delta") b._json += d.delta.partial_json;
          else if (d.delta.type === "thinking_delta") b.thinking = (b.thinking || "") + d.delta.thinking;
        } else if (d.type === "message_delta") {
          msg.stop_reason = d.delta && d.delta.stop_reason;
          if (d.usage) msg.usage.output_tokens = d.usage.output_tokens;
        }
      });
      blocks.forEach(function (b) {
        if (b && b._json) b.input = parseArgs(b._json);
      });
    }
    if (msg && msg.content) {
      view.output = { role: "assistant", parts: anthropicParts(msg.content.filter(Boolean)) };
      view.model = msg.model || view.model;
      view.stop = msg.stop_reason || null;
      if (msg.usage) view.usage = { input: msg.usage.input_tokens, output: msg.usage.output_tokens };
    }
  }

  function openaiParts(m) {
    var parts = [];
    if (m.role === "tool") return [{ kind: "result", id: m.tool_call_id, text: textOf(m.content) }];
    if (typeof m.content === "string" && m.content) parts.push({ kind: "text", text: m.content });
    else if (Array.isArray(m.content)) {
      m.content.forEach(function (c) {
        if (c.type === "text") parts.push({ kind: "text", text: c.text });
        else parts.push({ kind: "other", label: c.type, value: c });
      });
    }
    if (m.refusal) parts.push({ kind: "text", text: "Refusal: " + m.refusal });
    (m.tool_calls || []).forEach(function (c) {
      parts.push({ kind: "call", name: c.function && c.function.name, args: parseArgs(c.function && c.function.arguments), id: c.id });
    });
    return parts;
  }

  function openaiChat(view, body, res) {
    var system = [];
    (body.messages || []).forEach(function (m) {
      if (m.role === "system" || m.role === "developer") system.push(textOf(m.content));
      else view.input.push({ role: m.role, parts: openaiParts(m) });
    });
    view.system = system.length ? system.join("\n\n") : null;
    var out = res.json;
    if (view.streamed) {
      var message = { role: "assistant", content: "", tool_calls: [] };
      out = { choices: [{ message: message }] };
      parseSse(res.events).forEach(function (e) {
        var d = e.data;
        if (!d || typeof d !== "object") return;
        if (d.model) out.model = d.model;
        if (d.usage) out.usage = d.usage;
        var choice = d.choices && d.choices[0];
        if (!choice) return;
        if (choice.finish_reason) out.choices[0].finish_reason = choice.finish_reason;
        var delta = choice.delta || {};
        if (delta.content) message.content += delta.content;
        (delta.tool_calls || []).forEach(function (t) {
          var slot = message.tool_calls[t.index] || (message.tool_calls[t.index] = { id: t.id, function: { name: "", arguments: "" } });
          if (t.id) slot.id = t.id;
          if (t.function && t.function.name) slot.function.name += t.function.name;
          if (t.function && t.function.arguments) slot.function.arguments += t.function.arguments;
        });
      });
      message.tool_calls = message.tool_calls.filter(Boolean);
    }
    if (out && out.choices && out.choices[0]) {
      view.output = { role: "assistant", parts: openaiParts(out.choices[0].message || {}) };
      view.stop = out.choices[0].finish_reason || null;
      view.model = out.model || view.model;
      if (out.usage) view.usage = { input: out.usage.prompt_tokens, output: out.usage.completion_tokens };
    }
  }

  function responsesItems(items) {
    var messages = [];
    (items || []).forEach(function (item) {
      var type = item.type || (item.role ? "message" : null);
      if (type === "message") {
        var parts = typeof item.content === "string"
          ? [{ kind: "text", text: item.content }]
          : (item.content || []).map(function (c) {
              if (c.type === "input_text" || c.type === "output_text" || c.type === "text") return { kind: "text", text: c.text };
              if (c.type === "refusal") return { kind: "text", text: "Refusal: " + c.refusal };
              return { kind: "other", label: c.type, value: c };
            });
        messages.push({ role: item.role || "assistant", parts: parts });
      } else if (type === "function_call") {
        messages.push({ role: "assistant", parts: [{ kind: "call", name: item.name, args: parseArgs(item.arguments), id: item.call_id }] });
      } else if (type === "function_call_output") {
        messages.push({ role: "tool", parts: [{ kind: "result", id: item.call_id, text: textOf(item.output) }] });
      } else if (type === "reasoning") {
        var summary = (item.summary || []).map(function (s) { return s.text; }).join("\n");
        if (summary) messages.push({ role: "assistant", parts: [{ kind: "thinking", text: summary }] });
      } else if (type) {
        messages.push({ role: "assistant", parts: [{ kind: "other", label: type, value: item }] });
      }
    });
    return messages;
  }

  function openaiResponses(view, body, res) {
    view.system = body.instructions || null;
    view.input = typeof body.input === "string" ? [{ role: "user", parts: [{ kind: "text", text: body.input }] }] : responsesItems(body.input);
    var out = res.json;
    if (view.streamed) {
      var text = "";
      out = null;
      parseSse(res.events).forEach(function (e) {
        var d = e.data;
        if (!d || typeof d !== "object") return;
        if (/^response\.(completed|incomplete|failed)$/.test(d.type) && d.response) out = d.response;
        else if (d.type === "response.output_text.delta") text += d.delta;
      });
      if (!out) out = { output: [{ type: "message", role: "assistant", content: [{ type: "output_text", text: text }] }] };
    }
    if (out) {
      var parts = [];
      responsesItems(out.output).forEach(function (m) { parts = parts.concat(m.parts); });
      view.output = { role: "assistant", parts: parts };
      view.model = out.model || view.model;
      view.stop = out.status || null;
      if (out.usage) view.usage = { input: out.usage.input_tokens, output: out.usage.output_tokens };
    }
  }

  function embeddings(view, body, res) {
    var inputs = Array.isArray(body.input) ? body.input : [body.input];
    view.input = [{ role: "user", parts: inputs.map(function (t) { return { kind: "text", text: typeof t === "string" ? t : JSON.stringify(t) }; }) }];
    var data = (res.json && res.json.data) || [];
    var dims = data[0] && Array.isArray(data[0].embedding) ? data[0].embedding.length : "?";
    view.output = { role: "assistant", parts: [{ kind: "text", text: data.length + " embedding" + (data.length === 1 ? "" : "s") + " × " + dims + " dimensions" }] };
    if (res.json && res.json.usage) view.usage = { input: res.json.usage.prompt_tokens, output: 0 };
  }

  function previewOf(view) {
    if (!view.output) return "";
    for (var i = 0; i < view.output.parts.length; i++) {
      var p = view.output.parts[i];
      if (p.kind === "text" && p.text) return p.text.replace(/\s+/g, " ").slice(0, 160);
      if (p.kind === "call") return "→ " + p.name + "(…)";
    }
    return "";
  }

  // ---------- Routing ----------

  function route() {
    var hash = decodeURIComponent(location.hash.replace(/^#\/?/, ""));
    var live = hash.match(/^live(?:\/(\d+))?$/);
    if (live) return { view: "live", event: live[1] === undefined ? null : Number(live[1]) };
    var m = hash.match(/^s\/(.+?)(?:\/(\d+))?$/);
    if (m) return { view: "session", name: m[1], index: m[2] === undefined ? -1 : Number(m[2]) };
    return { view: "home" };
  }

  function go(hash) {
    if (location.hash !== hash) location.hash = hash;
    else onRoute();
  }

  function sessionHash(name, index) {
    return "#/s/" + encodeURIComponent(name).replace(/%2F/g, "/") + (index >= 0 ? "/" + index : "");
  }

  function onRoute() {
    var r = route();
    $("nav-live").setAttribute("aria-current", r.view === "live" ? "page" : "false");
    if (r.view === "live" && state.info && state.info.live) {
      state.session = null;
      state.eventId = r.event;
      renderSessions();
      renderLive();
      document.body.dataset.view = r.event === null ? "list" : "detail";
      return;
    }
    if (r.view === "session") {
      var load = state.session && state.session.name === r.name ? Promise.resolve() : loadSession(r.name);
      load.then(function () {
        if (!state.session) return;
        state.selected = r.index >= 0 && r.index < state.session.cassette.interactions.length ? r.index : state.session.cassette.interactions.length ? 0 : -1;
        renderSessions();
        renderList();
        renderDetail();
        document.body.dataset.view = r.index >= 0 ? "detail" : "list";
      });
      return;
    }
    // Home: live feed when attached to a proxy, otherwise the first cassette.
    if (state.info && state.info.live) return go("#/live");
    if (state.sessions.length) return go(sessionHash(state.sessions[0].name, -1));
    state.session = null;
    renderSessions();
    renderHomeEmpty();
    document.body.dataset.view = "list";
  }

  // ---------- Loading ----------

  function loadInfo() {
    return api("info").then(function (info) {
      state.info = info;
      $("dir").textContent = info.cassettes;
      $("dir").title = "Cassette directory: " + info.cassettes_path;
      var mode = $("mode");
      mode.hidden = false;
      if (info.live) {
        mode.textContent = "Live · " + info.mode;
        mode.className = "mode " + (info.mode === "record" ? "record" : "live");
        $("nav-live").hidden = false;
      } else {
        mode.textContent = "Browsing · v" + info.version;
        mode.className = "mode";
      }
      document.title = (info.live ? "Live · " : "") + "retake dashboard";
    });
  }

  function loadSessions() {
    return api("sessions").then(function (list) {
      state.sessions = list;
      renderSessions();
    });
  }

  function loadSession(name) {
    return api("session?name=" + encodeURIComponent(name)).then(
      function (cassette) {
        state.session = { name: name, cassette: cassette, views: cassette.interactions.map(understand) };
      },
      function (err) {
        state.session = null;
        renderSessions();
        renderError(err.message);
      }
    );
  }

  // ---------- Sidebar ----------

  function renderSessions() {
    var box = clear($("sessions"));
    var filter = state.filter.toLowerCase();
    var list = state.sessions.filter(function (s) { return s.name.toLowerCase().indexOf(filter) !== -1; });
    $("session-count").textContent = state.sessions.length ? String(state.sessions.length) : "";
    if (!list.length) {
      box.appendChild(h("p", { class: "folder", text: state.sessions.length ? "No matches" : "No cassettes yet" }));
      return;
    }
    var lastFolder = "";
    list.forEach(function (s) {
      var slash = s.name.lastIndexOf("/");
      var folder = slash === -1 ? "" : s.name.slice(0, slash);
      var leaf = slash === -1 ? s.name : s.name.slice(slash + 1);
      if (folder && folder !== lastFolder) box.appendChild(h("div", { class: "folder", text: folder + "/" }));
      lastFolder = folder;
      var current = state.session && state.session.name === s.name;
      box.appendChild(
        h("a", {
          class: "session" + (folder ? " nested" : ""),
          href: sessionHash(s.name, -1),
          role: "listitem",
          title: s.name,
          "aria-current": current ? "page" : null,
        },
          h("span", { class: "name", text: leaf }),
          s.error ? h("span", { class: "pill rec", text: "invalid" }) : h("span", { class: "pill", text: String(s.interactions) })
        )
      );
    });
  }

  function renderLiveCounts() {
    var box = clear($("live-counts"));
    var l = state.live;
    if (l.replayed) box.appendChild(h("span", { class: "pill ok", title: "Replayed", text: fmtNum(l.replayed) }));
    if (l.recorded) box.appendChild(h("span", { class: "pill rec", title: "Recorded", text: fmtNum(l.recorded) }));
    if (l.missed) box.appendChild(h("span", { class: "pill warn", title: "Unmatched", text: fmtNum(l.missed) }));
    document.querySelector("#nav-live .pulse").classList.toggle("off", !l.connected);
  }

  // ---------- Session list ----------

  function backButton(target, label, cls) {
    return h("button", {
      class: "btn back " + cls,
      type: "button",
      onclick: function () { document.body.dataset.view = target; },
    }, svg(ICONS.back), label);
  }

  function renderList() {
    var pane = clear($("list"));
    var s = state.session;
    var interactions = s.cassette.interactions;
    var info = state.sessions.filter(function (x) { return x.name === s.name; })[0];
    var tokens = s.views.reduce(function (sum, v) { return sum + (v.usage ? (v.usage.input || 0) + (v.usage.output || 0) : 0); }, 0);

    var actions = h("div", { class: "actions" }, backButton("sessions", "Cassettes", "to-sessions"));
    if (state.info.writable) {
      actions.appendChild(h("button", { class: "btn danger", type: "button", onclick: deleteSession }, svg(ICONS.trash), "Delete cassette"));
    }
    pane.appendChild(
      h("div", { class: "list-head" },
        h("h1", null, s.name),
        h("div", { class: "meta" },
          h("span", { text: interactions.length + " interaction" + (interactions.length === 1 ? "" : "s") }),
          tokens ? h("span", { text: fmtNum(tokens) + " tokens recorded" }) : null,
          info ? h("span", { text: fmtBytes(info.bytes) }) : null,
          info ? h("span", { text: "updated " + fmtAgo(info.modified) }) : null
        ),
        actions
      )
    );

    if (!interactions.length) {
      pane.appendChild(empty(ICONS.tape, "Empty cassette", "This session has no recorded interactions."));
      return;
    }
    var rows = h("div", { class: "rows", role: "listbox", "aria-label": "Interactions" });
    interactions.forEach(function (it, i) {
      var view = s.views[i];
      var sub = [statusPill(it.response.status)];
      if (view.model) sub.push(h("span", { text: view.model }));
      if (view.streamed) sub.push(h("span", { text: "streamed" }));
      if (view.usage && (view.usage.input || view.usage.output)) sub.push(h("span", { text: fmtNum((view.usage.input || 0) + (view.usage.output || 0)) + " tok" }));
      var preview = previewOf(view);
      rows.appendChild(
        h("button", {
          class: "row",
          type: "button",
          role: "option",
          "aria-selected": i === state.selected ? "true" : "false",
          onclick: function () { go(sessionHash(s.name, i)); },
        },
          h("span", { class: "num", text: "#" + (i + 1) }),
          h("span", { class: "title" }, h("span", { class: "method", text: it.request.method }), "/" + it.request.upstream + it.request.path),
          h("span"),
          h("span", { class: "sub" }, sub),
          preview ? h("span", { class: "preview", text: preview }) : null
        )
      );
    });
    pane.appendChild(rows);
    var selected = rows.querySelector('[aria-selected="true"]');
    if (selected) selected.scrollIntoView({ block: "nearest" });
  }

  // ---------- Detail ----------

  function renderDetail() {
    var pane = clear($("detail"));
    var s = state.session;
    if (!s || state.selected < 0) {
      pane.appendChild(empty(ICONS.click, "Select an interaction", "Pick a recorded call to see the conversation, raw request and response."));
      return;
    }
    var it = s.cassette.interactions[state.selected];
    var view = s.views[state.selected];
    var inner = h("div", { class: "detail-inner" });

    var head = h("div", { class: "detail-head" },
      backButton("list", "Interactions", "to-list"),
      h("span", { class: "pill", text: "#" + (state.selected + 1) }),
      h("h2", null, it.request.method + " /" + it.request.upstream + it.request.path + (it.request.query ? "?" + it.request.query : "")),
      h("span", { class: "spacer" })
    );
    if (state.info.writable) {
      head.appendChild(h("button", { class: "btn danger", type: "button", onclick: function () { deleteInteraction(state.selected); } }, svg(ICONS.trash), "Delete"));
    }
    inner.appendChild(head);

    var facts = [statusPill(it.response.status)];
    if (view.model) facts.push(h("span", { class: "pill violet", text: view.model }));
    if (view.streamed) facts.push(h("span", { class: "pill info", text: "streamed · " + it.response.events.length + " events" }));
    if (view.usage) {
      if (view.usage.input !== undefined) facts.push(h("span", { class: "pill", text: fmtNum(view.usage.input) + " in" }));
      if (view.usage.output) facts.push(h("span", { class: "pill", text: fmtNum(view.usage.output) + " out" }));
    }
    if (view.stop) facts.push(h("span", { class: "pill", text: "stop: " + view.stop }));
    if (it.response.duration_ms !== undefined) facts.push(h("span", { class: "pill", text: it.response.duration_ms + " ms" }));
    inner.appendChild(h("div", { class: "facts" }, facts));

    var tabs = [];
    if (view.kind) tabs.push(["conversation", "Conversation"]);
    tabs.push(["request", "Request"], ["response", "Response"]);
    if (view.streamed) tabs.push(["events", "Stream events"]);
    if (!tabs.some(function (t) { return t[0] === state.tab; })) state.tab = tabs[0][0];

    var tabBar = h("div", { class: "tabs", role: "tablist" });
    tabs.forEach(function (t) {
      tabBar.appendChild(h("button", {
        class: "tab",
        type: "button",
        role: "tab",
        "aria-selected": t[0] === state.tab ? "true" : "false",
        onclick: function () { state.tab = t[0]; renderDetail(); },
      }, t[1]));
    });
    inner.appendChild(tabBar);

    var body = h("div", { role: "tabpanel" });
    if (state.tab === "conversation") body.appendChild(conversation(view));
    else if (state.tab === "request") body.appendChild(payload(it.request, "Request body"));
    else if (state.tab === "response") body.appendChild(responseView(it.response));
    else body.appendChild(eventsView(it.response.events));
    inner.appendChild(body);
    pane.appendChild(inner);
    pane.scrollTop = 0;
  }

  function payload(part, label) {
    var box = h("div");
    box.appendChild(h("div", { class: "section-label", text: label }));
    if (part.json !== undefined) box.appendChild(jsonBlock(part.json));
    else if (part.text !== undefined) box.appendChild(textBlock(part.text));
    else if (part.base64 !== undefined) box.appendChild(textBlock("(binary, " + fmtBytes(Math.floor(part.base64.length * 0.75)) + " base64-encoded)"));
    else box.appendChild(h("p", { class: "muted", text: "Empty body" }));
    return box;
  }

  function responseView(res) {
    if (res.events && res.events.length) {
      var raw = res.events.map(function (e) { return e.data; }).join("");
      var box = h("div");
      box.appendChild(h("div", { class: "section-label", text: "Raw event stream (" + (res.content_type || "text/event-stream") + ")" }));
      box.appendChild(textBlock(raw));
      return box;
    }
    return payload(res, "Response body" + (res.content_type ? " (" + res.content_type + ")" : ""));
  }

  function eventsView(events) {
    var list = h("div", { class: "events" });
    var parsed = parseSse(events);
    parsed.forEach(function (e, i) {
      var name = e.event || (e.data && typeof e.data === "object" && e.data.type) || "data";
      var data = typeof e.data === "object" ? JSON.stringify(e.data) : e.raw;
      list.appendChild(
        h("div", { class: "event" },
          h("span", { class: "idx", text: (i + 1) + (e.t !== undefined ? "\n" + e.t + "ms" : "") }),
          h("div", null, h("span", { class: "name", text: name }), h("pre", { text: data }))
        )
      );
    });
    if (!parsed.length) list.appendChild(h("p", { class: "muted", text: "No events" }));
    return list;
  }

  function longText(text) {
    var el = h("p", { class: "text", text: text });
    if (text.length < 1400) return el;
    el.classList.add("long");
    var more = h("button", {
      class: "more",
      type: "button",
      text: "Show all",
      onclick: function () {
        var open = el.classList.toggle("open");
        more.textContent = open ? "Show less" : "Show all";
      },
    });
    return [el, more];
  }

  function renderPart(p) {
    if (p.kind === "text") return longText(p.text || "");
    if (p.kind === "thinking") {
      return h("div", { class: "part thinking" }, h("div", { class: "part-head" }, "thinking"), h("p", { class: "text", text: p.text || "" }));
    }
    if (p.kind === "call") {
      return h("div", { class: "part" },
        h("div", { class: "part-head" }, "→ tool call ", h("strong", { text: p.name || "?" }), p.id ? h("span", { class: "muted", text: p.id }) : null),
        typeof p.args === "string" ? textBlock(p.args) : jsonBlock(p.args === undefined ? {} : p.args)
      );
    }
    if (p.kind === "result") {
      var parsed = parseArgs(p.text);
      return h("div", { class: "part" + (p.error ? " error" : "") },
        h("div", { class: "part-head" }, p.error ? "← tool error" : "← tool result", p.id ? h("span", { class: "muted", text: p.id }) : null),
        typeof parsed === "object" && parsed !== null ? jsonBlock(parsed) : textBlock(p.text || "")
      );
    }
    return h("div", { class: "part" }, h("div", { class: "part-head", text: p.label || "content" }), jsonBlock(p.value));
  }

  function message(role, parts, extraClass, label) {
    var cls = role === "user" || role === "assistant" || role === "system" || role === "tool" ? role : "other";
    return h("div", { class: "msg " + cls + (extraClass ? " " + extraClass : "") },
      h("div", { class: "msg-head" }, h("span", { class: "dot" }), label || role),
      h("div", { class: "msg-body" }, parts.map(renderPart))
    );
  }

  function conversation(view) {
    var box = h("div", { class: "convo" });
    if (view.tools.length) {
      box.appendChild(h("div", { class: "tools" }, "Tools:", view.tools.map(function (t) { return h("span", { class: "pill", text: t }); })));
    }
    if (view.system) box.appendChild(message("system", [{ kind: "text", text: view.system }]));
    view.input.forEach(function (m) { box.appendChild(message(m.role, m.parts)); });
    if (view.output) {
      box.appendChild(h("div", { class: "divider", text: "Recorded response" }));
      box.appendChild(message("assistant", view.output.parts.length ? view.output.parts : [{ kind: "text", text: "(empty)" }], "response", "assistant · " + (view.model || "model")));
    }
    return box;
  }

  // ---------- Live ----------

  function renderLive() {
    var pane = clear($("list"));
    pane.appendChild(
      h("div", { class: "list-head" },
        h("h1", null, "Live activity"),
        h("div", { class: "meta" },
          h("span", { text: state.live.connected ? "Connected · " + state.info.mode + " mode" : "Disconnected — the run may have ended" }),
          h("span", { text: state.events.length + " request" + (state.events.length === 1 ? "" : "s") })
        ),
        h("div", { class: "actions" }, backButton("sessions", "Cassettes", "to-sessions"))
      )
    );
    if (!state.events.length) {
      pane.appendChild(empty(ICONS.pulse, "Waiting for requests", "Calls through this proxy appear here as they happen: replayed, recorded, or unmatched."));
    } else {
      var rows = h("div", { class: "rows", role: "listbox", "aria-label": "Live requests" });
      state.events.slice().reverse().forEach(function (e) {
        var kindPill = eventPill(e.kind);
        rows.appendChild(
          h("button", {
            class: "row event-row",
            type: "button",
            role: "option",
            "aria-selected": e.id === state.eventId ? "true" : "false",
            onclick: function () { selectEvent(e); },
          },
            h("span", { class: "num", text: "#" + e.id }),
            h("span", { class: "title" }, h("span", { class: "method", text: e.method }), e.path),
            h("span", { class: "time", text: fmtTime(e.at) }),
            h("span", { class: "sub" }, kindPill, e.status ? statusPill(e.status) : null, h("span", { text: e.session + (e.index !== undefined ? " · step " + (e.index + 1) : "") }))
          )
        );
      });
      pane.appendChild(rows);
    }
    renderEventDetail();
  }

  function eventPill(kind) {
    var styles = {
      replayed: ["ok", "replayed"],
      recorded: ["rec", "recorded"],
      missed: ["warn", "unmatched"],
      passed: ["", "not recorded"],
      failed: ["rec", "failed"],
    };
    var style = styles[kind] || ["", kind];
    return h("span", { class: "pill " + style[0], text: style[1] });
  }

  function selectEvent(e) {
    if ((e.kind === "replayed" || e.kind === "recorded") && e.index !== undefined) {
      // Show the real interaction from the cassette, in the session view.
      return go(sessionHash(e.session, e.index));
    }
    go("#/live/" + e.id);
  }

  function renderEventDetail() {
    var pane = clear($("detail"));
    var e = state.events.filter(function (x) { return x.id === state.eventId; })[0];
    if (!e) {
      pane.appendChild(empty(ICONS.click, "Select a request", "Replayed and recorded calls open in their cassette. Unmatched calls show what changed."));
      return;
    }
    var inner = h("div", { class: "detail-inner" },
      h("div", { class: "detail-head" }, backButton("list", "Live activity", "to-list"), h("h2", null, e.method + " " + e.path)),
      h("div", { class: "facts" },
        eventPill(e.kind),
        e.status ? statusPill(e.status) : null,
        h("span", { class: "pill", text: "session " + e.session }),
        h("span", { class: "pill", text: fmtTime(e.at) })
      )
    );
    if (e.kind === "missed") {
      inner.appendChild(h("div", { class: "section-label", text: "No recording matched. Diff against the closest recording:" }));
      var detail = e.detail || "";
      var nl = detail.indexOf("\n");
      inner.appendChild(h("p", { class: "muted", text: nl === -1 ? detail : detail.slice(0, nl) }));
      if (nl !== -1) inner.appendChild(diffBlock(detail.slice(nl + 1)));
      inner.appendChild(h("p", { class: "muted", text: "Re-record with `retake record`, or run `retake auto` to record only what's missing." }));
    } else if ((e.kind === "replayed" || e.kind === "recorded") && e.index !== undefined) {
      inner.appendChild(h("p", { class: "muted", text: (e.kind === "replayed" ? "Served from" : "Saved to") + " step " + (e.index + 1) + " of " + e.session + "." }));
      inner.appendChild(h("a", { class: "btn", href: sessionHash(e.session, e.index) }, "Open in cassette"));
    } else if (e.detail) {
      inner.appendChild(textBlock(e.detail));
    }
    pane.appendChild(inner);
  }

  var refreshTimer;
  function scheduleRefresh(event) {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(function () {
      loadSessions();
      if (event.kind === "recorded" && state.session && state.session.name === event.session) {
        var name = state.session.name;
        loadSession(name).then(function () {
          if (route().view === "session") { renderList(); renderDetail(); }
        });
      }
    }, 250);
  }

  function connectLive() {
    var source = new EventSource(API + "events");
    source.onopen = function () {
      // The server resends its history on every connection, and a restarted
      // proxy numbers events from 1 again, so start from a clean slate.
      state.events = [];
      state.live = { connected: true, replayed: 0, recorded: 0, missed: 0 };
      $("mode").textContent = "Live · " + state.info.mode;
      $("mode").className = "mode " + (state.info.mode === "record" ? "record" : "live");
      renderLiveCounts();
      if (route().view === "live") renderLive();
    };
    source.onerror = function () {
      state.live.connected = false;
      renderLiveCounts();
      $("mode").textContent = "Stopped · " + state.info.mode;
      $("mode").className = "mode offline";
      if (route().view === "live") renderLive();
    };
    source.onmessage = function (msg) {
      var event;
      try { event = JSON.parse(msg.data); } catch (e) { return; }
      if (state.events.some(function (x) { return x.id === event.id; })) return; // defensive: never show an event twice
      $("mode").textContent = "Live · " + state.info.mode;
      $("mode").className = "mode " + (state.info.mode === "record" ? "record" : "live");
      state.events.push(event);
      if (event.kind === "replayed") state.live.replayed++;
      else if (event.kind === "recorded") state.live.recorded++;
      else if (event.kind === "missed") state.live.missed++;
      renderLiveCounts();
      if (route().view === "live") renderLive();
      scheduleRefresh(event);
    };
  }

  // ---------- Empty and error states ----------

  function empty(icon, title, text, extra) {
    return h("div", { class: "empty" }, h("div", { class: "empty-icon" }, svg(icon)), h("h2", { text: title }), h("p", { text: text }), extra || null);
  }

  function renderHomeEmpty() {
    var list = clear($("list"));
    clear($("detail"));
    list.appendChild(
      empty(ICONS.tape, "No cassettes yet", "Record your first run and the cassettes show up here.",
        textBlock("retake record -- npm test\nretake ui")
      )
    );
    $("detail").appendChild(empty(ICONS.click, "Nothing selected", "Recorded interactions will appear in the list."));
  }

  function renderError(message) {
    var list = clear($("list"));
    clear($("detail"));
    list.appendChild(empty(ICONS.tape, "Couldn't open cassette", message));
  }

  // ---------- Changes ----------

  function confirmDialog(title, text, okLabel) {
    var dialog = $("confirm");
    $("confirm-title").textContent = title;
    $("confirm-text").textContent = text;
    $("confirm-ok").textContent = okLabel;
    return new Promise(function (resolve) {
      function done(result) {
        $("confirm-ok").removeEventListener("click", ok);
        $("confirm-cancel").removeEventListener("click", cancel);
        dialog.removeEventListener("close", cancel);
        if (dialog.open) dialog.close();
        resolve(result);
      }
      function ok() { done(true); }
      function cancel() { done(false); }
      $("confirm-ok").addEventListener("click", ok);
      $("confirm-cancel").addEventListener("click", cancel);
      dialog.addEventListener("close", cancel);
      dialog.showModal();
      $("confirm-cancel").focus();
    });
  }

  function deleteInteraction(index) {
    var name = state.session.name;
    confirmDialog("Delete interaction #" + (index + 1) + "?", "It will be removed from " + name + ".json. Tests that need it will fail in replay until you record again.", "Delete").then(function (ok) {
      if (!ok) return;
      api("interaction?session=" + encodeURIComponent(name) + "&index=" + index, { method: "DELETE" }).then(function (cassette) {
        state.session = { name: name, cassette: cassette, views: cassette.interactions.map(understand) };
        toast("Interaction deleted");
        loadSessions();
        go(sessionHash(name, Math.min(index, cassette.interactions.length - 1)));
      }, function (err) { toast(err.message, true); });
    });
  }

  function deleteSession() {
    var name = state.session.name;
    confirmDialog("Delete cassette?", name + ".json and all its recordings will be deleted from disk. This can't be undone.", "Delete cassette").then(function (ok) {
      if (!ok) return;
      api("session?name=" + encodeURIComponent(name), { method: "DELETE" }).then(function () {
        toast("Cassette deleted");
        state.session = null;
        loadSessions().then(function () { go("#/"); });
      }, function (err) { toast(err.message, true); });
    });
  }

  // ---------- Theme and keyboard ----------

  function initTheme() {
    var stored = null;
    try { stored = localStorage.getItem("retake-ui-theme"); } catch (e) { /* storage unavailable */ }
    var light = stored ? stored === "light" : window.matchMedia("(prefers-color-scheme: light)").matches;
    if (light) document.documentElement.setAttribute("data-theme", "light");
    $("theme").addEventListener("click", function () {
      var nowLight = document.documentElement.getAttribute("data-theme") !== "light";
      if (nowLight) document.documentElement.setAttribute("data-theme", "light");
      else document.documentElement.removeAttribute("data-theme");
      try { localStorage.setItem("retake-ui-theme", nowLight ? "light" : "dark"); } catch (e) { /* ignore */ }
    });
  }

  function initKeyboard() {
    $("search").addEventListener("input", function (e) {
      state.filter = e.target.value;
      renderSessions();
    });
    document.addEventListener("keydown", function (e) {
      var typing = /^(INPUT|TEXTAREA)$/.test(document.activeElement.tagName);
      if (e.key === "/" && !typing) {
        e.preventDefault();
        $("search").focus();
        return;
      }
      if (e.key === "Escape" && document.activeElement === $("search")) {
        $("search").value = "";
        state.filter = "";
        renderSessions();
        $("search").blur();
        return;
      }
      if (typing || $("confirm").open) return;
      var r = route();
      if (r.view !== "session" || !state.session) return;
      var count = state.session.cassette.interactions.length;
      if ((e.key === "ArrowDown" || e.key === "j") && state.selected < count - 1) {
        e.preventDefault();
        go(sessionHash(state.session.name, state.selected + 1));
      } else if ((e.key === "ArrowUp" || e.key === "k") && state.selected > 0) {
        e.preventDefault();
        go(sessionHash(state.session.name, state.selected - 1));
      }
    });
  }

  // ---------- Start ----------

  initTheme();
  initKeyboard();
  window.addEventListener("hashchange", onRoute);
  Promise.all([loadInfo(), loadSessions()]).then(function () {
    if (state.info.live) connectLive();
    onRoute();
  }, function (err) {
    renderError("Couldn't reach retake: " + err.message);
  });
})();
