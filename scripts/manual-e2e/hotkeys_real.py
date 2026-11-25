"""Real-machine check for hotkeys (ADR 040 step 4): run it before and after the talk key (说话) leaves the main-key
slot. It goes through the real hotkey registration → Kotone → voice input plugin path with keys pressed by SendInput.

========================================================================================================================
SAFETY — this script PRESSES KEYS and once lets Kotone TYPE into a test window. Read before running.
  * Keys pressed: F19-F22 only (no keyboard has them; F23 is the console-channel trigger, F24 the hook probe), each
    only while the test window (target_window.ps1 -Chat, title KotoneScriptTarget) is in front. Esc combinations are
    never pressed (Ctrl+Esc opens Start, Alt+Esc switches windows, Ctrl+Shift+Esc opens Task Manager) — they are only
    checked through the registration status. It prints:
        会按 F19-F22，还会往测试窗口里打两条字，测试期间别碰键盘鼠标（约 2 分钟）
  * Refuses to start if a game process (voice_turn_real.GAMES, Steam included) is running or the foreground is not a
    terminal / the test window (SAFE_FOREGROUND below). Before every key press the test window is brought to front
    (fg.ps1) and checked; while waiting for Kotone the foreground is watched: anything else in front →
    `runtime.stop` (nothing gets typed) and the script exits. Clean-up releases held F-keys only when no game is in
    front.
  * Isolated: KOTONE_HOME, the WebView2 profile and the test window live under target/hotkeys-real/; the exe is a
    copy of target/debug/kotone.exe in target/hotkeys-real/bin/. ~/.kotone is not read; at the end the script checks
    that ~/.kotone/config.json did not change. No microphone: speech recognition is the built-in `mock-stream`
    engine (fixed text “对面打野在下路”) recording VB-CABLE's “CABLE Output” (silence). No AI key, no credentials.
    Her input tools (game_chat.send, game.act, automation.run, companion.type) are denied.
  * Clean-up (always): runtime.stop, app.quit, close the test window.
========================================================================================================================

How keys reach Kotone: the default hotkey backend is the low-level hook, which ignores injected input by design. The
isolated config sets `hotkeyBackend: "register"` (RegisterHotKey through tauri-plugin-global-shortcut), which does
react to SendInput, so `SendInput(F22)` is the player pressing the talk key (as voice_input_real.py does). A key that
Kotone took never reaches the test window (no `DOWN F22` line in its log); a key Kotone let through does.
Voice input runs as hold + preview (`interactionMode: null`, `hotkey.mode: "hold"`, `autoSend: false`): holding the
talk key listens, releasing ends the sentence and shows the preview; the script cancels the preview (`cancel_session`,
the floating window's close) except once, where it confirms it (`confirm_send`) to have something for 重发.

The hotkey table (`hotkeys.bindings`, ADR 040 D8) is written into the isolated config before start:
  说话 voice-input.talk = F22, 频道切换 voice-input.channel-cycle = F20, 重发最近一条 voice-input.resend-last = F19,
  文字处理开关 voice-input.post-process-toggle = Shift+Escape (as an older version could have saved it).

Checks (PASS / FAIL / SKIP lines, total at the end):
  1. start: the RegisterHotKey backend is active; talk, channel cycle and resend are registered at their keys;
     the Esc combination is NOT registered and its status says why (“Esc 留给停下她和取消收音”), the row stays in the
     table (shown red on the page);
  2. talk key hold / release (F22): holding starts listening, it keeps listening while held, releasing ends the
     sentence (transcribing → preview); F22 never reached the test window, nothing typed;
  3. 重发 (F19): a confirmed preview types “对面打野在下路” into the test window (Enter opens, Enter sends); then F19
     types the same sentence again; F19 never reached the test window;
  4. 频道切换 (F20) with a two-channel test profile selected: each press switches channel (队伍 → 所有人 → 队伍, seen as
     `kotone://channel` events through WebView2's DevTools port); F20 never reached the test window;
  5. a key changed in the table takes effect at once: `set_hotkey` (what 「设置 → 热键」 calls) moves talk to F21 —
     registered right away, written to the table and the old `hotkey.key` mirror; F22 now goes to the test window and
     starts nothing; F21 hold / release works as in 2; the Esc row that was already there did not block this change;
  6. conflicts are not saved: `hotkeys.conflicts` reports F21 as talk's; 重发 → F21 is refused through the table
     (`set_hotkey`), the old field (`settings.update resendLastHotkey`) and a whole-table replacement
     (`settings.update hotkeys.bindings`); 频道切换 → Ctrl+Escape is refused with the Esc reason; the config file still
     has F19 / F20 and 重发 is still registered at F19;
  7. speech recognition not running (`runtime.stop`): talk / 频道切换 / 重发 are no longer registered, their passive
     hints are armed on the low-level hook (kotone.log “llhook passive hint hint:record …”), and F21 / F20 now reach
     the test window and start nothing (not swallowed). The hint itself (bubble / floating window) needs a REAL press —
     the low-level hook ignores injected keys — so it is a SKIP line and part of the checklist printed at the end;
  8. isolation: ~/.kotone/config.json unchanged.
Not covered (needs a person at the keyboard, see the checklist printed at the end and ADR 040's implementation
record): the low-level hook backend (the default) — hold / release, a combination released modifier-first, the hint
when recognition isn't running, modifier-mismatch diagnostics, mouse side buttons, the WebView in front.

Prerequisites: target/debug/kotone.exe with the web UI embedded (README 准备: `pnpm build:web`, then
`cargo build -p kotone-tauri --features tauri/custom-protocol`), copied to target/hotkeys-real/bin/ on start; VB-CABLE
(“CABLE Output”); node + @playwright/test in apps/desktop. If the repo folder carries the Low integrity label, pass
`--exe` with a copy outside the repo (README 准备 step 2), otherwise Windows drops the injected keys.
usage: python -X utf8 -I scripts/manual-e2e/hotkeys_real.py [--exe path\\to\\kotone.exe]
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import os
import re
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "hotkeys-real")
HOME = os.path.join(WORK, "home")
WINDOW_DIR = os.path.join(WORK, "window")
BIN = os.path.join(WORK, "bin", "kotone.exe")  # a copy, so a running test instance is never taken for `pnpm dev`
DEBUG_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
DEVTOOLS = 9344
PLAYER_CONFIG = os.path.join(os.path.expanduser("~"), ".kotone", "config.json")  # never read, only its mtime
CABLE_OUTPUT = "CABLE Output (VB-Audio Virtual Cable)"
MOCK_TEXT = "对面打野在下路"  # the mock-stream engine's fixed final text
TEST_TITLE = "KotoneScriptTarget"
BANNER = "会按 F19-F22，还会往测试窗口里打两条字，测试期间别碰键盘鼠标（约 2 分钟）"
SAFE_FOREGROUND = {"powershell.exe", "pwsh.exe", "windowsterminal.exe", "openconsole.exe", "conhost.exe", "cmd.exe",
                   "warp.exe", "code.exe", "claude.exe", "wezterm-gui.exe", "alacritty.exe", "mintty.exe"}
RULES = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}

VK = {"F19": 0x82, "F20": 0x83, "F21": 0x84, "F22": 0x85}
TALK, TALK_NEW, CHANNEL, RESEND = "F22", "F21", "F20", "F19"
ESC_COMBO = "Shift+Escape"  # never pressed
ESC_REASON = "Esc 留给停下她和取消收音"
CONFLICT_WORDS = "已经是"  # “「F21」已经是「…」的热键：先在「设置 → 热键」把它换掉，或者换一个键。”
CMD_TALK = "voice-input.talk"
CMD_CHANNEL = "voice-input.channel-cycle"
CMD_RESEND = "voice-input.resend-last"
CMD_PP_TOGGLE = "voice-input.post-process-toggle"
# `hotkeys.status` `named[].id` keeps the phase-1 ids (ADR 040 implementation record, revision 4)
STATUS_ID = {CMD_TALK: "record", CMD_CHANNEL: "channel-cycle", CMD_RESEND: "resend-last",
             CMD_PP_TOGGLE: "post-process-toggle"}
TABLE = [
    {"key": TALK, "command": CMD_TALK},
    {"key": CHANNEL, "command": CMD_CHANNEL},
    {"key": RESEND, "command": CMD_RESEND},
    {"key": ESC_COMBO, "command": CMD_PP_TOGGLE},
]
CHANNEL_PROFILE = {
    "id": "e2e-channels", "displayName": "测试：两个频道（没有游戏）",
    "processNames": ["kotone-e2e-no-such-game.exe"], "windowTitlePatterns": [],
    "openChatKey": "Enter", "sendKey": "Enter", "preOpenDelayMs": 20, "prePasteDelayMs": 20, "preSendDelayMs": 100,
    "preferClipboardPaste": False, "hotwords": [],
    "channels": [{"id": "team", "displayName": "队伍", "default": True},
                 {"id": "all", "displayName": "所有人", "openChatKey": "Shift+Enter"}],
}
CHECKLIST = """
Needs a real press (the low-level hook ignores injected keys) — default backend, Kotone's own config:
  a. 说话 = CapsLock, push-to-talk: hold → listening, release → sent; CapsLock's light does not change.
  b. 说话 = a combination (e.g. Ctrl+Space): hold, let go of Ctrl FIRST, keep Space down a second, then let go:
     still listening until Space is released, then it ends; no space typed into the game / chat; nothing else fires.
  c. Speech recognition stopped: press 说话 / 呼叫键 / 频道切换 → the floating window / bubble says it isn't ready,
     and the key still reaches the game (CapsLock toggles, the letter is typed).
  d. Press a hotkey with an extra modifier (Shift+CapsLock, Ctrl+F-key): it goes to the game; the process log
     (diagnostics) has a `hotkey_fired` line with MainKeyMissed / NamedKeyMissed (after step 4: NamedKeyMissed).
  e. Mouse4 / Mouse5 as 说话: hold / release like a.
  f. Change 说话 in 「设置 → 热键」 while recognition runs: the new key works at once, the old one goes to the game.
  g. A row set to an Esc combination is red with the reason; Esc during a sentence still cancels it.
  h. Kotone's settings window in front (WebView focused): 说话 still works (0.1.6: CapsLock fell into the page).
