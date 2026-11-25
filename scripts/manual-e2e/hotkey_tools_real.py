"""ADR 040 phase 3 step 2 on the real Kotone: hotkeys the player adds that call a tool, run an automation or open a page,
and "ask once when it is bound" (D9) — with a third-party MCP server (@modelcontextprotocol/server-everything over
stdio, as in mcp_servers_real.py).

========================================================================================================================
SAFETY
  * This script sends NO keyboard or mouse input and types nothing: everything goes through the isolated instance's
    tool service (`kotone.exe tools call`, i.e. what an external agent does) and WebView2's DevTools port
    (`window.__TAURI_INTERNALS__.invoke` in the settings page, i.e. what the 「设置 → 热键」 page does).
    Pressing the hotkeys is a checklist for a person, printed at the end: these hotkeys are registered with the
    application (they work while speech recognition is off), and that registration prefers the low-level hook, which
    ignores injected keys by design — `SendInput` can't stand in for the player here.
  * Refuses to start if a game process is running or a game is in front.
  * Isolated: KOTONE_HOME, the WebView2 profile and the exe copy live under target/hotkey-tools-real/; ~/.kotone is
    not read; at the end the script checks that ~/.kotone/config.json did not change. No AI key, no model.
  * Needs node/npx (server-everything is fetched by npx the first time) and node + @playwright/test in apps/desktop.
========================================================================================================================

Part A (no input) — PASS / FAIL lines:
  A1. the stdio server is added the way the plugins page does it (preview → add = trusted) and connects;
      a third-party tool that is not read-only (third-party-action) is picked from her tool list;
  A2. an external agent (`settings.update`, the tool service) adds four bindings: the third-party tool with a FORGED
      `confirmed`, `automation.run {id}`, a page (`ui://kotone/settings/models`), and `history.clear` (set to "deny" in
      her permissions); the forged confirmation is NOT on disk;
  A3. `hotkeys.status`: the third-party one is not registered and says it must be confirmed in 「设置 → 热键」 (its row
      has `confirm`); `automation.run` and the page are registered right away (allowed, no question); the denied one is
      not registered and says why;
  A4. the confirm command (`confirm_hotkey`, what the page's button calls; not a tool) writes `confirmed` =
      ["third-party-action"] and the key gets registered;
  A5. the external agent changes only the key → still confirmed, registered at the new key; it changes the
      arguments (keeping `confirmed`) → the confirmation is gone, not registered; confirmed again through the command;
  A6. the server's config is changed by hand (an env var) and reloaded → the confirmation is cleared (like the trust and
      "always allow"), not registered;
  A7. the plugin is switched off → its hotkey is off (not registered, no error: the key goes to the game);
  A8. isolation: ~/.kotone/config.json unchanged.
Needs a person (CHECKLIST printed at the end).
Not covered: exclusive fullscreen (P1: needs a real game), `game_chat.send` bindings typing into a game
(send_to_game_e2e.py covers the entry).

Prerequisites: target/debug/kotone.exe with the web UI embedded (README 准备: `pnpm build:web`, then
`cargo build -p kotone-tauri --features tauri/custom-protocol`). Takes target/tmp/real.lock.
usage: python -X utf8 -I scripts/manual-e2e/hotkey_tools_real.py [--keep]
"""
import ctypes
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "hotkey-tools-real")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")
EXE = os.path.join(BIN, "kotone.exe")
SRC_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
DEVTOOLS = 9361
PACKAGE = "@modelcontextprotocol/server-everything"
PLAYER_CONFIG = os.path.join(os.path.expanduser("~"), ".kotone", "config.json")
NEEDS_CONFIRM = "要先在「设置 → 热键」确认一次"
RESULTS = []
CHECKLIST = """
Needs a person at the keyboard (the isolated instance with --keep, or your own Kotone with the same bindings):
  a. A hotkey bound to a third-party tool that is NOT confirmed: the key reaches the game / the window in front,
     nothing runs; 「设置 → 热键」 shows the row waiting for confirmation.
  b. After confirming it: press it → the tool runs, one danmaku line with its answer, no voice.
  c. A hotkey bound to automation.run with a deleted automation: press it → she says it failed and where to look.
  d. A hotkey bound to a page (ui://kotone/settings/models), Kotone behind a borderless game or a normal window:
     press it → Kotone comes to front on that page. With a game in exclusive fullscreen: Kotone stays behind,
     the taskbar button flashes, she says the page is open and to switch the game to a borderless window.
  e. Press a confirmed tool hotkey twice quickly while the first is still running (a slow tool): the second press
     does nothing.
  f. Speech recognition stopped: the tool / automation / page hotkeys still work.
"""

ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={DEVTOOLS}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)
user32 = ctypes.WinDLL("user32", use_last_error=True)


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


