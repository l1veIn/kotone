"""ADR 047 phase 2 (2026-11-28): third-party events from watches, on the real Kotone, with a FAKE Zenless Zone Zero
OneDragon (ZenlessZoneZero-OneDragon) MCP server written in this script.

The fake server speaks Streamable HTTP (plain JSON replies) on a free 127.0.0.1 port — never 23001, so a OneDragon
the player installed is not touched — and copies the real one's surface (docs/develop/zzz/backend/mcp.md and
src/zzz_od/backend/schemas.py, commit f0975a64, 2026-11-25): server name `zzz_od`, `get_run_status`
(readOnlyHint, returns `RunStatusResult` as structuredContent + the same JSON as text), `run_one_dragon`, `stop_run`;
no `kotone://manifest` resource, so Kotone uses its bundled preset (recognised by the server name `zzz_od`).
The run state is scripted: idle → running → success, later running → failed.

The game is a FAKE process too: a copy of PING.EXE renamed `ZenlessZoneZero.exe` under target/ (pinging 127.0.0.1),
so the game hub sees 绝区零 running; the real game is never started or touched.

No AI key, no OS input, nothing typed, no sound. The real desktop app (debug build with the web UI) runs
`--background` in an isolated KOTONE_HOME=target/remote-watch-real/home; host commands go through WebView2's DevTools
port (`window.__TAURI_INTERNALS__.invoke` in the settings page), tools through `kotone.exe tools call`.

Checks:
W1 pasted as `{"mcpServers": {"zzz_od": {...}}}` → added as `zzz-od`, connects, the bundled OneDragon preset applies
   (games zzz, two events, two watches on get_run_status, preset id zzz-onedragon), both watches off;
W2 nothing is polled while the watches are off; turning one on without the digest is refused;
W3 the template “一条龙跑完推一行” is in `automation_saved_list`; saved with its trigger mapped to `zzz-od.*`;
W4 both watches on (confirmed with their digests) → waiting (绝区零 is not running), still no calls;
W5 the fake 绝区零 starts → polling: the first call only records the baseline (no event);
W6 running → success: kotone.log has the third-party event `zzz-od.run_finished` (game zzz, from running to success,
   lastStatus with a marker) → the automation fired → the conversation has the push “绝区零一条龙跑完了”;
W7 her context never sees the event: game.state's recent events for zzz have no `zzz-od.*` (the catalog lists the group
   marked third-party), and the marker text is nowhere in the conversation files;
W8 the fake 绝区零 exits, then running → failed within the 3-minute grace: `zzz-od.run_failed` still arrives (game zzz),
   and the push “绝区零一条龙出错了，去看看”;
W9 one call per round for the two watches on the same tool (calls ≈ rounds, not twice).

Stops before starting if a game is running or in front. Takes target/tmp/real.lock. Prerequisites (README 准备):
`pnpm -C apps/desktop build:web`, then `cargo build -p kotone-tauri --features tauri/custom-protocol`; the exe is
copied to target/remote-watch-real/bin first. Takes about 3 minutes (the preset polls every 30 seconds).
usage: python -X utf8 -I scripts/manual-e2e/remote_watch_real.py [--keep]
"""
import glob
import json
import os
import re
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
WORK = os.path.join(REPO, "target", "remote-watch-real")
HOME = os.path.join(WORK, "home")
FAKE_GAME_DIR = os.path.join(WORK, "fake-zzz")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
SRC_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
BIN = os.path.join(WORK, "bin")
EXE = os.path.join(BIN, "kotone.exe")
DEVTOOLS = 9358
MARKER = "E2E标记-全部完成"
RESULTS = []

ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={DEVTOOLS}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def guard():
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, encoding="mbcs",
                         errors="replace").stdout.lower()
    running = [g for g in ("league of legends.exe", "leagueclient", "riotclient", "valorant", "cs2.exe",
                           "dota2.exe", "yuanshen", "genshinimpact", "zenlesszonezero") if g in out]
    if running:
        raise SystemExit(f"STOP: a game is running ({running}); not testing while the player plays")
    path = foreground_process()
    if any(g in os.path.basename(path).lower() for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    return port if port != 23001 else free_port()


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


# ---------- the fake OneDragon (Streamable HTTP, JSON replies) ----------

FAKE = {"state": "idle", "calls": 0, "names": [], "lock": threading.Lock()}


def run_status():
    """`RunStatusResult` (schemas.py): state idle/running/success/failed/stopped and the rest."""
    state = FAKE["state"]
    idle = state == "idle"
    status = {
        "state": state,
        "source": None if idle else "mcp",
        "app": None if idle else "OneDragonApp",
        "started_at": None if idle else "2026-11-28T10:00:00",
        "duration_seconds": None if idle else 321.5,
        "current_node": "体力刷本" if state == "running" else None,
        "retry_count": 0 if state == "running" else None,
        "last_status": MARKER if state == "success" else ("识别不到画面" if state == "failed" else None),
        "failed_node": "进入副本" if state == "failed" else None,
    }
    return {"content": [{"type": "text", "text": json.dumps(status, ensure_ascii=False)}], "structuredContent": status,
            "isError": False}


TOOLS = [
    {"name": "get_run_status", "title": "查询运行状态", "description": "查当前/最近一次运行状态(无副作用)。观察类。",
     "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": True, "title": "查询运行状态"}},
    {"name": "run_one_dragon", "description": "运行一条龙", "annotations": {"title": "运行一条龙"},
     "inputSchema": {"type": "object", "properties": {"block": {"type": "boolean"}}}},
    {"name": "stop_run", "description": "停止运行", "annotations": {"title": "停止运行"},
     "inputSchema": {"type": "object", "properties": {}}},
]