"""

EXE = BIN
ENV = {}
RESULTS = []  # (name, "PASS" | "FAIL" | "SKIP")
HELD = set()

user32 = ctypes.WinDLL("user32", use_last_error=True)


# ---------- results ----------

def check(name, ok, detail=""):
    RESULTS.append((name, "PASS" if ok else "FAIL"))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def skip(name, reason):
    RESULTS.append((name, "SKIP"))
    print(f"SKIP {name} — {reason}", flush=True)


# ---------- safety (from voice_input_real.py) ----------

def running_games():
    out = subprocess.run(["tasklist", "/fo", "csv", "/nh"], capture_output=True, text=True, errors="replace").stdout
    names = {line.split('","')[0].strip('"').lower() for line in out.splitlines() if line}
    return sorted(n for n in names if any(g in n for g in GAMES))


def foreground():
    """(exe path, window title) of the foreground window."""
    path = foreground_process()
    title = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(user32.GetForegroundWindow(), title, 256)
    return path, title.value


def in_front():
    return foreground()[1] == TEST_TITLE


def game_in_front():
    path, _ = foreground()
    name = os.path.basename(path).lower()
    return "riot games" in path.lower() or any(g in name for g in GAMES)


def guard(where):
    games = running_games()
    if games:
        raise SystemExit(f"STOP ({where}): a game is running ({', '.join(games)}) - not sending any input.")
    path, title = foreground()
    name = os.path.basename(path)
    if game_in_front() or (title != TEST_TITLE and name.lower() not in SAFE_FOREGROUND):
        raise SystemExit(f"STOP ({where}): '{title}' ({name or 'unknown process'}) is in front - only a terminal "
                         "or the test window may be.")


def focus(where):
    """Bring the test window to front (only from a terminal / the test window) and make sure it is there."""
    guard(where)
    if not in_front():
        out = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                              os.path.join(HERE, "fg.ps1"), "-Title", TEST_TITLE], capture_output=True, text=True)
        time.sleep(0.4)
        if "True" not in out.stdout:
            raise SystemExit(f"STOP ({where}): could not bring the test window to front: {out.stdout.strip()} "
                             f"{out.stderr.strip()[:200]} - not pressing keys.")
    if not in_front():
        raise SystemExit(f"STOP ({where}): the test window is not in front ({foreground()}) - not pressing keys.")


def emergency_stop(why):
    """Something else came to front: stop voice input (cancels the sentence) before anything gets typed."""
    call_raw("runtime.stop", timeout_ms=5000)
    raise SystemExit(f"STOP: {why} - stopped Kotone's voice input so nothing gets typed.")


def watch_foreground():
    if not in_front():
        emergency_stop(f"the test window lost the foreground to {foreground()}")


def pause(seconds):
    """Wait while watching the foreground."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        watch_foreground()
        time.sleep(0.1)


