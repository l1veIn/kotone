"""MCP plan phase 3/5 (2026-11-25, ADR 039 D5): `kotone.exe mcp` offers the League of Legends MCP App.

One real MCP conversation over stdio against a real, isolated Kotone (the same way Claude Desktop / Codex would talk to
it):

- a real desktop app (`--background`: no window, no focus change) in an isolated KOTONE_HOME=target/mcp-app-real/home:
  tool service on, the League of Legends plugin with its defaults (it only reads the local client, which is not running
  tonight), speech volume 0, hotkeys and the wake word off, no AI connection;
- initialize (capabilities: `resources` and the MCP Apps extension `io.modelcontextprotocol/ui`) →
  notifications/initialized → resources/list (the pages that are MCP Apps, `plugins/lol/src/app.rs` APP_PAGES:
  ui://kotone/lol/tier and ui://kotone/lol/match, text/html;profile=mcp-app, CSP declares CommunityDragon; the other
  pages are not listed: player / game / champion are drawn inside the same bundle by its own navigation, settings is the
  host's page) → resources/read of each App (a whole HTML document with `<meta name="kotone:page" content="<page>">`;
  the real bundle when `pnpm -C apps/desktop build:apps` ran before the cargo build, otherwise the placeholder page —
  reported, not failed) → resources/read of a page that is not an App (the settings page: error -32002) →
  tools/list (`kotone_lol_tier` and `kotone_lol_scout` have `_meta.ui.resourceUri`, `kotone_lol_player` does not (its
  page is not offered on its own), `kotone_lol_champions` is listed with visibility ["app"]);
- unless `--offline`: tools/call kotone_lol_tier {mode: rift, lane: 中} — one public request to Tencent 101 (no account,
  no key): the result follows MCP Apps (content = the few lines for the model, structuredContent = the page data).
  A network failure is reported as SKIP, not FAIL.

Sends no OS input, plays no sound, never calls lol.client / game_chat.send. Stops before starting if a game is running or
in front. The exe is copied to target/mcp-app-real/bin first (a running exe would block other builds); the work directory
is deleted afterwards (`--keep` keeps it). Prerequisite: `pnpm -C apps/desktop build:apps` then
`cargo build -p kotone-tauri` (the web UI is not needed with --background).
usage: python -X utf8 -I scripts/manual-e2e/mcp_app_resources_real.py [--exe PATH] [--offline] [--keep]
"""
import json
import os
import queue
import shutil
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "mcp-app-real")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")
SRC_EXE = (os.path.abspath(sys.argv[sys.argv.index("--exe") + 1]) if "--exe" in sys.argv
           else os.path.join(REPO, "target", "debug", "kotone.exe"))
EXE = os.path.join(BIN, "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)
APP_MIME = "text/html;profile=mcp-app"
TIER = "ui://kotone/lol/tier"
MATCH = "ui://kotone/lol/match"
# 做成 MCP App 的页面（plugins/lol/src/app.rs 的 APP_PAGES）→ 页面标记
APPS = {TIER: "tier", MATCH: "match"}
# 清单里有、但不单独作为资源给的页面：战绩 / 单局详情 / 英雄在同一个打包里由 App 自己导航过去，设置是宿主的页面
NOT_APPS = ("ui://kotone/lol/player", "ui://kotone/lol/game", "ui://kotone/lol/champion", "ui://kotone/lol/settings")
SETTINGS = "ui://kotone/lol/settings"
# 会打字、出声、操作客户端的：这个脚本一个都不调
FORBIDDEN = {"kotone_lol_client", "kotone_game_chat_send", "kotone_voice_chat_say", "kotone_call"}
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
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


class Mcp:
    """One `kotone.exe mcp` process: newline-delimited JSON-RPC on stdin / stdout (MCP stdio)."""

    def __init__(self):
        self.proc = subprocess.Popen([EXE, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, env=ENV)
        self.lines = queue.Queue()
        self.next = 1
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def send(self, message):
        self.proc.stdin.write((json.dumps(message, ensure_ascii=False) + "\n").encode("utf-8"))
        self.proc.stdin.flush()

    def request(self, method, params=None, timeout=45):
        if method == "tools/call" and (params or {}).get("name") in FORBIDDEN:
            raise SystemExit(f"refusing to call {params}: it types / speaks / controls the client")
        rid = self.next
        self.next += 1
        print(f"→ {method}" + (f" {params.get('name') or params.get('uri')}" if params else ""))
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}})
        deadline = time.time() + timeout
        while True:
            line = self.lines.get(timeout=max(0.1, deadline - time.time()))
            if line is None:
                raise RuntimeError(f"{method}: the mcp process exited")
            msg = json.loads(line)
            if msg.get("id") == rid:
                return msg
            print("  (notification)", msg.get("method"))

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(10)
        except Exception:  # noqa: BLE001
            self.proc.kill()


def cli(*args, timeout=30):
    out = subprocess.run([EXE, *args], capture_output=True, text=True, encoding="utf-8", env=ENV,
                         stdin=subprocess.DEVNULL, timeout=timeout)
    try:
        return out.returncode, json.loads(out.stdout)
    except json.JSONDecodeError:
        return out.returncode, {"raw": out.stdout[-500:]}


def wait_service(timeout=60):
    deadline = time.time() + timeout
    while time.time() < deadline:
        code, value = cli("tools", "call", "app.status", "--timeout-ms", "3000")
        if code == 0 and value.get("ok"):
            return value["result"]
        time.sleep(1)
    raise RuntimeError("the isolated instance's tool service did not come up")


