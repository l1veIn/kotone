"""MCP plan phase 0 (2026-11-25): `kotone.exe mcp` maps tool names that already contain underscores back correctly.

The bug: `kotone.exe mcp` lists Kotone's `x.y` as `kotone_x_y` and, when called, turned every `_` back into `.`, so
`kotone_voice_chat_status` became `voice.chat.status` (no such tool). The MCP names stay the same (external Agents may
have remembered them or set permissions on them); the server now looks the Kotone name up in a table built from
capabilities. This is one real MCP conversation over stdio against a real Kotone:

- a real desktop app (`--background`: no window, no focus change) in an isolated KOTONE_HOME=target/mcp-names-real/home:
  tool service on, the game voice plugin on (`plugins.game-voice.enabled`) with `includeMic: false` (the player's
  microphone is never opened), speech volume 0, hotkeys and the wake word off, no AI connection;
- session 1: initialize → notifications/initialized → tools/list (must list kotone_voice_chat_status and
  kotone_voice_chat_default_mic, names unchanged, no duplicates) → tools/call kotone_voice_chat_status (read only:
  VB-CABLE detection, the Windows default microphone, the plugin's settings) → kotone_call {tool: voice_chat.status}
  (the old generic path, for comparison) → a name that does not exist (the error says what to do next);
- session 2 (a new `mcp` process): tools/call kotone_voice_chat_status WITHOUT tools/list first (the server fetches
  capabilities itself).

Never calls game_chat.send, voice_chat.say, voice_chat.test, voice_chat.default_mic or voice_chat.install (they type,
speak, change the Windows default microphone or install a driver): `call()` refuses them. Sends no OS input. Stops
before starting if a game is running or in front. The exes are copied to target/mcp-names-real/bin first (a running
exe would block other builds); the work directory is deleted afterwards (`--keep` keeps it).

`kotone.exe mcp` (what the installed app offers) and `kotone-cli mcp` (dev build) run the same code
(`kotone_cli::main_with_args`). `--mcp` picks the exe for the MCP side; default = the app exe.
Prerequisite: a debug build (`cargo build -p kotone-tauri`, the web UI is not needed with --background), or
`cargo build -p kotone-cli` and `--mcp target/debug/kotone-cli.exe` when only the CLI was rebuilt.
usage: python -X utf8 -I scripts/manual-e2e/mcp_names_real.py [--exe PATH] [--mcp PATH] [--keep]
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
WORK = os.path.join(REPO, "target", "mcp-names-real")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")


def arg(name, default):
    return os.path.abspath(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


SRC_EXE = arg("--exe", os.path.join(REPO, "target", "debug", "kotone.exe"))
SRC_MCP = arg("--mcp", SRC_EXE)
EXE = os.path.join(BIN, "kotone.exe")
MCP = EXE if SRC_MCP == SRC_EXE else os.path.join(BIN, "mcp", os.path.basename(SRC_MCP))
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)
# 会打字、出声、改 Windows 默认麦克风、装驱动的：这个脚本一个都不调
FORBIDDEN = {"game_chat.send", "voice_chat.say", "voice_chat.test", "voice_chat.default_mic", "voice_chat.install"}
FORBIDDEN_MCP = {"kotone_" + t.replace(".", "_") for t in FORBIDDEN}
UNLISTED = {"system.capabilities", "event.read"}
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


def find(value, key):
    """First value of `key` anywhere in nested dicts / lists."""
    if isinstance(value, dict):
        if key in value:
            return value[key]
        value = list(value.values())
    if isinstance(value, list):
        for v in value:
            found = find(v, key)
            if found is not None:
                return found
    return None


class Mcp:
    """One `mcp` process: newline-delimited JSON-RPC on stdin / stdout (MCP stdio)."""

    def __init__(self):
        self.proc = subprocess.Popen([MCP, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
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
        rid = self.next
        self.next += 1
        print(f"→ {method}" + (f" {params.get('name')}" if params and "name" in params else ""))
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

    def call(self, name, arguments=None):
        """tools/call → (structuredContent or parsed text, isError)."""
        if name in FORBIDDEN_MCP or (name == "kotone_call" and (arguments or {}).get("tool") in FORBIDDEN):
            raise SystemExit(f"refusing to call {name} {arguments}: it types / speaks / changes the system")
        msg = self.request("tools/call", {"name": name, "arguments": arguments or {}})
        result = msg.get("result") or {}
        body = result.get("structuredContent")
        if body is None:
            text = (result.get("content") or [{}])[0].get("text", "")
            try:
                body = json.loads(text)
            except json.JSONDecodeError:
                body = {"text": text, "error": msg.get("error")}
        return body, result.get("isError") is True

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


def status_summary(body):
    keys = ("driverInstalled", "enabled", "includeMic", "running", "ttsCanSpeak")
    return {k: find(body, k) for k in keys}


def session_one():
    mcp = Mcp()
    try:
        init = mcp.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                          "clientInfo": {"name": "mcp-names-real", "version": "1"}})
        info = init.get("result", {})
        check("initialize: serverInfo kotone, tools.listChanged",
              info.get("serverInfo", {}).get("name") == "kotone" and info["capabilities"]["tools"]["listChanged"],
              f"{info.get('serverInfo')} protocol {info.get('protocolVersion')}")
        mcp.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

        listed = mcp.request("tools/list")["result"]["tools"]
        names = [t["name"] for t in listed]
        check("every listed name is unique and starts with kotone_",
              len(set(names)) == len(names) and all(n.startswith("kotone_") for n in names), f"{len(names)} tools")
        check("every inputSchema is an object", all(t["inputSchema"].get("type") == "object" for t in listed))
        expected = ["kotone_voice_chat_status", "kotone_voice_chat_default_mic", "kotone_voice_chat_say"]
        missing = [n for n in expected if n not in names]
        check("tools/list has the game voice tools under their old MCP names", not missing,
              f"missing {missing}" if missing else ", ".join(expected))

        # Kotone 自己的名字（read only）：名字里带下划线的，每个都照旧以 kotone_<点换成下划线> 列出来
        caps, is_error = mcp.call("kotone_capabilities")
        kotone = [t["name"] for t in caps.get("tools", [])] if not is_error else []
        underscored = sorted(t for t in kotone if "_" in t and t not in UNLISTED)
        print(f"  Kotone tools with an underscore in the name ({len(underscored)}):", ", ".join(underscored))
        not_listed = [t for t in underscored if "kotone_" + t.replace(".", "_") not in names]
        check("every Kotone tool with an underscore is listed under its old MCP name",
              underscored and not not_listed, f"not listed: {not_listed}" if not_listed else "")
        print("  game_chat.send listed:", "kotone_game_chat_send" in names, "(not called)")

        body, is_error = mcp.call("kotone_voice_chat_status")
        summary = status_summary(body)
        print("  ←", json.dumps(summary, ensure_ascii=False))
        check("tools/call kotone_voice_chat_status reaches voice_chat.status",
              not is_error and summary["driverInstalled"] is not None and summary["enabled"] is True,
              "" if not is_error else json.dumps(body, ensure_ascii=False)[:400])
        check("the player's microphone stays out (includeMic false)", summary["includeMic"] is False)

        body, is_error = mcp.call("kotone_call", {"tool": "voice_chat.status"})
        check("kotone_call {tool: voice_chat.status} (the generic path) answers too",
              not is_error and find(body, "driverInstalled") is not None)

        body, is_error = mcp.call("kotone_voice_chat_nope")
        print("  ←", json.dumps({k: body.get(k) for k in ("error", "hint")}, ensure_ascii=False))
        check("an unknown name is an error that says what to do next",
              is_error and "tools/list" in str(body.get("hint")) and "kotone_call" in str(body.get("hint")))
    finally:
        mcp.close()


def session_two():
    mcp = Mcp()
    try:
        mcp.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                   "clientInfo": {"name": "mcp-names-real", "version": "1"}})
        mcp.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        body, is_error = mcp.call("kotone_voice_chat_status")
        check("a new session can call kotone_voice_chat_status without tools/list first",
              not is_error and find(body, "driverInstalled") is not None,
              "" if not is_error else json.dumps(body, ensure_ascii=False)[:400])
    finally:
        mcp.close()


def main():
    guard()
    for path in {SRC_EXE, SRC_MCP}:
        if not os.path.exists(path):
            raise SystemExit(f"missing {path}: build it first (see the docstring)")
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    os.makedirs(os.path.dirname(MCP), exist_ok=True)
    shutil.copy2(SRC_EXE, EXE)
    if MCP != EXE:
        shutil.copy2(SRC_MCP, MCP)
    print(f"app: {SRC_EXE}\nmcp: {SRC_MCP} mcp")
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "plugins": {"game-voice": {"enabled": True, "settings": {"includeMic": False}}}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    try:
        status = wait_service()
        print("isolated instance up:", status.get("appVersion"), "pid", status.get("pid"))
        session_one()
        session_two()
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