# ---------- keys (from voice_input_real.py) ----------

class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def send_key(vk, up):
    i = INPUT(type=1)
    i.ki = KEYBDINPUT(vk, user32.MapVirtualKeyW(vk, 0), 2 if up else 0, 0, 0)
    sent = user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))
    (HELD.discard if up else HELD.add)(vk)
    if sent != 1:
        raise RuntimeError(f"SendInput failed ({ctypes.get_last_error()})")


def key_down(key):
    """Press one of F19-F22, only with the test window in front."""
    if not in_front():
        emergency_stop(f"the test window is not in front before pressing {key}")
    send_key(VK[key], False)


def key_up(key):
    send_key(VK[key], True)


def tap(key):
    key_down(key)
    time.sleep(0.05)
    key_up(key)


def release_held():
    """Clean-up: never leave an F-key held down; skipped when a game is in front."""
    for vk in list(HELD):
        if game_in_front():
            print(f"WARN not releasing VK {vk:#x}: a game is in front")
            continue
        send_key(vk, True)


# ---------- Kotone: tool service (`kotone.exe tools call`, arguments on stdin) and DevTools port ----------

def call_raw(tool, args=None, timeout_ms=30_000):
    if not ENV:
        return {"ok": False, "error": {"code": "NOT_STARTED"}}
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", "-", "--timeout-ms", str(timeout_ms)],
                         input=json.dumps(args if args is not None else {}, ensure_ascii=False),
                         capture_output=True, text=True, encoding="utf-8", env=ENV)
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return {"ok": False, "error": {"code": "NO_JSON", "message": (out.stdout + out.stderr)[-400:]}}


