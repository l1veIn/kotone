"""MCP plan phase 2 (2026-11-25): third-party MCP servers as plugins, on the real Kotone, with the official example
server (@modelcontextprotocol/server-everything) run locally twice — once over stdio, once over Streamable HTTP.

No AI key, no OS input, nothing typed, no sound. The real desktop app (debug build) runs `--background` in an isolated
KOTONE_HOME=target/mcp-servers-real/home; host commands go through WebView2's DevTools port
(`window.__TAURI_INTERNALS__.invoke` in the settings page); her side runs against a local mock model in this script.

0. (review fixes, 2026-11-25) mcp.json starts with a hand-copied `lol` entry — the same name as the built-in
   英雄联盟 plugin: it is NOT a plugin (`mcp_servers_status` lists it with the reason, the plugins page shows the row),
   the built-in 英雄联盟 stays one built-in plugin (its App resource reads the built-in way, external agents are not
   refused as for a third-party tool), and removing that entry only drops it from mcp.json (the built-in's switch
   in config.json stays);
1. the HTTP server: `npx -y @modelcontextprotocol/server-everything streamableHttp` on a free port (PORT); this also
   puts the package into the npx cache so the stdio start below is quick (the connect timeout is 10 s);
2. `mcp_servers_preview` of a Claude-Desktop-style paste (`{"mcpServers": {"everything": {"command": "npx", ...}}}`):
   suggested id, trust-prompt facts (program, package) → `mcp_server_add`; it connects by itself (on, tools unknown);
   the card has its tools named `everything.<tool>`, the server's own title and icon, trusted; her tool list
   (`agent_tools`) has them in the main session (<= 20 third-party tools) and the non-read-only ones ask
   (“第三方插件的操作”);
3. her path: in a new conversation the mock model calls `everything.echo`; the conversation has the result and the
   follow-up request to the model carries it inside the untrusted-data block ("含不可信内容", the ADR 030 marker —
   in the main session too). Permissions: the isolated config sets “always allow” (a rule `allow`) for exactly the
   two echo tools, which is what the player gets by choosing “always allow this tool”; the asking rules themselves
   are unit tests (kotone-remote-plugin);
4. the HTTP one, pasted as a bare `{"url": ...}`, added as `everything-http`, connects; the cap: with both, more
   than 20 third-party tools → none of them in the main session (only the background, found with `tools.find`);
5. external agents never see them: `kotone.exe tools call system.capabilities` (what `kotone.exe mcp` lists) has
   none of their tools, and `kotone.exe tools call everything.echo` is refused;
6. switch `everything` off → its card says notConnected; she calls `everything-http.echo` the same way;
7. the page path (`plugin_call` as its own page) works; `mcp_server_remove` → gone from mcp.json and the list,
   and its “always allow” rule is gone too; then `everything`'s config is changed by hand (an extra env var) and
   reloaded → its “always allow” rule is cleared and it is no longer trusted (both follow the config);
8. switched back on, it connects with the new config; its processes go when Kotone quits (Job Object).

Stops before starting if a game is running or in front. Takes target/tmp/real.lock. Prerequisites (README 准备):
`pnpm -C apps/desktop build:web`, then `cargo build -p kotone-tauri --features tauri/custom-protocol`; the exe is
copied to target/mcp-servers-real/bin first. Needs node/npx and the network the first time (npm download).
usage: python -X utf8 -I scripts/manual-e2e/mcp_servers_real.py [--keep]
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "mcp-servers-real")
HOME = os.path.join(WORK, "home")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
SRC_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
BIN = os.path.join(WORK, "bin")
EXE = os.path.join(BIN, "kotone.exe")
DEVTOOLS = 9357
PACKAGE = "@modelcontextprotocol/server-everything"
RESULTS = []
MOCK_SEEN = []  # every chat request the mock model got (messages only)


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def guard():
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, encoding="mbcs",
                         errors="replace").stdout.lower()
    running = [g for g in ("league of legends.exe", "leagueclient", "riotclient", "valorant", "cs2.exe",
                           "dota2.exe", "yuanshen", "genshinimpact") if g in out]
    if running:
        raise SystemExit(f"STOP: a game is running ({running}); not testing while the player plays")
    path = foreground_process()
    if any(g in os.path.basename(path).lower() for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={DEVTOOLS}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)


def tools_call(tool, args=None, timeout_ms=30_000):
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", "-", "--timeout-ms", str(timeout_ms)],
                         input=json.dumps(args if args is not None else {}, ensure_ascii=False),
                         capture_output=True, text=True, encoding="utf-8", env=ENV)
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return {"ok": False, "error": {"code": "NO_JSON", "message": (out.stdout + out.stderr)[-400:]}}


PAGE_EVAL = r"""
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:__PORT__");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
  const r = page ? await page.evaluate(async () => {
    try { return { ok: await (async () => { __BODY__ })() }; } catch (e) { return { error: String(e) }; }
  }) : { error: "settings page not found" };
  console.log(JSON.stringify(r));
} finally { await browser.close(); }
"""


def invoke(command, args=None):
    body = "return await window.__TAURI_INTERNALS__.invoke(%s, %s);" % (
        json.dumps(command), json.dumps(args or {}, ensure_ascii=False))
    return page_js(body)


def page_js(body):
    """Run `body` (an async function body) in the settings page; DOM clicks only, no OS input."""
    code = PAGE_EVAL.replace("__PORT__", str(DEVTOOLS)).replace("__BODY__", body)
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        return {"error": (out.stdout + out.stderr)[-800:]}


def wait_until(predicate, seconds, step=1.0):
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            value = predicate()
        except Exception:  # noqa: BLE001
            value = None
        if value:
            return value
        time.sleep(step)
    return None


# ---------- mock model: "（E2E）调用 <tool>" → that tool with {"message": ...} → "好了" ----------

def text_of(message):
    content = message.get("content")
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(part.get("text", "") for part in content if isinstance(part, dict))
    return ""


def chunk(delta, finish_reason=None):
    return {"id": "x", "object": "chat.completion.chunk", "created": 0, "model": "mock",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]}


def mock_plan(body):
    messages = body.get("messages", [])
    MOCK_SEEN.append(messages)
    last_user = max((i for i, m in enumerate(messages) if m.get("role") == "user"), default=-1)
    user = text_of(messages[last_user]) if last_user >= 0 else ""
    done = sum(1 for m in messages[last_user + 1:] if m.get("role") == "tool")
    names = [(t.get("function") or {}).get("name", "") for t in body.get("tools") or []]
    if "（E2E）调用 " in user and done == 0:
        target = user.split("（E2E）调用 ", 1)[1].split()[0]
        wire = next((n for n in names if n == target.replace(".", "_")), None)
        if wire:
            args = json.dumps({"message": f"hello from kotone via {target}"}, ensure_ascii=False)
            return [chunk({"role": "assistant", "tool_calls": [{"index": 0, "id": "call_0", "type": "function",
                                                               "function": {"name": wire, "arguments": args}}]}),
                    chunk({}, "tool_calls")]
    return [chunk({"role": "assistant", "content": "好了"}, "stop")]


class MockModel(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, data, content_type):
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.reply(200, json.dumps({"data": [{"id": "mock"}]}).encode(), "application/json")

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", "0") or 0))
        if not self.path.endswith("/chat/completions"):
            self.reply(404, b'{"error":{"message":"not in this mock"}}', "application/json")
            return
        events = mock_plan(json.loads(raw or b"{}"))
        payload = "".join(f"data: {json.dumps(e, ensure_ascii=False)}\n\n" for e in events) + "data: [DONE]\n\n"
        self.reply(200, payload.encode(), "text/event-stream")


def write_config():
    config = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "legacyUpgrade": "notNeeded"},
        "adminPromptDismissed": True,
        "toolService": {"enabled": True},
        "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False}},
        # “总是允许这个工具”（一条 allow 规则）只给这两个；别的照默认问
        "agent": {"permissions": {"mode": "default", "rules": {"everything.echo": "allow",
                                                              "everything-http.echo": "allow"}},
                  "progressSpeech": False},
        # 内置英雄联盟的开关写明（删掉 mcp.json 里同名的那一项以后还在）
        "plugins": {"lol": {"enabled": True}},
    }
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, ensure_ascii=False, indent=1)


def sweep_credentials():
    listing = subprocess.run(["cmdkey", "/list"], capture_output=True, text=True, encoding="mbcs",
                             errors="replace").stdout
    for line in listing.splitlines():
        if "target=" in line and WORK in line:
            subprocess.run(["cmdkey", f"/delete:{line.split('target=', 1)[1].strip()}"], capture_output=True)


def config_file():
    with open(os.path.join(HOME, "config.json"), encoding="utf-8") as f:
        return json.load(f)


def read_mcp_file():
    with open(os.path.join(HOME, "mcp.json"), encoding="utf-8") as f:
        return json.load(f)


def rules():
    return ((config_file().get("agent") or {}).get("permissions") or {}).get("rules") or {}


PROBLEM_ROW = r"""
document.querySelector('[data-testid="settings-nav-plugins"]')?.click();
for (let i = 0; i < 60; i++) {
  const row = document.querySelector('[data-testid="plugins-mcp-problem"][data-id="lol"]');
  if (row) return { kind: row.dataset.kind, text: row.textContent,
                    cards: document.querySelectorAll('[data-testid="plugins-card-lol"]').length,
                    thirdParty: document.querySelectorAll('[data-testid="plugins-card-lol-third-party"]').length };
  await new Promise((r) => setTimeout(r, 250));
}
return null;
"""


def builtin_lol_checks():
    """0. mcp.json's `lol` entry is a problem, not a plugin; the built-in 英雄联盟 is untouched."""
    status = invoke("mcp_servers_status").get("ok") or {}
    problem = next((p for p in status.get("problems") or [] if p.get("id") == "lol"), None)
    check("mcp.json `lol`: not a plugin, listed with the reason",
          problem and problem.get("kind") == "taken" and "撞了" in problem.get("error", ""),
          json.dumps(status, ensure_ascii=False)[:300])
    plugins = invoke("plugins_list").get("ok") or []
    lols = [p for p in plugins if p.get("manifest", {}).get("id") == "lol"]
    check("built-in 英雄联盟 is still one built-in plugin (no `remote`)",
          len(lols) == 1 and "remote" not in lols[0] and lols[0].get("enabled") is True,
          json.dumps([{k: p.get(k) for k in ("enabled",)} | {"remote": "remote" in p} for p in lols]))
    resource = invoke("plugin_resource", {"plugin": "lol", "uri": "ui://kotone/lol/tier"})
    check("built-in 英雄联盟's App resource reads the built-in way",
          "mcp-app" in ((resource.get("ok") or {}).get("mimeType") or ""), json.dumps(resource, ensure_ascii=False)[:200])
    external = tools_call("lol.scout", {}, timeout_ms=20_000)
    check("external agents are not refused as for a third-party tool",
          "第三方插件" not in json.dumps(external, ensure_ascii=False), json.dumps(external, ensure_ascii=False)[:200])
    row = page_js(PROBLEM_ROW)
    shown = row.get("ok") or {}
    check("plugins page: the `lol` row says why; the 英雄联盟 card is the built-in one",
          shown.get("kind") == "taken" and shown.get("cards") == 1 and shown.get("thirdParty") == 0,
          json.dumps(row, ensure_ascii=False)[:300])
    removed = invoke("mcp_server_remove", {"id": "lol"})
    status = invoke("mcp_servers_status").get("ok") or {}
    lol_config = (config_file().get("plugins") or {}).get("lol") or {}
    check("removing that entry only drops it from mcp.json (the built-in's switch stays)",
          "error" not in removed and "lol" not in read_mcp_file().get("mcpServers", {})
          and not status.get("problems") and lol_config.get("enabled") is True,
          json.dumps({"removed": removed, "lol": lol_config}, ensure_ascii=False)[:300])