# ---------- safety ----------

def running_games():
    out = subprocess.run(["tasklist", "/fo", "csv", "/nh"], capture_output=True, text=True, errors="replace").stdout
    names = {line.split('","')[0].strip('"').lower() for line in out.splitlines() if line}
    return sorted(n for n in names if any(g in n for g in GAMES))


def foreground():
    path = foreground_process()
    title = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(user32.GetForegroundWindow(), title, 256)
    return path, title.value


def game_in_front():
    path, _ = foreground()
    return "riot games" in path.lower() or any(g in os.path.basename(path).lower() for g in GAMES)


def guard(where):
    games = running_games()
    if games:
        raise SystemExit(f"STOP ({where}): a game is running ({', '.join(games)})")
    if game_in_front():
        raise SystemExit(f"STOP ({where}): a game is in front")


# ---------- Kotone ----------

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


def config_file():
    with open(os.path.join(HOME, "config.json"), encoding="utf-8") as f:
        return json.load(f)


def entry_on_disk(entry_id):
    for b in (config_file().get("hotkeys") or {}).get("bindings") or []:
        if isinstance(b, dict) and b.get("id") == entry_id:
            return b
    return None


def status():
    env = tools_call("hotkeys.status")
    return env.get("result") or {} if env.get("ok") else {}


def named(st, row):
    """(key it is registered at or None, error) for `custom.<id>` in hotkeys.status named[]."""
    for n in st.get("named") or []:
        if n.get("id") == row:
            return n.get("key"), n.get("error") or ""
    return None, ""


def row_of(st, row):
    return next((b for b in st.get("bindings") or [] if b.get("id") == row), {})


def remote_of(plugins, pid):
    return next((p for p in plugins if (p.get("manifest") or {}).get("id") == pid and p.get("remote")), None)


def connected(pid):
    card = remote_of(invoke("plugins_list").get("ok") or [], pid)
    if card and (card.get("remote") or {}).get("status") == "connected" and card.get("tools"):
        return card
    if card and (card.get("remote") or {}).get("status") == "failed":
        invoke("mcp_server_reconnect", {"id": pid})
    return None


def write_config():
    config = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "legacyUpgrade": "notNeeded"},
        "adminPromptDismissed": True,
        "toolService": {"enabled": True},
        "hotkeyBackend": "register",  # RegisterHotKey reacts to SendInput (part B)
        "assistant": {"speechVolume": 0, "wake": {"enabled": False}},
        # history.clear denied: a hotkey bound to it can't run. No "always allow" rules: third-party tools ask.
        "agent": {"permissions": {"mode": "default", "rules": {"history.clear": "deny", "game.act": "deny",
                                                              "companion.type": "deny"}},
                  "progressSpeech": False},
    }
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, ensure_ascii=False, indent=1)


def update(patch):
    """What an external agent does: settings.update through the tool service."""
    return tools_call("settings.update", {"patch": patch})


def bindings_now():
    """The table as saved (the config file is written on every change)."""
    return (config_file().get("hotkeys") or {}).get("bindings") or []


# ---------- the test ----------