def call(tool, args=None, timeout_ms=30_000):
    envelope = call_raw(tool, args, timeout_ms)
    if not envelope.get("ok"):
        raise RuntimeError(f"{tool}: {json.dumps(envelope.get('error'), ensure_ascii=False)[:400]}")
    return envelope["result"]


def error_text(envelope):
    return json.dumps(envelope.get("error"), ensure_ascii=False)


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


def page_eval(body):
    """Run `body` (the inside of an async JS function) in the settings page through WebView2's DevTools port."""
    code = PAGE_EVAL.replace("__PORT__", str(DEVTOOLS)).replace("__BODY__", body)
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        raise RuntimeError((out.stdout + out.stderr)[-1500:])


def invoke(command, args=None):
    """A Tauri command as the page calls it: {"ok": result} or {"error": "..."}."""
    return page_eval("return await window.__TAURI_INTERNALS__.invoke(%s, %s);"
                     % (json.dumps(command), json.dumps(args or {}, ensure_ascii=False)))


# Channel switches: record every `kotone://channel` event in the settings page (what the floating window shows).
CHANNEL_RECORDER = """
const I = window.__TAURI_INTERNALS__;
if (window.__hotkeysE2E) return "already";
const seen = [];
await I.invoke("plugin:event|listen", { event: "kotone://channel", target: { kind: "Any" },
  handler: I.transformCallback((e) => { seen.push(e.payload || {}); }) });
window.__hotkeysE2E = seen;
return "installed";
"""


def channels_seen():
    return page_eval("return window.__hotkeysE2E || null;").get("ok") or []


# ---------- files: kotone.log, config, test window ----------

LOG = os.path.join(HOME, "kotone.log")
CONFIG = os.path.join(HOME, "config.json")


def size(path):
    return os.path.getsize(path) if os.path.exists(path) else 0


def read_from(path, mark):
    if not os.path.exists(path):
        return ""
    with open(path, "rb") as f:
        if os.path.getsize(path) >= mark:
            f.seek(mark)
        return f.read().decode("utf-8", errors="replace")


def wait_log(mark, pattern, timeout, watch=True):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if watch:
            watch_foreground()
        m = re.search(pattern, read_from(LOG, mark))
        if m:
            return m
        time.sleep(0.2)
    return None


def states(mark):
    out = []
    for s in re.findall(r"state -> (\w+)", read_from(LOG, mark)):
        if not out or out[-1] != s:
            out.append(s)
    return out


def config_on_disk():
    with open(CONFIG, encoding="utf-8") as f:
        return json.load(f)


def table_key(config, command):
    for b in (config.get("hotkeys") or {}).get("bindings") or []:
        if isinstance(b, dict) and b.get("command") == command:
            return b.get("key")
    return None


def window_lines():
    try:
        with open(os.path.join(WINDOW_DIR, "events.log"), encoding="utf-8", errors="replace") as f:
            return f.read().splitlines()
    except FileNotFoundError:
        return []


def parse(lines):
    """target_window.ps1 -Chat lines → events (from input_gate_e2e.py)."""
    out = []
    for line in lines:
        ts, _, rest = line.partition(" ")
        parts = rest.split()
        if not parts or not ts.isdigit():
            continue
        ev = {"ts": int(ts), "kind": parts[0], "raw": rest}
        if parts[0] in ("DOWN", "UP"):
            ev["key"] = parts[1] if len(parts) > 1 else ""
            for p in parts[2:]:
                k, _, v = p.partition("=")
                ev[k] = v
        elif parts[0] == "CHAR" and len(parts) > 1:
            ev["unit"] = int(parts[1], 16)
        out.append(ev)
    return out


def chat(events):
    """What a chat box would have received: [(channel, text)] (Enter opens, characters go in, Enter sends)."""
    messages, open_channel, units = [], None, []
    for ev in events:
        if ev["kind"] == "DOWN" and ev.get("key") == "Return":
            if open_channel is None:
                open_channel, units = ("all" if ev.get("shift") == "True" else "team"), []
            else:
                messages.append((open_channel, b"".join(u.to_bytes(2, "little") for u in units).decode("utf-16-le")))
                open_channel = None
        elif ev["kind"] == "CHAR" and open_channel is not None and ev["unit"] >= 0x20:
            units.append(ev["unit"])
    return messages