class FakeOneDragon(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, body=None, headers=None):
        data = json.dumps(body, ensure_ascii=False).encode() if body is not None else b""
        self.send_response(status)
        if body is not None:
            self.send_header("content-type", "application/json")
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.reply(405)

    def do_DELETE(self):
        self.reply(200)

    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("content-length", "0") or 0))
        msg = json.loads(raw or b"{}")
        method, rid, params = msg.get("method"), msg.get("id"), msg.get("params") or {}
        if rid is None:  # notification
            self.reply(202)
            return
        ok = lambda result, headers=None: self.reply(200, {"jsonrpc": "2.0", "id": rid, "result": result}, headers)  # noqa: E731
        if method == "initialize":
            ok({"protocolVersion": params.get("protocolVersion", "2025-11-25"),
                "capabilities": {"tools": {}, "resources": {}},
                "serverInfo": {"name": "zzz_od", "version": "fake-e2e"},
                "instructions": "fake OneDragon for the Kotone e2e"}, {"mcp-session-id": "fake-session"})
        elif method == "ping":
            ok({})
        elif method == "tools/list":
            ok({"tools": TOOLS})
        elif method == "resources/list":
            ok({"resources": []})
        elif method == "resources/read":
            self.reply(200, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32002, "message": "Resource not found"}})
        elif method == "tools/call":
            name = params.get("name")
            with FAKE["lock"]:
                FAKE["names"].append(name)
                if name == "get_run_status":
                    FAKE["calls"] += 1
            if name == "get_run_status":
                ok(run_status())
            else:
                ok({"content": [{"type": "text", "text": json.dumps({"started": True})}], "isError": False})
        else:
            self.reply(200, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": f"no {method}"}})


def calls():
    with FAKE["lock"]:
        return FAKE["calls"]


def set_state(state):
    FAKE["state"] = state


# ---------- the fake game process ----------

def start_fake_game():
    os.makedirs(FAKE_GAME_DIR, exist_ok=True)
    exe = os.path.join(FAKE_GAME_DIR, "ZenlessZoneZero.exe")
    if not os.path.exists(exe):
        shutil.copy2(os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "PING.EXE"), exe)
    return subprocess.Popen([exe, "-n", "600", "127.0.0.1"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            creationflags=subprocess.CREATE_NO_WINDOW)


# ---------- reading Kotone ----------

def write_config():
    config = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "legacyUpgrade": "notNeeded"},
        "adminPromptDismissed": True,
        "runAsAdminOnStart": False,
        "toolService": {"enabled": True},
        "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False},
                      "bubble": {"enabled": False}},
        "agent": {"progressSpeech": False, "autoCards": {"popup": False}, "matchRecap": False},
    }
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, ensure_ascii=False, indent=1)


def remote(pid="zzz-od"):
    plugins = invoke("plugins_list").get("ok") or []
    return next((p.get("remote") for p in plugins if (p.get("manifest") or {}).get("id") == pid), None)


def watch(event, pid="zzz-od"):
    r = remote(pid) or {}
    return next((w for w in r.get("watches") or [] if w.get("event") == event), None)


def log_text():
    try:
        return open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except OSError:
        return ""


def third_party_events():
    out = []
    for line in log_text().splitlines():
        m = re.search(r"game: 第三方 (\S+) (zzz-od\.[a-z_]+) (\{.*\})\s*$", line)
        if m:
            try:
                out.append((m.group(1), m.group(2), json.loads(m.group(3))))
            except json.JSONDecodeError:
                pass
    return out