def session(offline):
    mcp = Mcp()
    try:
        init = mcp.request("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {"extensions": {"io.modelcontextprotocol/ui": {"mimeTypes": [APP_MIME]}}},
            "clientInfo": {"name": "mcp-app-real", "version": "1"}})
        caps = init.get("result", {}).get("capabilities", {})
        print("  capabilities:", json.dumps(caps, ensure_ascii=False))
        check("initialize declares resources and the MCP Apps extension",
              "resources" in caps and APP_MIME in caps.get("extensions", {})
              .get("io.modelcontextprotocol/ui", {}).get("mimeTypes", []))
        mcp.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

        resources = mcp.request("resources/list").get("result", {}).get("resources", [])
        print("  resources:", json.dumps(resources, ensure_ascii=False))
        listed = {r.get("uri"): r for r in resources}
        for uri in APPS:
            app = listed.get(uri)
            domains = ((app or {}).get("_meta", {}).get("ui", {}).get("csp", {}).get("resourceDomains", []))
            check(f"resources/list has the {uri} App ({APP_MIME}); its CSP declares CommunityDragon (icons)",
                  app and app.get("mimeType") == APP_MIME and "https://raw.communitydragon.org" in domains,
                  json.dumps(app, ensure_ascii=False) if app else "missing")
        check("pages that are not offered as Apps on their own (player, game, champion, settings) are not listed",
              not [uri for uri in NOT_APPS if uri in listed], str([uri for uri in NOT_APPS if uri in listed]))

        for uri, page in APPS.items():
            read = mcp.request("resources/read", {"uri": uri}).get("result", {})
            content = (read.get("contents") or [{}])[0]
            html = content.get("text", "")
            built = "build:apps" not in html
            print(f"  read {len(html)} chars; {'the built App' if built else 'the placeholder page (App not built)'}")
            check(f"resources/read {uri}: a whole HTML document as {APP_MIME} with its page marker",
                  content.get("mimeType") == APP_MIME and html.lstrip().lower().startswith("<!doctype html")
                  and f'<meta name="kotone:page" content="{page}">' in html)
            if built and uri == TIER:
                check("the built App talks MCP Apps (ui/initialize is in the bundle)", "ui/initialize" in html)

        missing = mcp.request("resources/read", {"uri": SETTINGS})
        check(f"a page that is not an App ({SETTINGS}, the host's page) is resource-not-found (-32002)",
              missing.get("error", {}).get("code") == -32002, json.dumps(missing.get("error"), ensure_ascii=False))

        tools = {t["name"]: t for t in mcp.request("tools/list")["result"]["tools"]}
        meta = lambda name: tools.get(name, {}).get("_meta", {})  # noqa: E731
        print("  kotone_lol_tier _meta:", json.dumps(meta("kotone_lol_tier"), ensure_ascii=False))
        print("  kotone_lol_scout _meta:", json.dumps(meta("kotone_lol_scout"), ensure_ascii=False))
        for tool, uri in (("kotone_lol_tier", TIER), ("kotone_lol_scout", MATCH)):
            check(f"{tool} links its App {uri} (_meta.ui.resourceUri, and the old flat key)",
                  meta(tool).get("ui", {}).get("resourceUri") == uri and meta(tool).get("ui/resourceUri") == uri)
        check("kotone_lol_player does not link a page external hosts cannot read",
              "kotone_lol_player" in tools and "resourceUri" not in meta("kotone_lol_player").get("ui", {})
              and "ui/resourceUri" not in meta("kotone_lol_player"),
              json.dumps(meta("kotone_lol_player"), ensure_ascii=False))
        check("an app-only tool is listed with visibility [\"app\"]",
              meta("kotone_lol_champions").get("ui", {}).get("visibility") == ["app"])

        if offline:
            print("SKIP tools/call kotone_lol_tier (--offline)")
            return
        result = mcp.request("tools/call", {"name": "kotone_lol_tier",
                                            "arguments": {"mode": "rift", "lane": "中"}}, timeout=60).get("result", {})
        data = result.get("structuredContent") or {}
        text = (result.get("content") or [{}])[0].get("text", "")
        print("  ←", text[:200].replace("\n", " "))
        if data.get("mode") == "rift":
            rows = next(iter(data.get("lanes", {}).values()), [])
            check("tools/call kotone_lol_tier: structuredContent is the page data, content is the short text",
                  rows and "lanes" not in text and (result.get("_meta") or {}).get("ui", {}).get("resourceUri") == TIER,
                  f"{len(rows)} rows, first {rows[0]['champion']['short'] if rows else '-'}")
        else:
            print("SKIP tools/call kotone_lol_tier: no data (network / Tencent 101 down?):", text[:300])
    finally:
        mcp.close()


def main():
    guard()
    if not os.path.exists(SRC_EXE):
        raise SystemExit(f"missing {SRC_EXE}: build it first (see the docstring)")
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    os.makedirs(BIN)
    shutil.copy2(SRC_EXE, EXE)
    print(f"app: {SRC_EXE}")
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    try:
        status = wait_service()
        print("isolated instance up:", status.get("appVersion"), "pid", status.get("pid"))
        session("--offline" in sys.argv)
    finally:
        cli("tools", "call", "app.quit")
        try:
            app.wait(15)
        except subprocess.TimeoutExpired:
            app.terminate()
            app.wait(10)
        if "--keep" not in sys.argv:
            for _ in range(10):
                shutil.rmtree(WORK, ignore_errors=True)
                if not os.path.exists(WORK):
                    break
                time.sleep(1)
            print("cleaned up", WORK if not os.path.exists(WORK) else f"(could not delete {WORK})")
    failed = [n for n, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; FAILED: {failed}" if failed else ""))
    sys.exit(1 if failed else 0)


main()