def part_a():
    # A1. add the server like the plugins page (trusted), pick a tool that is not read-only
    paste = json.dumps({"mcpServers": {"everything": {"command": "npx", "args": ["-y", PACKAGE]}}})
    cand = ((invoke("mcp_servers_preview", {"text": paste}).get("ok") or {}).get("servers") or [{}])[0]
    added = invoke("mcp_server_add", {"id": cand.get("id"), "entry": cand.get("entry")})
    card = wait_until(lambda: connected("everything"), 180, 2)
    if not check("A1: server-everything added (trusted) and connected", "error" not in added and card,
                 json.dumps(added, ensure_ascii=False)[:200]):
        return None
    her = (invoke("agent_tools").get("ok") or {}).get("tools") or []
    acting = [t["name"] for t in her if t["name"].startswith("everything.")
              and "third-party-action" in t.get("sideEffects", [])]
    tool = "everything.echo" if "everything.echo" in acting else (acting[0] if acting else None)
    if not check("A1: a third-party tool that is not read-only (asks)", tool, f"{acting}"):
        return None
    args = {"message": "hello from a hotkey"} if tool == "everything.echo" else {}

    # A2. an external agent adds four bindings; the forged confirmation does not stick
    mine = [
        {"key": "F21", "tool": tool, "args": args, "id": "ext", "label": "第三方",
         "confirmed": ["third-party-action"]},
        {"key": "F20", "tool": "automation.run", "args": {"id": "e2e-no-such-automation"}, "id": "auto"},
        {"key": "F19", "page": "ui://kotone/settings/models", "id": "page"},
        {"key": "F18", "tool": "history.clear", "id": "denied"},
    ]
    base = [b for b in bindings_now() if isinstance(b, dict) and b.get("command")]
    env = update({"hotkeys": {"bindings": base + mine}})
    ext = entry_on_disk("ext") or {}
    check("A2: settings.update (external agent) saved the bindings", env.get("ok"),
          json.dumps(env.get("error"), ensure_ascii=False)[:300])
    check("A2: the forged confirmation is not on disk (only 「设置 → 热键」 can confirm)",
          ext.get("tool") == tool and "confirmed" not in ext, json.dumps(ext, ensure_ascii=False))

    # A3. what got registered
    st = wait_until(lambda: (lambda s: s if named(s, "custom.auto")[0] else None)(status()), 15) or status()
    key, error = named(st, "custom.ext")
    check("A3: the third-party one is not registered and says to confirm it",
          key is None and NEEDS_CONFIRM in error and row_of(st, "custom.ext").get("confirm"),
          f"{key} {error} / row confirm {row_of(st, 'custom.ext').get('confirm')}")
    check("A3: automation.run and the page are registered right away (allowed: no question)",
          named(st, "custom.auto")[0] == "F20" and named(st, "custom.page")[0] == "F19",
          f"auto {named(st, 'custom.auto')}, page {named(st, 'custom.page')}")
    key, error = named(st, "custom.denied")
    check("A3: the denied tool is not registered and says why", key is None and "禁止" in error, f"{key} {error}")

    # A4. the confirm command
    confirmed = invoke("confirm_hotkey", {"id": "custom.ext"})
    ext = entry_on_disk("ext") or {}
    st = wait_until(lambda: (lambda s: s if named(s, "custom.ext")[0] else None)(status()), 10) or status()
    check("A4: confirm_hotkey writes confirmed = the tool's side effects",
          "error" not in confirmed and ext.get("confirmed") == ["third-party-action"],
          json.dumps({"r": confirmed.get("error"), "entry": ext}, ensure_ascii=False)[:300])
    check("A4: after confirming it is registered", named(st, "custom.ext")[0] == "F21", f"{named(st, 'custom.ext')}")
    tool_list = [t.get("name") for t in (tools_call("system.capabilities").get("result") or {}).get("tools") or []]
    check("A4: confirming is not a tool (external agents can't call it)",
          not any("confirm" in (n or "") and "hotkey" in (n or "") for n in tool_list))

    # A5. key only → still confirmed; arguments changed → confirmation gone
    def edit(change):
        current = [dict(b) for b in bindings_now() if isinstance(b, dict)]
        for b in current:
            if b.get("id") == "ext":
                change(b)
        return update({"hotkeys": {"bindings": current}})
    edit(lambda b: b.update(key="F16"))
    st = wait_until(lambda: (lambda s: s if named(s, "custom.ext")[0] == "F16" else None)(status()), 10) or status()
    check("A5: only the key changed → still confirmed, registered at F16",
          (entry_on_disk("ext") or {}).get("confirmed") == ["third-party-action"]
          and named(st, "custom.ext")[0] == "F16", f"{named(st, 'custom.ext')}")
    edit(lambda b: b.update(args={**args, "message": "changed"} if args else {"x": 1}))
    st = wait_until(lambda: (lambda s: s if named(s, "custom.ext")[0] is None else None)(status()), 10) or status()
    check("A5: the arguments changed (confirmed kept in the patch) → confirmation gone, not registered",
          "confirmed" not in (entry_on_disk("ext") or {}) and named(st, "custom.ext")[0] is None,
          f"{json.dumps(entry_on_disk('ext'), ensure_ascii=False)} {named(st, 'custom.ext')}")
    edit(lambda b: b.update(args=args))
    again = invoke("confirm_hotkey", {"id": "custom.ext"})
    check("A5: confirmed again through the command", "error" not in again
          and (entry_on_disk("ext") or {}).get("confirmed") == ["third-party-action"])
    return tool