def window_since(mark):
    events = parse(window_lines()[mark:])
    return {"downs": [e["key"] for e in events if e["kind"] == "DOWN"],
            "chars": sum(1 for e in events if e["kind"] == "CHAR"),
            "messages": chat(events)}


# ---------- hotkey status ----------

def status():
    return call("hotkeys.status")


def registered_at(st, command):
    """The key a command is registered at right now (None = not registered): talk may sit in the main-key slot
    (before step 4) or in `named[]` (after)."""
    sid = STATUS_ID.get(command, command)
    for n in st.get("named") or []:
        if n.get("id") in (sid, command) and n.get("key"):
            return n["key"]
    if command == CMD_TALK and st.get("registered") and st.get("key"):
        return st["key"]
    return None


def named_error(st, command):
    sid = STATUS_ID.get(command, command)
    for n in st.get("named") or []:
        if n.get("id") in (sid, command):
            return n.get("error") or ""
    return ""


def row_key(st, command):
    for b in st.get("bindings") or []:
        if b.get("command") == command:
            return b.get("key")
    return None


# ---------- one sentence on the talk key ----------

def talk(key, where):
    """Hold `key`, wait for listening, keep it down 1.5 s, release; the preview is cancelled. Nothing is typed."""
    focus(where)
    log0, win0 = size(LOG), len(window_lines())
    key_down(key)
    held_states, released = [], None
    try:
        listening = wait_log(log0, r"state -> listening\b", 10)
        if listening:
            pause(1.5)
            held_states = states(log0)
    finally:
        if VK[key] in HELD and in_front():
            released = size(LOG)
            key_up(key)
    ended = wait_log(released, r"state -> (preview|error|idle)\b", 20) if (listening and released is not None) else None
    after = states(released) if released is not None else []
    if ended and ended.group(1) in ("preview", "error"):
        invoke("cancel_session")
    wait_log(log0, r"state -> idle\b", 10)
    time.sleep(0.6)  # the test window logs asynchronously
    return {"listening": bool(listening), "held_states": held_states, "after": after,
            "ended": ended.group(1) if ended else None, "window": window_since(win0)}


def expect_talk(step, key, s):
    check(f"{step}: holding {key} starts listening", s["listening"], f"{s['held_states']}")
    check(f"{step}: still listening while {key} is held (nothing ended before the release)",
          s["listening"] and s["held_states"] and s["held_states"][-1] == "listening", f"{s['held_states']}")
    check(f"{step}: releasing {key} ends the sentence (→ transcribing → preview)",
          "transcribing" in s["after"] and s["ended"] == "preview", f"after release {s['after']}, ended {s['ended']}")
    check(f"{step}: {key} never reached the test window and nothing was typed",
          key not in s["window"]["downs"] and s["window"]["chars"] == 0, f"{s['window']}")


def passes_through(key, where, wait=2.0):
    """Tap `key`: True if the test window got it and Kotone started nothing."""
    focus(where)
    log0, win0 = size(LOG), len(window_lines())
    tap(key)
    pause(wait)
    w = window_since(win0)
    started = re.search(r"state -> listening\b|named hotkey fired", read_from(LOG, log0))
    return key in w["downs"] and not started, f"window {w['downs']}, kotone {started.group(0) if started else '-'}"


# ---------- setup / clean-up ----------

def write_config():
    config = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "legacyUpgrade": "notNeeded"},
        "adminPromptDismissed": True,
        "toolService": {"enabled": True},
        "models": {"dir": os.path.join(WORK, "models")},  # empty: the mock engine needs no model
        "sttEngine": "mock-stream",
        "audioDeviceId": CABLE_OUTPUT,
        "hotkeyBackend": "register",
        # hold + preview: release ends the sentence, nothing is typed until confirmed
        "interactionMode": None,
        "hotkey": {"key": TALK, "mode": "hold"},
        "autoSend": False,
        "activeProfileId": "generic",
        "hotkeys": {"bindings": TABLE},
        "soundFeedback": {"record": {"enabled": False}, "send": {"enabled": False}},
        "postProcessing": {"enabled": False},
        "assistant": {"speechVolume": 0, "wake": {"enabled": False}},
        "agent": {"permissions": {"mode": "default", "rules": RULES}, "progressSpeech": False},
    }
    with open(CONFIG, "w", encoding="utf-8") as f:
        json.dump(config, f, ensure_ascii=False, indent=1)