def remote_of(plugins, pid):
    return next((p for p in plugins if p.get("manifest", {}).get("id") == pid), None)


def her_call(tool):
    """She calls `tool` (mock model); returns (tool message in the conversation, follow-up request to the model)."""
    seen_before = len(MOCK_SEEN)
    sent = invoke("agent_send", {"text": f"（E2E）调用 {tool} 看看"})
    cid = (sent.get("ok") or {}).get("conversationId")

    def tool_message():
        conv = (invoke("agent_conversation", {"id": cid}).get("ok") or {}) if cid else {}
        return next((m for m in conv.get("messages", []) if m.get("role") == "tool" and m.get("name") == tool), None)
    message = wait_until(tool_message, 60) if cid else None
    time.sleep(1.5)
    followup = None
    for messages in MOCK_SEEN[seen_before:]:
        tool_msgs = [m for m in messages if m.get("role") == "tool"]
        if tool_msgs:
            followup = text_of(tool_msgs[-1])
    return sent, message, followup


def everything_pids():
    """Node processes running server-everything over stdio (not the HTTP one this script started)."""
    out = subprocess.run(["powershell", "-NoProfile", "-Command",
                          "Get-CimInstance Win32_Process -Filter \"Name='node.exe'\" | "
                          "ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }"],
                         capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
    return [int(line.split()[0]) for line in out.splitlines()
            if "server-everything" in line and "streamableHttp" not in line and line.split()[0].isdigit()]


def connected(ids):
    """Cards of `ids` once all are connected with tools (a failed one gets one reconnect per poll)."""
    plugins = invoke("plugins_list").get("ok") or []
    cards = [remote_of(plugins, i) for i in ids]
    if all(c and (c.get("remote") or {}).get("status") == "connected" and c.get("tools") for c in cards):
        return cards
    for i, c in zip(ids, cards):
        if c and (c.get("remote") or {}).get("status") == "failed":
            print(f"     {i}: {(c.get('remote') or {}).get('error')} — reconnecting", flush=True)
            invoke("mcp_server_reconnect", {"id": i})
    return None


def card_checks(card):
    pid = card["manifest"]["id"]
    names = [t["name"] for t in card["tools"]]
    remote = card["remote"]
    check(f"{pid}: tools are named {pid}.<tool> (echo there)",
          all(n.startswith(pid + ".") for n in names) and f"{pid}.echo" in names, f"{len(names)} tools")
    check(f"{pid}: the card has the server's own name and icon, trusted (added through the trust prompt)",
          (remote.get("server") or {}).get("title") and (remote.get("server") or {}).get("icons")
          and remote.get("trusted") is True, json.dumps(remote.get("server"), ensure_ascii=False)[:160])
    print(f"     {pid}: {len(names)} tools, pages={remote.get('pages')}", flush=True)


def model_tools(pid):
    card = remote_of(invoke("plugins_list").get("ok") or [], pid) or {}
    return sum(1 for t in card.get("tools", []) if "model" in t.get("visibility", []))


def tools_of_her():
    return (invoke("agent_tools").get("ok") or {}).get("tools") or []


def her_path(tool):
    invoke("agent_new_conversation")
    sent, message, followup = her_call(tool)
    check(f"she called {tool} (mock model)", message and "hello from kotone" in (message.get("content") or ""),
          json.dumps(message or sent, ensure_ascii=False)[:300])
    check(f"{tool}: the model got the result as untrusted data",
          followup and "含不可信内容" in followup and "hello from kotone" in followup, (followup or "")[:200])


def main():
    keep = "--keep" in sys.argv
    guard()
    os.makedirs(os.path.dirname(LOCK), exist_ok=True)
    try:
        os.mkdir(LOCK)
    except FileExistsError:
        raise SystemExit(f"{LOCK} exists: another real-machine test is running (remove it if stale)")
    http_server = app = mock = None
    server_pids = []
    try:
        if not os.path.exists(SRC_EXE):
            raise SystemExit("no target/debug/kotone.exe — build it first (see the docstring)")
        tools_call("app.quit", timeout_ms=3000) if os.path.exists(EXE) else None
        time.sleep(1)
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        shutil.copy2(SRC_EXE, EXE)
        write_config()
        # 0. 手抄来的配置，键名正好是内置插件的 id
        with open(os.path.join(HOME, "mcp.json"), "w", encoding="utf-8") as f:
            json.dump({"mcpServers": {"lol": {"command": "npx", "args": ["-y", "some-lol-mcp"]}}}, f)

        # 1. the HTTP server (also warms the npx cache for the stdio one)
        port = free_port()
        npx = shutil.which("npx") or "npx"
        http_server = subprocess.Popen([npx, "-y", PACKAGE, "streamableHttp"], env=dict(os.environ, PORT=str(port)),
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                       encoding="utf-8", errors="replace",
                                       creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
        listening = wait_until(lambda: socket.create_connection(("127.0.0.1", port), 1).close() or True, 180, 2)
        if not check("HTTP everything server is up (npx)", listening, f"port {port}"):
            return

        mock_port = free_port()
        mock = ThreadingHTTPServer(("127.0.0.1", mock_port), MockModel)
        threading.Thread(target=mock.serve_forever, daemon=True).start()

        app = subprocess.Popen([EXE, "--background"], env=ENV)
        if not wait_until(lambda: tools_call("app.status", timeout_ms=3000).get("ok"), 90):
            raise SystemExit("the isolated instance's tool service did not come up")
        if not wait_until(lambda: "ok" in invoke("agent_state"), 90):
            raise SystemExit("the settings page did not come up (DevTools)")
        up = tools_call("connections.upsert", {"connection": {
            "id": "mock", "displayName": "mock", "kind": "remote", "provider": "custom",
            "baseUrl": f"http://127.0.0.1:{mock_port}/v1", "model": "mock"}, "apiKey": "sk-local-fixture"})
        tools_call("settings.update", {"patch": {"assistant": {"connectionId": "mock"}}})
        check("mock model connection", up.get("ok"), json.dumps(up, ensure_ascii=False)[:200])
        builtin_lol_checks()

        # 2. preview + add the stdio one (pasted the Claude Desktop way); it connects (on, tools unknown)
        paste = json.dumps({"mcpServers": {"everything": {"command": "npx", "args": ["-y", PACKAGE]}}})
        preview = invoke("mcp_servers_preview", {"text": paste}).get("ok") or {}
        cand = (preview.get("servers") or [{}])[0]
        facts = cand.get("facts") or {}
        check("preview: suggested id and trust-prompt facts",
              cand.get("id") == "everything" and facts.get("kind") == "stdio" and facts.get("package") == PACKAGE
              and facts.get("command") == "npx", json.dumps(cand, ensure_ascii=False)[:300])
        added = invoke("mcp_server_add", {"id": cand.get("id"), "entry": cand.get("entry")})
        check("add stdio server", added.get("ok"), json.dumps(added, ensure_ascii=False)[:300])
        stdio = wait_until(lambda: connected(["everything"]), 120, 2)
        if not check("stdio server connected and listed its tools", stdio,
                     json.dumps(invoke("plugins_list").get("ok"), ensure_ascii=False)[-600:]):
            return
        card_checks(stdio[0])
        n_stdio = model_tools("everything")
        her_tools = tools_of_her()
        check("her list: its tools are in the main session (<= 20 third-party tools), non-read-only ones ask",
              all(t["core"] for t in her_tools if t["name"].startswith("everything."))
              and any(t["defaultRule"] == "ask" and "third-party-action" in t["sideEffects"]
                      for t in her_tools if t["name"].startswith("everything.")),
              f"{n_stdio} tools for her")

        # 3. her path (main session, mock model), the result marked untrusted
        her_path("everything.echo")

        # 4. the HTTP one (pasted as a bare server)
        preview = invoke("mcp_servers_preview", {"text": json.dumps({"url": f"http://127.0.0.1:{port}/mcp"})})
        cand = ((preview.get("ok") or {}).get("servers") or [{}])[0]
        check("preview: bare HTTP server", (cand.get("facts") or {}).get("kind") == "http",
              json.dumps(cand, ensure_ascii=False)[:300])
        added = invoke("mcp_server_add", {"id": "everything-http", "entry": cand.get("entry")})
        check("add HTTP server", added.get("ok"), json.dumps(added, ensure_ascii=False)[:300])
        with open(os.path.join(HOME, "mcp.json"), encoding="utf-8") as f:
            mcp_file = json.load(f)
        check("mcp.json has both, in the mcpServers format",
              set(mcp_file.get("mcpServers", {})) == {"everything", "everything-http"}
              and mcp_file["mcpServers"]["everything-http"].get("type") == "http", json.dumps(mcp_file)[:300])
        both = wait_until(lambda: connected(["everything", "everything-http"]), 120, 2)
        if not check("HTTP server connected too", both,
                     json.dumps(invoke("plugins_list").get("ok"), ensure_ascii=False)[-600:]):
            return
        card_checks(both[1])
        total = n_stdio + model_tools("everything-http")
        remote_main = [t["core"] for t in tools_of_her() if t["name"].startswith("everything")]
        if total > 20:
            check(f"cap: {total} third-party tools > 20 → all only for the background", remote_main
                  and not any(remote_main))
        else:
            check(f"cap: {total} third-party tools <= 20 → still in the main session", all(remote_main))

        # 5. not for external agents
        caps = tools_call("system.capabilities")
        listed = [t.get("name", "") for t in ((caps.get("result") or {}).get("tools") or [])]
        check("tool service / kotone.exe mcp: no third-party tools",
              caps.get("ok") and listed and not any(n.startswith("everything") for n in listed), f"{len(listed)} tools")
        refused = tools_call("everything.echo", {"message": "x"})
        check("external call of a third-party tool is refused", not refused.get("ok"),
              json.dumps(refused.get("error"), ensure_ascii=False)[:200])

        # 6. switch the stdio one off → disconnected, the HTTP one alone fits the main session again
        invoke("update_settings", {"patch": {"plugins": {"everything": {"enabled": False}}}})
        off = wait_until(lambda: (lambda c: c and c["remote"]["status"] == "notConnected" and not c["enabled"])(
            remote_of(invoke("plugins_list").get("ok") or [], "everything")), 30)
        check("switching it off disconnects", off)
        her_path("everything-http.echo")

        # 7. the page path (its own page calls it directly), then remove
        direct = invoke("plugin_call", {"plugin": "everything-http", "tool": "everything-http.echo",
                                        "arguments": {"message": "direct"}})
        check("plugin_call (its own page) works", "direct" in json.dumps(direct.get("ok"), ensure_ascii=False),
              json.dumps(direct, ensure_ascii=False)[:200])
        removed = invoke("mcp_server_remove", {"id": "everything-http"})
        with open(os.path.join(HOME, "mcp.json"), encoding="utf-8") as f:
            left = json.load(f).get("mcpServers", {})
        check("remove: gone from mcp.json and the list",
              "error" not in removed and "everything-http" not in left
              and not remote_of(invoke("plugins_list").get("ok") or [], "everything-http"),
              json.dumps(removed, ensure_ascii=False)[:200])
        check("remove: its “always allow” rule is gone too", "everything-http.echo" not in rules(),
              json.dumps(rules(), ensure_ascii=False))

        # 7b. everything's config is changed by hand (still off): its “always allow” and the trust don't carry over
        check("before the change: everything.echo is always allowed", rules().get("everything.echo") == "allow",
              json.dumps(rules(), ensure_ascii=False))
        data = read_mcp_file()
        data["mcpServers"]["everything"]["env"] = {"KOTONE_E2E": "changed"}
        with open(os.path.join(HOME, "mcp.json"), "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
        reloaded = invoke("mcp_servers_reload")
        card = remote_of(invoke("plugins_list").get("ok") or [], "everything") or {}
        check("config changed → its “always allow” rule is cleared and it is no longer trusted",
              "error" not in reloaded and "everything.echo" not in rules()
              and (card.get("remote") or {}).get("trusted") is False,
              json.dumps({"rules": rules(), "trusted": (card.get("remote") or {}).get("trusted")}, ensure_ascii=False))

        # 8. a stdio server still running when Kotone quits goes with it (Job Object)
        invoke("update_settings", {"patch": {"plugins": {"everything": {"enabled": True}}}})
        again = wait_until(lambda: connected(["everything"]), 60, 2)
        check("switching it back on connects again", again)
        server_pids[:] = everything_pids()
        check("its program is running", server_pids, f"{server_pids}")
    finally:
        if app is not None:
            tools_call("connections.delete", {"id": "mock"}, timeout_ms=5000)
            tools_call("app.quit", timeout_ms=5000)
            try:
                app.wait(20)
            except subprocess.TimeoutExpired:
                app.kill()
        if server_pids:
            time.sleep(2)
            left_over = [p for p in server_pids if p in everything_pids()]
            check("after Kotone quit, the stdio server's processes are gone", not left_over, f"{left_over}")
            for pid in left_over:
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(pid)], capture_output=True)
        if http_server is not None:
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(http_server.pid)], capture_output=True)
        if mock is not None:
            mock.shutdown()
            mock.server_close()
        sweep_credentials()
        if not keep:
            time.sleep(1)
            shutil.rmtree(WORK, ignore_errors=True)
        os.rmdir(LOCK)
    passed = sum(1 for _, ok in RESULTS if ok)
    print(f"\n{passed}/{len(RESULTS)} passed")
    sys.exit(0 if passed == len(RESULTS) else 1)


if __name__ == "__main__":
    main()