def part_a_config_and_switch():
    # A6. the server's config changes → the confirmation is cleared
    with open(os.path.join(HOME, "mcp.json"), encoding="utf-8") as f:
        data = json.load(f)
    data["mcpServers"]["everything"]["env"] = {"KOTONE_E2E": "changed"}
    with open(os.path.join(HOME, "mcp.json"), "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    reloaded = invoke("mcp_servers_reload")
    cleared = wait_until(lambda: "confirmed" not in (entry_on_disk("ext") or {"confirmed": 1}), 15, 0.5)
    st = wait_until(lambda: (lambda s: s if named(s, "custom.ext")[0] is None else None)(status()), 10) or status()
    check("A6: config changed by hand → the hotkey's confirmation is cleared, not registered",
          "error" not in reloaded and cleared and named(st, "custom.ext")[0] is None,
          f"{json.dumps(entry_on_disk('ext'), ensure_ascii=False)} {named(st, 'custom.ext')}")

    # A7. plugin off → its hotkey is off: not registered, no error (the key goes to the game)
    invoke("update_settings", {"patch": {"plugins": {"everything": {"enabled": False}}}})
    st = wait_until(lambda: (lambda s: s if row_of(s, "custom.ext").get("enabled") is False else None)(status()),
                    15) or status()
    key, error = named(st, "custom.ext")
    check("A7: plugin off → the hotkey is off (not registered, no error)",
          row_of(st, "custom.ext").get("enabled") is False and key is None and not error,
          f"{row_of(st, 'custom.ext').get('enabled')} {key} {error}")


def main():
    keep = "--keep" in sys.argv
    guard("start")
    os.makedirs(os.path.dirname(LOCK), exist_ok=True)
    try:
        os.mkdir(LOCK)
    except FileExistsError:
        raise SystemExit(f"another real-machine test holds {LOCK}")
    app = None
    before = os.stat(PLAYER_CONFIG).st_mtime_ns if os.path.exists(PLAYER_CONFIG) else None
    try:
        if not os.path.exists(SRC_EXE):
            raise SystemExit("no target/debug/kotone.exe - build it first (see the docstring)")
        if os.path.exists(EXE):
            tools_call("app.quit", timeout_ms=3000)
            time.sleep(1)
        shutil.rmtree(WORK, ignore_errors=True)
        for d in (HOME, BIN):
            os.makedirs(d)
        shutil.copy2(SRC_EXE, EXE)
        write_config()
        app = subprocess.Popen([EXE, "--background"], env=ENV)
        if not wait_until(lambda: tools_call("app.status", timeout_ms=3000).get("ok"), 90):
            raise SystemExit("the isolated instance's tool service did not come up")
        if not wait_until(lambda: "ok" in invoke("agent_state"), 90):
            raise SystemExit("the settings page did not come up (DevTools; embedded web UI built?)")
        if part_a():
            part_a_config_and_switch()
    finally:
        if app is not None:
            tools_call("app.quit", timeout_ms=5000)
            try:
                app.wait(20)
            except subprocess.TimeoutExpired:
                app.kill()
        if not keep:
            time.sleep(1)
            shutil.rmtree(WORK, ignore_errors=True)
        os.rmdir(LOCK)
    after = os.stat(PLAYER_CONFIG).st_mtime_ns if os.path.exists(PLAYER_CONFIG) else None
    check("A8: isolation: ~/.kotone/config.json unchanged", after == before)
    passed = sum(1 for _, ok in RESULTS if ok)
    print(f"\n{passed}/{len(RESULTS)} passed")
    print(CHECKLIST)
    sys.exit(0 if passed == len(RESULTS) else 1)


if __name__ == "__main__":
    main()