def conversation_text():
    texts = []
    for path in glob.glob(os.path.join(HOME, "agent", "**", "*.json*"), recursive=True):
        try:
            texts.append(open(path, encoding="utf-8", errors="replace").read())
        except OSError:
            pass
    return "\n".join(texts)


def pushed(text):
    return text in conversation_text()


def main():
    keep = "--keep" in sys.argv
    guard()
    os.makedirs(os.path.dirname(LOCK), exist_ok=True)
    try:
        os.mkdir(LOCK)
    except FileExistsError:
        raise SystemExit(f"{LOCK} exists: another real-machine test is running (remove it if stale)")
    server = app = game = None
    try:
        if not os.path.exists(SRC_EXE):
            raise SystemExit("no target/debug/kotone.exe — build it first (see the docstring)")
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        shutil.copy2(SRC_EXE, EXE)
        write_config()

        port = free_port()
        server = ThreadingHTTPServer(("127.0.0.1", port), FakeOneDragon)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        url = f"http://127.0.0.1:{port}/mcp"

        app = subprocess.Popen([EXE, "--background"], env=ENV)
        if not wait_until(lambda: tools_call("app.status", timeout_ms=3000).get("ok"), 90):
            raise SystemExit("the isolated instance's tool service did not come up")
        if not wait_until(lambda: "ok" in invoke("agent_state"), 90):
            raise SystemExit("the settings page did not come up (DevTools)")

        # W1 add it the Claude-Code way
        paste = json.dumps({"mcpServers": {"zzz_od": {"type": "http", "url": url}}})
        preview = invoke("mcp_servers_preview", {"text": paste}).get("ok") or {}
        cand = (preview.get("servers") or [{}])[0]
        check("W1 preview suggests zzz-od", cand.get("id") == "zzz-od", json.dumps(cand, ensure_ascii=False)[:200])
        added = invoke("mcp_server_add", {"id": cand.get("id"), "entry": cand.get("entry")})
        check("W1 added", "ok" in added, json.dumps(added, ensure_ascii=False)[:200])
        r = wait_until(lambda: (lambda x: x if x and (x.get("kotoneManifest") or {}).get("source") == "preset"
                                and len(x.get("watches") or []) == 2 else None)(remote()), 60)
        if not check("W1 connected; the bundled OneDragon preset applies", r,
                     json.dumps(remote(), ensure_ascii=False)[:600]):
            return
        km = r["kotoneManifest"]
        check("W1 games zzz, preset id zzz-onedragon, two events",
              km["manifest"]["games"] == ["zzz"] and km.get("preset") == "zzz-onedragon"
              and [e["name"] for e in km["manifest"]["events"]] == ["run_finished", "run_failed"], json.dumps(km)[:300])
        check("W1 trusted on add, both watches off, tool read-only",
              r.get("trusted") is True and all(w["status"]["state"] == "off" and w["readOnly"] is True
                                               for w in r["watches"]),
              json.dumps([(w["event"], w["status"], w["readOnly"]) for w in r["watches"]], ensure_ascii=False))

        # W2 nothing polled while off; turning on without the digest is refused
        time.sleep(5)
        check("W2 no get_run_status calls while the watches are off", calls() == 0, str(calls()))
        refused = invoke("remote_watch_set", {"id": "zzz-od", "event": "run_finished", "on": True})
        check("W2 turning on without confirming is refused", "error" in refused, json.dumps(refused, ensure_ascii=False))

        # W3 the template
        saved_list = invoke("automation_saved_list").get("ok") or {}
        template = next((t for t in saved_list.get("templates") or [] if t.get("id") == "onedragon-push"), None)
        if not check("W3 the template is listed", template, str([t.get("id") for t in saved_list.get("templates") or []])):
            return
        on = dict(template["on"])
        on["event"] = [e.replace("zzz-onedragon.", "zzz-od.", 1) for e in on["event"]]
        draft = {"name": "一条龙跑完推一行", "description": template["description"], "params": template["params"],
                 "action": {"kind": "program", "source": template["source"]}, "schedule": None, "on": on}
        saved = invoke("automation_save", {"draft": draft, "confirm": False})
        check("W3 saved with the trigger on zzz-od.*", "ok" in saved, json.dumps(saved, ensure_ascii=False)[:300])

        # W4 both on → waiting (绝区零 not running)
        for event in ("run_finished", "run_failed"):
            w = watch(event)
            res = invoke("remote_watch_set", {"id": "zzz-od", "event": event, "on": True, "digest": w["digest"]})
            check(f"W4 {event} turned on", "ok" in res, json.dumps(res, ensure_ascii=False)[:200])
        check("W4 waiting for the game", wait_until(lambda: (watch("run_finished") or {}).get("status", {}).get("state")
                                                    == "waiting", 15), json.dumps(watch("run_finished"))[:300])
        time.sleep(4)
        check("W4 still no calls (the game is not running)", calls() == 0, str(calls()))

        # W5 the fake game starts → polling, baseline only
        game = start_fake_game()
        first = wait_until(lambda: calls() >= 1, 30, 0.5)
        check("W5 the game started → it polls", first, str(calls()))
        state = (watch("run_finished") or {}).get("status", {}).get("state")
        check("W5 the watch says polling", state == "polling", str(state))
        time.sleep(1)
        check("W5 first call only records the baseline", not third_party_events(), str(third_party_events()))

        # W6 running → success
        set_state("running")
        base = calls()
        wait_until(lambda: calls() > base, 45, 0.5)
        set_state("success")
        base = calls()
        wait_until(lambda: calls() > base, 45, 0.5)
        finished = wait_until(lambda: next((e for e in third_party_events() if e[1] == "zzz-od.run_finished"), None), 10)
        check("W6 the event reached the game hub (zzz, running → success, lastStatus)",
              finished and finished[0] == "zzz" and finished[2].get("from") == "running"
              and finished[2].get("to") == "success" and finished[2].get("lastStatus") == MARKER,
              json.dumps(finished, ensure_ascii=False))
        check("W6 only registered fields (no currentNode / startedAt)",
              finished and set(finished[2]) <= {"from", "to", "app", "lastStatus", "durationSeconds"},
              json.dumps(finished[2] if finished else None, ensure_ascii=False))
        fired = wait_until(lambda: re.search(r"automation \S+: 游戏事件 zzz-od\.run_finished 触发（zzz）", log_text()), 10)
        check("W6 the automation fired", fired)
        check("W6 the template pushed a line", wait_until(lambda: pushed("绝区零一条龙跑完了"), 20))

        # W7 not in her context
        gs = tools_call("game.state")
        gtext = json.dumps(gs, ensure_ascii=False)
        playing = ((gs.get("result") or {}).get("playing")) or []
        zzz = next((g for g in playing if g.get("game") == "zzz"), {})
        check("W7 game.state recent events have no third-party event",
              zzz and not any(str(e.get("type", "")).startswith("zzz-od.") for e in zzz.get("recent") or []),
              json.dumps(zzz.get("recent") if zzz else gs, ensure_ascii=False)[:300])
        groups = (gs.get("result") or {}).get("events") or []
        check("W7 the catalog group is marked third-party",
              any(g.get("adapter") == "zzz-od" and g.get("thirdParty") for g in groups), gtext[:200])
        check("W7 the marker text is nowhere in the conversations", MARKER not in conversation_text())

        # W8 the game exits; failed within the grace
        game.kill()
        game.wait(5)
        game = None
        time.sleep(4)
        set_state("running")
        base = calls()
        got = wait_until(lambda: calls() > base, 60, 0.5)
        check("W8 still polling right after the game exited (grace)", got, str(calls()))
        set_state("failed")
        base = calls()
        wait_until(lambda: calls() > base, 60, 0.5)
        failed = wait_until(lambda: next((e for e in third_party_events() if e[1] == "zzz-od.run_failed"), None), 10)
        check("W8 run_failed arrives in the grace period, on zzz",
              failed and failed[0] == "zzz" and failed[2].get("failedNode") == "进入副本",
              json.dumps(failed, ensure_ascii=False))
        check("W8 the template pushed the failure line", wait_until(lambda: pushed("绝区零一条龙出错了，去看看"), 20))

        # W9 one call per round
        rounds_note = f"{calls()} calls"
        finished_events = [e for e in third_party_events() if e[1] == "zzz-od.run_finished"]
        check("W9 run_finished once, calls not doubled by two watches", len(finished_events) == 1 and calls() <= 8,
              rounds_note)
    finally:
        if game:
            game.kill()
        if app:
            try:
                tools_call("app.quit", timeout_ms=5000)
            except Exception as e:  # noqa: BLE001
                print("quit:", e)
            time.sleep(3)
            if app.poll() is None:
                app.terminate()
        if server:
            server.shutdown()
        try:
            os.rmdir(LOCK)
        except OSError:
            pass
        if not keep:
            shutil.rmtree(FAKE_GAME_DIR, ignore_errors=True)
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