def prepare_exe(explicit):
    if explicit:
        return os.path.abspath(explicit)
    if not os.path.exists(DEBUG_EXE) and not os.path.exists(BIN):
        raise SystemExit("no target/debug/kotone.exe - build it first (README 准备: pnpm build:web, then "
                         "cargo build -p kotone-tauri --features tauri/custom-protocol)")
    if os.path.exists(DEBUG_EXE) and (not os.path.exists(BIN) or os.path.getmtime(DEBUG_EXE) > os.path.getmtime(BIN)):
        os.makedirs(os.path.dirname(BIN), exist_ok=True)
        shutil.copy2(DEBUG_EXE, BIN)
    return BIN


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


def player_mtime():
    return os.stat(PLAYER_CONFIG).st_mtime_ns if os.path.exists(PLAYER_CONFIG) else None


# ---------- the test ----------

def main():
    global EXE, ENV
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", help="kotone.exe to test (default: a copy of target/debug/kotone.exe in "
                                      "target/hotkeys-real/bin)")
    opts = parser.parse_args()

    print("=" * 72)
    print("  " + BANNER)
    print("  (Ctrl+C within 5 s to cancel)")
    print("=" * 72, flush=True)
    guard("start")
    time.sleep(5)
    guard("start")

    EXE = prepare_exe(opts.exe)
    ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
               WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={DEVTOOLS}")
    ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
    # a test instance / window left over from an interrupted run
    call_raw("app.quit", timeout_ms=3000)
    if os.path.isdir(WINDOW_DIR):
        with open(os.path.join(WINDOW_DIR, "control.txt"), "w") as f:
            f.write("close")
    time.sleep(2)
    for d in (HOME, WINDOW_DIR, os.path.join(WORK, "webview")):
        shutil.rmtree(d, ignore_errors=True)
    os.makedirs(HOME, exist_ok=True)
    os.makedirs(WINDOW_DIR, exist_ok=True)
    os.makedirs(os.path.join(WORK, "models"), exist_ok=True)
    write_config()
    before = player_mtime()

    app = window = None
    started = time.time()
    try:
        app = subprocess.Popen([EXE, "--background"], env=ENV)
        if not wait_until(lambda: call_raw("app.status", timeout_ms=3000).get("ok"), 90):
            raise SystemExit("the isolated instance's tool service did not come up")
        if not wait_until(lambda: "ok" in invoke("agent_state"), 90):
            raise SystemExit("the settings page is not reachable on the DevTools port (embedded web UI built?)")
        print("kotone://channel recorder:", page_eval(CHANNEL_RECORDER))

        devices = call("audio.devices").get("devices", [])
        cable = next((d["id"] for d in devices if str(d.get("id", "")).startswith("CABLE Output")), None)
        if not cable:
            raise SystemExit(f"VB-CABLE “CABLE Output” not found among {[d.get('id') for d in devices]} - "
                             "not recording a real microphone")
        if cable != CABLE_OUTPUT:
            call("settings.update", {"patch": {"audioDeviceId": cable}})
        rt = call("runtime.start", timeout_ms=30_000)
        if rt.get("phase") != "running":
            rt = wait_until(lambda: (lambda s: s if s.get("phase") == "running" else None)(call("runtime.status")),
                            60, 1) or call("runtime.status")
        if rt.get("phase") != "running":
            raise SystemExit(f"speech recognition (mock-stream) did not start: {rt}")

        window = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                                   os.path.join(HERE, "target_window.ps1"), "-Dir", WINDOW_DIR, "-Chat"])
        if not wait_until(lambda: os.path.exists(os.path.join(WINDOW_DIR, "ready.txt")), 30, 0.25):
            raise SystemExit("the test window did not come up")

        # 1. start: what got registered
        print("\n== 1. start ==")
        st = wait_until(lambda: (lambda s: s if registered_at(s, CMD_TALK) == TALK else None)(status()), 10) or status()
        named = [(n.get("id"), n.get("key"), n.get("error")) for n in st.get("named") or []
                 if n.get("key") or n.get("error")]
        print(f"hotkeys: backend {st.get('backend')}, main {st.get('key')}, named {named}")
        if st.get("backend") != "register":
            raise SystemExit(f"the RegisterHotKey backend is not active ({st.get('backend')}) - injected keys would "
                             "not reach Kotone, nothing to test")
        check("1: talk, 频道切换 and 重发 registered at F22 / F20 / F19",
              [registered_at(st, c) for c in (CMD_TALK, CMD_CHANNEL, CMD_RESEND)] == [TALK, CHANNEL, RESEND],
              f"{[registered_at(st, c) for c in (CMD_TALK, CMD_CHANNEL, CMD_RESEND)]}")
        check("1: the Esc combination is not registered and says why",
              registered_at(st, CMD_PP_TOGGLE) is None and ESC_REASON in named_error(st, CMD_PP_TOGGLE),
              f"{registered_at(st, CMD_PP_TOGGLE)} / {named_error(st, CMD_PP_TOGGLE)}")
        check("1: the Esc row stays in the table (shown red, not dropped)", row_key(st, CMD_PP_TOGGLE) == ESC_COMBO,
              f"{row_key(st, CMD_PP_TOGGLE)}")

        # 2. talk key hold / release
        print("\n== 2. talk key hold / release (F22) ==")
        expect_talk("2", TALK, talk(TALK, "2"))

        # 3. 重发: one confirmed sentence, then F19 sends it again
        print("\n== 3. 重发最近一条 (F19) ==")
        focus("3a")
        log0, win0 = size(LOG), len(window_lines())
        key_down(TALK)
        try:
            listening = wait_log(log0, r"state -> listening\b", 10)
            if listening:
                pause(1.0)
        finally:
            if VK[TALK] in HELD and in_front():
                key_up(TALK)
        preview = wait_log(log0, r"state -> (preview|error)\b", 20) if listening else None
        sent = None
        if preview and preview.group(1) == "preview":
            focus("3a (confirm)")
            invoke("confirm_send")
            sent = wait_log(log0, r"state -> (success|error)\b", 20)
            wait_log(log0, r"state -> idle\b", 10)
            time.sleep(0.6)
        first = window_since(win0)
        check("3: a confirmed sentence is typed once", first["messages"] == [("team", MOCK_TEXT)],
              f"{first['messages']} ({sent.group(1) if sent else 'no send'})")
        focus("3b")
        log0, win0 = size(LOG), len(window_lines())
        tap(RESEND)
        fired = wait_log(log0, r"named hotkey fired: resend-last", 5)
        resent = wait_log(log0, r"state -> (success|error)\b", 15)
        wait_log(log0, r"state -> idle\b", 10)
        time.sleep(0.6)
        again = window_since(win0)
        check("3: F19 sends the same sentence again", bool(fired) and again["messages"] == [("team", MOCK_TEXT)],
              f"fired {bool(fired)}, {again['messages']} ({resent.group(1) if resent else 'no send'})")
        check("3: F19 never reached the test window", RESEND not in again["downs"], f"{again['downs']}")

        # 4. 频道切换 with a two-channel profile
        print("\n== 4. 频道切换 (F20) ==")
        call("profiles.save", {"profile": CHANNEL_PROFILE})
        call("settings.update", {"patch": {"activeProfileId": CHANNEL_PROFILE["id"]}})
        switched, downs = [], []
        for i in range(2):
            focus(f"4.{i + 1}")
            log0, win0, seen0 = size(LOG), len(window_lines()), len(channels_seen())
            tap(CHANNEL)
            fired = wait_log(log0, r"named hotkey fired: channel-cycle", 5)
            got = wait_until(lambda: channels_seen()[seen0:], 5, 0.25) or []
            switched.append((bool(fired), [c.get("channelId") for c in got]))
            downs += window_since(win0)["downs"]
        check("4: each F20 press switches channel: 所有人, then back to 队伍",
              switched == [(True, ["all"]), (True, ["team"])], f"{switched}")
        check("4: F20 never reached the test window", CHANNEL not in downs, f"{downs}")
        call("settings.update", {"patch": {"activeProfileId": "generic"}})
        call("profiles.delete", {"id": CHANNEL_PROFILE["id"]})

        # 5. change the talk key in the table → takes effect at once
        print("\n== 5. change 说话 to F21 in the table ==")
        changed = invoke("set_hotkey", {"id": CMD_TALK, "key": TALK_NEW})
        check("5: set_hotkey (「设置 → 热键」) saves F21 for 说话, although an Esc row is already in the table",
              "ok" in changed, f"{changed.get('error', '')[:300]}")
        st = wait_until(lambda: (lambda s: s if registered_at(s, CMD_TALK) == TALK_NEW else None)(status()), 10) \
            or status()
        check("5: registered at F21 right away", registered_at(st, CMD_TALK) == TALK_NEW,
              f"{registered_at(st, CMD_TALK)}")
        disk = config_on_disk()
        check("5: written to the table and the old hotkey.key mirror",
              table_key(disk, CMD_TALK) == TALK_NEW and (disk.get("hotkey") or {}).get("key") == TALK_NEW,
              f"table {table_key(disk, CMD_TALK)}, hotkey.key {(disk.get('hotkey') or {}).get('key')}")
        ok, detail = passes_through(TALK, "5 (old key)")
        check("5: the old key F22 now goes to the test window and starts nothing", ok, detail)
        expect_talk("5", TALK_NEW, talk(TALK_NEW, "5"))

        # 6. conflicts are not saved
        print("\n== 6. conflicts are not saved ==")
        conflicts = call("hotkeys.conflicts", {"key": TALK_NEW, "except": CMD_RESEND}).get("conflicts") or []
        check("6: hotkeys.conflicts says F21 is 说话's", any(c.get("command") == CMD_TALK for c in conflicts),
              f"{[(c.get('id'), c.get('command')) for c in conflicts]}")
        refused = invoke("set_hotkey", {"id": CMD_RESEND, "key": TALK_NEW})
        check("6: 重发 → F21 through the table is refused, saying who has it",
              "error" in refused and CONFLICT_WORDS in refused["error"], f"{refused}"[:300])
        env = call_raw("settings.update", {"patch": {"resendLastHotkey": TALK_NEW}})
        check("6: 重发 → F21 through the old field is refused", not env.get("ok") and CONFLICT_WORDS in error_text(env),
              error_text(env)[:300])
        bindings = [dict(b) for b in (config_on_disk().get("hotkeys") or {}).get("bindings") or []]
        for b in bindings:
            if b.get("command") == CMD_RESEND:
                b["key"] = TALK_NEW
        env = call_raw("settings.update", {"patch": {"hotkeys": {"bindings": bindings}}})
        check("6: 重发 → F21 through a whole-table replacement is refused",
              not env.get("ok") and CONFLICT_WORDS in error_text(env), error_text(env)[:300])
        refused = invoke("set_hotkey", {"id": CMD_CHANNEL, "key": "Ctrl+Escape"})
        check("6: 频道切换 → Ctrl+Escape is refused with the Esc reason",
              "error" in refused and ESC_REASON in refused["error"], f"{refused}"[:300])
        disk, st = config_on_disk(), status()
        check("6: nothing refused was saved; 重发 still registered at F19",
              table_key(disk, CMD_RESEND) == RESEND and table_key(disk, CMD_CHANNEL) == CHANNEL
              and registered_at(st, CMD_RESEND) == RESEND,
              f"table 重发 {table_key(disk, CMD_RESEND)} / 频道切换 {table_key(disk, CMD_CHANNEL)}, "
              f"registered 重发 {registered_at(st, CMD_RESEND)}")

        # 7. speech recognition not running: hint only, keys go through
        print("\n== 7. speech recognition stopped ==")
        log0 = size(LOG)
        call("runtime.stop", timeout_ms=10_000)
        stopped = wait_until(lambda: (lambda s: s if s.get("phase") != "running" else None)(call("runtime.status")),
                             20, 0.5)
        st = wait_until(lambda: (lambda s: s if registered_at(s, CMD_TALK) is None else None)(status()), 10) or status()
        check("7: stopped; 说话 / 频道切换 / 重发 no longer registered",
              bool(stopped) and [registered_at(st, c) for c in (CMD_TALK, CMD_CHANNEL, CMD_RESEND)] == [None] * 3,
              f"{(stopped or {}).get('phase')} {[registered_at(st, c) for c in (CMD_TALK, CMD_CHANNEL, CMD_RESEND)]}")
        armed = wait_until(lambda: all(re.search(rf"llhook passive hint hint:{sid}: Some", read_from(LOG, log0))
                                       for sid in ("record", "channel-cycle", "resend-last")), 10, 0.5)
        hint_log = re.findall(r"(?:llhook passive hint|提示键|没准备好的提示键)[^\r\n]*", read_from(LOG, log0))
        check("7: their hints are armed on the low-level hook (only watch, never swallow)", bool(armed),
              f"{hint_log[:6]}")
        for key in (TALK_NEW, CHANNEL):
            ok, detail = passes_through(key, f"7 ({key})")
            check(f"7: {key} goes to the test window and starts nothing", ok, detail)
        skip("7: the hint itself (floating window / bubble)",
             "needs a real press - the low-level hook ignores injected keys (checklist c)")
    finally:
        print("\n== clean-up ==")
        release_held()
        if app is not None:
            call_raw("runtime.stop", timeout_ms=10_000)
            print("app.quit:", call_raw("app.quit", timeout_ms=5000).get("ok"))
            try:
                app.wait(20)
            except subprocess.TimeoutExpired:
                app.kill()
        if window is not None:
            with open(os.path.join(WINDOW_DIR, "control.txt"), "w") as f:
                f.write("close")
            try:
                window.wait(10)
            except subprocess.TimeoutExpired:
                window.kill()

    check("8: isolation: ~/.kotone/config.json unchanged", player_mtime() == before)
    failed = [n for n, r in RESULTS if r == "FAIL"]
    skipped = [n for n, r in RESULTS if r == "SKIP"]
    passed = len(RESULTS) - len(failed) - len(skipped)
    print(f"\n{passed}/{len(RESULTS) - len(skipped)} passed, {len(skipped)} skipped, {time.time() - started:.0f} s"
          + (f"; failed: {failed}" if failed else ""))
    print(CHECKLIST)
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
