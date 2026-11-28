"""Real-machine baseline for voice input (语音输入): key → microphone → speech recognition → text processing → typing
into a chat box. Written before voice input moves into a plugin (ADR 038); run it before and after the move.

========================================================================================================================
SAFETY — this script TYPES. Read before running.
  * It presses F22 (the isolated instance's voice-input key) and lets Kotone type text + Enter into a test window
    (target_window.ps1 -Chat). Run it only while nobody uses the PC. It prints:
        会往测试窗口里打字，测试期间别碰键盘鼠标（约 3 分钟）
  * Refuses to start if a game process (voice_turn_real.GAMES) is running or the foreground is not a terminal / the
    test window (SAFE_FOREGROUND below).
  * Right before every step that can end in typing, the test window is brought to front (fg.ps1) and checked; if that
    fails the script stops. While a session runs the foreground is watched: anything else in front → `runtime.stop`
    (cancels the session, nothing gets typed) and the script exits. The only key event that may go elsewhere is the
    release of a held F22 during clean-up (a key no keyboard has), and only when no game is in front.
  * Isolated: KOTONE_HOME, the WebView2 profile and the test window live under target/voice-input-real/. ~/.kotone is
    only read: models.dir points at ~/.kotone/models (the installed streaming x-asr model); at the end the script
    checks that ~/.kotone/config.json, ~/.kotone/history/history.jsonl and the model files did not change.
  * No AI key: voice input needs none; her game_chat.send runs against a local mock model inside this script.
    Her other input tools (game.act, automation.run) are denied.
  * Clean-up (always): runtime.stop, delete the mock connection (+ leftover test credential), app.quit, close the test
    window, stop the mock server.
========================================================================================================================

How a session starts without the player's hotkey: Kotone's default hotkey backend is the low-level hook, which ignores
injected input by design. The isolated config sets `hotkeyBackend: "register"` (docs/cli.md: auto / llhook / register),
i.e. RegisterHotKey through tauri-plugin-global-shortcut, which does react to SendInput — so `SendInput(F22)` is the
player pressing the voice-input key and goes through the real hotkey → orchestrator path. F22 because no keyboard has
it (F23 is the console-channel trigger, F24 the hook probe). Push-to-talk: F22 down … up (global-hotkey polls
GetAsyncKeyState for the release); dictation (录音笔): tap to start, tap to stop. The preview is confirmed with
`confirm_send` — the command behind the floating window's 发送 button — through WebView2's DevTools port.
Not covered here: the ADR 032 input gate needs the low-level hook (input_gate_e2e.py covers it).

Audio: crates/kotone-stt/tests/fixtures/zh-game-3s.wav (“对面打野在下路”) is played into “CABLE Input”; the isolated
instance records “CABLE Output (VB-Audio Virtual Cable)”. The first syllable may be eaten through the cable, so the
text is matched on “打野在下路” (as scripts/e2e-virtual-audio.sh does); the test window must receive exactly the text
that the history record says was sent.

Checks (PASS / FAIL / SKIP lines, total at the end):
  1. push-to-talk (the default preset) + auto-send, generic profile (Enter opens, Enter sends): one history record,
     the test window got exactly its text, Kotone's keys were only Enter + the text;
  2. dictation preset (preview): nothing typed while in preview; confirm_send types the preview text;
  3. post-processing: the blocklist filter with a CSV “打野 → JG” changes the text before it is sent;
  4. history: every session recorded with outcome sent in the isolated home; `history.list` (the history page) lists them;
  5. the floating window's states over `kotone://state` (CDP): listening → transcribing → processing → (preview →)
     sending → success → idle; the floating window appeared and went away (diagnostics overlay_show / overlay_hide);
  6. her game_chat.send (same typer): `tools call game_chat.send` (outside agents); her own path through a typed
     message and the mock model (the old tool companion.type is gone since ADR 053). Since ADR 041 phase 1 all of these (and voice
     input) go through the send-to-game entry, which refuses to type for her / outside agents unless a game it knows
     is in front or the selected profile is "generic" (any window): this test selects "generic", so they type into
     the test window as before. It deliberately adds NO profile for the test window's process (powershell.exe): the
     entry would then prefer it over the selected one, and sections 1-5 and 7 check which profile voice input used;
  7. console channel: a profile that declares one falls back to the keyboard when its game isn't in front
     (console_chat_skipped); the console send itself needs a real Steam game → SKIP;
  8. voice input plugin off (ADR 039 D2: typing into game chat is the body's): the tool service still lists
     game_chat.send (not a plugin's), `tools call game_chat.send` still types, and in a new conversation she still
     has game_chat.send and it types; none of these sends is written to the automation runs (automation.runs has no
     `kind: "chat"` record since 2026-11-27: what she sent is in the conversation); then the plugin is switched back on.

Prerequisites: target/debug/kotone.exe with the web UI embedded (README 准备: `pnpm build:web`, then
`cargo build -p kotone-tauri --features tauri/custom-protocol`), copied to target/voice-input-real/bin/ on start;
VB-CABLE; the x-asr streaming model in ~/.kotone/models; Python numpy + sounddevice; node + @playwright/test in
apps/desktop.
usage: python -X utf8 -I scripts/manual-e2e/voice_input_real.py [--exe path\\to\\kotone.exe]
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
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process, play_into_cable  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "voice-input-real")
HOME = os.path.join(WORK, "home")
WINDOW_DIR = os.path.join(WORK, "window")
BIN = os.path.join(WORK, "bin", "kotone.exe")  # a copy, so a running test instance is never taken for `pnpm dev`
DEBUG_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
DEVTOOLS = 9343
MOCK_PORT = 18768
PLAYER_HOME = os.path.join(os.path.expanduser("~"), ".kotone")
MODELS = os.path.join(PLAYER_HOME, "models")  # read only
MODEL_ID = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"  # engine sherpa-onnx-x-asr-zh-en
FIXTURE = os.path.join(REPO, "crates", "kotone-stt", "tests", "fixtures", "zh-game-3s.wav")
HEARD = "打野在下路"  # “对面打野在下路”, first syllable may be eaten
REPLACED = "JG在下路"
CABLE_OUTPUT = "CABLE Output (VB-Audio Virtual Cable)"
KEY, VK_KEY = "F22", 0x85
TEST_TITLE = "KotoneScriptTarget"
KOTONE_TAG = "4B544E49"
BANNER = "会往测试窗口里打字，测试期间别碰键盘鼠标（约 3 分钟）"
# Programs that may be in front when the script starts (terminals the operator runs it from) — never a game.
SAFE_FOREGROUND = {"powershell.exe", "pwsh.exe", "windowsterminal.exe", "openconsole.exe", "conhost.exe", "cmd.exe",
                   "warp.exe", "code.exe", "claude.exe", "wezterm-gui.exe", "alacritty.exe", "mintty.exe"}
# Her input tools: only the one under test is allowed.
RULES = {"game_chat.send": "allow", "game.act": "deny", "automation.run": "deny"}
GC_ASK = "跟队友说一句下路小心（E2E）"
GC_TEXT = "E2E 琴音代发：下路小心"
GC_TOOL_TEXT = "E2E 外部调用：下路小心"
GC_OFF_TEXT = "E2E 语音输入关着：下路小心"
CONSOLE_PROFILE = {
    "id": "e2e-console", "displayName": "测试：控制台通道（没有游戏）",
    "processNames": ["kotone-e2e-no-such-game.exe"], "windowTitlePatterns": [],
    "openChatKey": "Enter", "sendKey": "Enter", "preOpenDelayMs": 20, "prePasteDelayMs": 20, "preSendDelayMs": 100,
    "preferClipboardPaste": False, "hotwords": [],
    "channels": [{"id": "team", "displayName": "队伍", "default": True, "consoleCommand": "say_team"}],
    "consoleChat": {"cfgDir": "game/kotone-e2e/cfg", "triggerKey": "F23"},
}

EXE = BIN
ENV = {}
RESULTS = []  # (name, "PASS" | "FAIL" | "SKIP")
HELD = set()
MOCK_CALLS = []

user32 = ctypes.WinDLL("user32", use_last_error=True)


# ---------- results ----------

def check(name, ok, detail=""):
    RESULTS.append((name, "PASS" if ok else "FAIL"))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def skip(name, reason):
    RESULTS.append((name, "SKIP"))
    print(f"SKIP {name} — {reason}", flush=True)


# ---------- safety ----------

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
                             f"{out.stderr.strip()[:200]} - not typing.")
    if not in_front():
        raise SystemExit(f"STOP ({where}): the test window is not in front ({foreground()}) - not typing.")


def emergency_stop(why):
    """Something else came to front during a session: stop voice input (cancels the session) before it types."""
    call_raw("runtime.stop", timeout_ms=5000)
    raise SystemExit(f"STOP: {why} - stopped Kotone's voice input so nothing gets typed.")


def watch_foreground():
    if not in_front():
        emergency_stop(f"the test window lost the foreground to {foreground()}")


# ---------- keys (copied from input_gate_e2e.py) ----------

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


def key_down():
    """The voice-input key, pressed only with the test window in front."""
    if not in_front():
        emergency_stop("the test window is not in front before pressing the voice-input key")
    send_key(VK_KEY, False)


def key_up():
    send_key(VK_KEY, True)


def tap():
    key_down()
    time.sleep(0.05)
    key_up()


def release_held():
    """Clean-up: never leave F22 held down (it is not a typing key); skipped when a game is in front."""
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
    return page_eval("return await window.__TAURI_INTERNALS__.invoke(%s, %s);"
                     % (json.dumps(command), json.dumps(args or {}, ensure_ascii=False)))


# The floating window's states: record every `kotone://state` event in the settings page (same events the overlay
# listens to), read them back later.
RECORDER = """
const I = window.__TAURI_INTERNALS__;
if (window.__voiceInputE2E) return "already";
const seen = [];
await I.invoke("plugin:event|listen", { event: "kotone://state", target: { kind: "Any" },
  handler: I.transformCallback((e) => {
    const p = (e.payload && e.payload.payload) || {};
    seen.push({ at: Date.now(), state: e.payload.state, purpose: p.purpose || null,
                text: typeof p.text === "string" ? p.text : null });
  }) });
window.__voiceInputE2E = seen;
return "installed";
"""


def states_mark():
    try:
        return page_eval("return window.__voiceInputE2E ? window.__voiceInputE2E.length : -1;").get("ok", -1)
    except RuntimeError:
        return -1


def states_since(mark):
    if mark < 0:
        return None
    try:
        return page_eval(f"return window.__voiceInputE2E ? window.__voiceInputE2E.slice({mark}) : null;").get("ok")
    except RuntimeError:
        return None


# ---------- files: kotone.log, history, diagnostics, test window ----------

def size(path):
    return os.path.getsize(path) if os.path.exists(path) else 0


def read_from(path, mark):
    if not os.path.exists(path):
        return ""
    with open(path, "rb") as f:
        if os.path.getsize(path) >= mark:
            f.seek(mark)
        return f.read().decode("utf-8", errors="replace")


LOG = os.path.join(HOME, "kotone.log")
HISTORY = os.path.join(HOME, "history", "history.jsonl")
DIAG = os.path.join(HOME, "diagnostics", "events.jsonl")


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


def history():
    out = []
    for line in read_from(HISTORY, 0).splitlines():
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return out


def diag_since(mark):
    out = []
    for line in read_from(DIAG, mark).splitlines():
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return out


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
    """What a chat box would have received: [(channel, text)], plus the game keys and Kotone's Enters
    (from input_gate_e2e.py: Enter opens, characters go in, Enter sends)."""
    messages, game_keys, enters = [], [], []
    open_channel, units = None, []
    for ev in events:
        if ev["kind"] == "DOWN" and ev.get("key") == "Return":
            if ev.get("x") == KOTONE_TAG:
                enters.append(ev)
            if open_channel is None:
                open_channel, units = ("all" if ev.get("shift") == "True" else "team"), []
            else:
                messages.append((open_channel, b"".join(u.to_bytes(2, "little") for u in units).decode("utf-16-le")))
                open_channel = None
        elif ev["kind"] == "DOWN" and open_channel is None and ev.get("key") != "Packet":
            game_keys.append(ev.get("key"))
        elif ev["kind"] == "CHAR" and open_channel is not None and ev["unit"] >= 0x20:
            units.append(ev["unit"])
    return messages, game_keys, enters, open_channel


# ---------- sessions ----------

def marks():
    return {"window": len(window_lines()), "history": len(history()), "log": size(LOG), "diag": size(DIAG),
            "states": states_mark()}


def collect(m, end):
    events = parse(window_lines()[m["window"]:])
    messages, _, enters, still_open = chat(events)
    return {"end": end.group(1) if end else None, "records": history()[m["history"]:], "messages": messages,
            "still_open": still_open, "enters": len(enters), "events": events,
            "tagged": sorted({e["key"] for e in events if e.get("x") == KOTONE_TAG}),
            "states": states_since(m["states"]), "diag": diag_since(m["diag"]),
            "log_states": dedup(re.findall(r"state -> (\w+)", read_from(LOG, m["log"])))}


def finish(m, timeout=40):
    end = wait_log(m["log"], r"state -> (success|error)\b", timeout)
    if end and end.group(1) == "error":
        invoke("cancel_session")  # the floating window's close: an error stays until dismissed
    wait_log(m["log"], r"state -> idle\b", 10)
    time.sleep(0.6)  # the test window logs asynchronously
    return collect(m, end)


def play():
    """The fixture into CABLE Input (0.8 s lead, 1 s tail) while watching the foreground."""
    failure = []

    def run():
        try:
            play_into_cable(FIXTURE, 0.8)
        except Exception as e:  # noqa: BLE001
            failure.append(e)
    t = threading.Thread(target=run, daemon=True)
    t.start()
    while t.is_alive():
        watch_foreground()
        time.sleep(0.1)
    if failure:
        raise RuntimeError(f"playing into CABLE Input failed: {failure[0]}")


def push_to_talk(where):
    """Push-to-talk (the player default): hold F22 while the fixture plays, release → recognised → sent."""
    focus(where)
    m = marks()
    key_down()
    try:
        listening = wait_log(m["log"], r"state -> listening\b", 10)
        if listening:
            play()
    finally:
        if VK_KEY in HELD and in_front():
            key_up()
    s = finish(m) if listening else collect(m, None)
    s["listening"] = bool(listening)
    return s


def dictation_preview(where):
    """Dictation preset (录音笔): tap F22, speak, tap F22 → preview; nothing typed; confirm_send → sent."""
    focus(where)
    m = marks()
    tap()
    listening = wait_log(m["log"], r"state -> listening\b", 10)
    preview = None
    if listening:
        play()
        focus(where + " (end)")
        tap()
        preview = wait_log(m["log"], r"state -> (preview|error)\b", 30)
    deadline = time.time() + 2.0  # time for anything that would be typed to arrive
    while time.time() < deadline:
        watch_foreground()
        time.sleep(0.1)
    before = parse(window_lines()[m["window"]:])
    confirmed = None
    if preview and preview.group(1) == "preview":
        focus(where + " (confirm)")
        confirmed = invoke("confirm_send")
        s = finish(m)
    else:
        invoke("cancel_session")
        wait_log(m["log"], r"state -> idle\b", 10)
        s = collect(m, None)
    s.update(listening=bool(listening), preview=preview.group(1) if preview else None, before=before,
             confirmed=confirmed)
    return s


def dedup(seq):
    out = []
    for x in seq:
        if not out or out[-1] != x:
            out.append(x)
    return out


def state_flow(states):
    """Voice-input states in order (repeats folded, leading idle dropped); None if the recorder is gone."""
    if states is None:
        return None
    seq = [s["state"] for s in states if s.get("purpose") in (None, "dictation")]
    while seq and seq[0] == "idle":
        seq.pop(0)
    return dedup(seq)


def expect_sent(step, s, contains, profile):
    """Checks shared by every auto-sent voice session."""
    recs = s["records"]
    rec = recs[-1] if recs else {}
    text = rec.get("finalText", "")
    check(f"{step}: the voice-input key ({KEY}) started a session", s["listening"])
    check(f"{step}: one new history record, outcome sent, profile {profile}",
          len(recs) == 1 and rec.get("outcome") == "sent" and rec.get("profileId") == profile,
          json.dumps([{k: r.get(k) for k in ("outcome", "profileId", "error")} for r in recs], ensure_ascii=False))
    check(f"{step}: recognised “…{contains}”", contains in text,
          f"source “{rec.get('sourceText')}” → final “{text}”")
    check(f"{step}: the test window got exactly that text", text and s["messages"] == [("team", text)]
          and not s["still_open"], f"{s['messages']} open={s['still_open']}")
    check(f"{step}: Kotone sent only the profile's open/send key (Enter ×2) and the text",
          s["tagged"] == ["Packet", "Return"] and s["enters"] == 2, f"tagged {s['tagged']}, Enters {s['enters']}")
    return rec


def expect_states(step, s, expected):
    seq = state_flow(s["states"])
    log_seq = " → ".join(x for x in s["log_states"] if x)
    if seq is None:
        skip(f"5 ({step}): floating-window states via CDP", f"event recorder not available; kotone.log: {log_seq}")
        return
    check(f"5 ({step}): floating-window states {' → '.join(expected)}", seq == expected,
          f"CDP {' → '.join(seq)} | kotone.log {log_seq}")


# ---------- mock model for her game_chat.send ----------

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
    """The typed request (it carries “（E2E）”) → game_chat.send(GC_TEXT, send) → “好了”. Anything else → “好了”."""
    messages = body.get("messages", [])
    last_user = max((i for i, m in enumerate(messages) if m.get("role") == "user"), default=-1)
    user = text_of(messages[last_user]) if last_user >= 0 else ""
    done = sum(1 for m in messages[last_user + 1:] if m.get("role") == "tool")
    names = [(t.get("function") or {}).get("name", "") for t in body.get("tools") or []]
    tool = next((n for n in names if n.replace(".", "_") == "game_chat_send"), None)
    if "（E2E）" in user and tool and done == 0:
        MOCK_CALLS.append(tool)
        args = json.dumps({"text": GC_TEXT, "send": True}, ensure_ascii=False)
        return [chunk({"role": "assistant", "tool_calls": [{"index": 0, "id": "call_0", "type": "function",
                                                           "function": {"name": tool, "arguments": args}}]}),
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


# ---------- setup / clean-up ----------

def player_snapshot():
    """mtimes (and sizes) of the player's files this test must not touch."""
    snap = {}
    for p in (os.path.join(PLAYER_HOME, "config.json"), os.path.join(PLAYER_HOME, "history", "history.jsonl")):
        snap[p] = os.stat(p).st_mtime_ns if os.path.exists(p) else None
    for root, _, files in os.walk(MODELS):
        for f in files:
            st = os.stat(os.path.join(root, f))
            snap[os.path.join(root, f)] = (st.st_size, st.st_mtime_ns)
    return snap


def write_config():
    csv = os.path.join(WORK, "replace.csv")
    with open(csv, "w", encoding="utf-8-sig", newline="") as f:
        f.write("屏蔽词,替换词\r\n打野,JG\r\n")

    def blocklist(pid, name, config):
        return {"id": pid, "displayName": name, "steps": [{
            "id": f"{pid}-1", "processorId": "builtin.blocklist-filter", "enabled": True, "config": config,
            "timeoutMs": 5000, "onError": "required"}]}
    config = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "legacyUpgrade": "notNeeded"},
        "adminPromptDismissed": True,
        "toolService": {"enabled": True},
        "models": {"dir": MODELS},
        "sttEngine": "sherpa-streaming",
        "activeModelId": MODEL_ID,
        "engineOptions": {"sherpa-streaming": {"provider": "cpu"}},
        "audioDeviceId": CABLE_OUTPUT,
        "hotkeyBackend": "register",
        "hotkey": {"key": KEY, "mode": "toggle"},
        "interactionMode": "push-to-talk",
        # "generic" = any foreground window: the send-to-game entry (ADR 041) then lets game_chat.send
        # type into the test window; no profile for powershell.exe on purpose (see the docstring, step 6)
        "activeProfileId": "generic",
        # nothing else registered system-wide through RegisterHotKey while the test runs
        "channelCycleHotkey": "", "resendLastHotkey": "", "togglePostProcessingHotkey": "",
        "cyclePostProcessingHotkey": "", "runtimeToggleHotkey": "", "automationStopHotkey": "",
        "soundFeedback": {"record": {"enabled": False}, "send": {"enabled": False}},
        "postProcessing": {"enabled": True, "activePipelineId": "blocklist", "pipelines": [
            blocklist("blocklist", "屏蔽词", {}),
            blocklist("e2e-replace", "测试替换", {"csvPath": csv}),
        ]},
        "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False}},
        "agent": {"permissions": {"mode": "default", "rules": RULES}, "progressSpeech": False},
    }
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
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


def sweep_credentials():
    """The mock connection's key lives in the isolated credential space `kotone@<KOTONE_HOME>` (from
    background_task_real.py): remove anything left there."""
    listing = subprocess.run(["cmdkey", "/list"], capture_output=True, text=True, encoding="mbcs",
                             errors="replace").stdout
    for line in listing.splitlines():
        if "target=" in line and WORK in line:
            subprocess.run(["cmdkey", f"/delete:{line.split('target=', 1)[1].strip()}"], capture_output=True)
            print("removed leftover test credential")


# ---------- the test ----------

def main():
    global EXE, ENV
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", help="kotone.exe to test (default: a copy of target/debug/kotone.exe in "
                                      "target/voice-input-real/bin)")
    opts = parser.parse_args()

    print("=" * 72)
    print("  " + BANNER)
    print("  (Ctrl+C within 5 s to cancel)")
    print("=" * 72, flush=True)
    guard("start")
    time.sleep(5)
    guard("start")
    if not os.path.exists(FIXTURE):
        raise SystemExit(f"missing fixture {FIXTURE}")
    if not os.path.isdir(MODELS):
        raise SystemExit(f"no models directory {MODELS} (install the x-asr streaming model first)")

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
    write_config()
    before = player_snapshot()

    app = window = mock = None
    mock_connection = False
    started = time.time()
    try:
        app = subprocess.Popen([EXE, "--background"], env=ENV)
        if not wait_until(lambda: call_raw("app.status", timeout_ms=3000).get("ok"), 90):
            raise SystemExit("the isolated instance's tool service did not come up")
        if not wait_until(lambda: "ok" in invoke("agent_state"), 90):
            raise SystemExit("the settings page is not reachable on the DevTools port (embedded web UI built?)")
        recorder = page_eval(RECORDER)
        print("kotone://state recorder:", recorder)

        devices = call("audio.devices").get("devices", [])
        cable = next((d["id"] for d in devices if str(d.get("id", "")).startswith("CABLE Output")), None)
        if not cable:
            raise SystemExit(f"VB-CABLE “CABLE Output” not found among {[d.get('id') for d in devices]}")
        if cable != CABLE_OUTPUT:
            call("settings.update", {"patch": {"audioDeviceId": cable}})
        # 工具服务的等待最多 30 秒（1-30000 ms）：先发启动，再等语音识别跑起来（加载模型可能要几十秒）
        status = call("runtime.start", timeout_ms=30_000)
        if status.get("phase") != "running":
            status = wait_until(lambda: (lambda st: st if st.get("phase") == "running" else None)(call("runtime.status")),
                                90, 1) or call("runtime.status")
        hk = call("hotkeys.status")
        print(f"runtime: {status.get('phase')} {status.get('modelId')} ({status.get('interactionMode')}); "
              f"hotkey {hk.get('key')} via {hk.get('backend')} registered={hk.get('registered')}")
        if status.get("phase") != "running" or hk.get("backend") != "register" or not hk.get("registered"):
            raise SystemExit("voice input did not start with the RegisterHotKey backend - nothing to test "
                             f"(runtime {status}, hotkeys {hk})")

        window = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                                   os.path.join(HERE, "target_window.ps1"), "-Dir", WINDOW_DIR, "-Chat"])
        if not wait_until(lambda: os.path.exists(os.path.join(WINDOW_DIR, "ready.txt")), 30, 0.25):
            raise SystemExit("the test window did not come up")

        # 1. push-to-talk + auto-send, generic profile, the default blocklist pipeline
        print("\n== 1. push-to-talk, auto-send, generic profile ==")
        s1 = push_to_talk("1")
        r1 = expect_sent("1", s1, HEARD, "generic")
        expect_states("1", s1, ["listening", "transcribing", "processing", "sending", "success", "idle"])
        shown = [e.get("activity") for e in s1["diag"] if e.get("activity") in ("overlay_show", "overlay_hide")]
        first_show = shown.index("overlay_show") if "overlay_show" in shown else None
        check("5 (1): the floating window appeared and went away (overlay_show → overlay_hide)",
              first_show is not None and "overlay_hide" in shown[first_show:], f"{shown}")

        # 2. dictation preset: preview, nothing typed until confirmed
        print("\n== 2. dictation (preview) ==")
        call("settings.update", {"patch": {"interactionMode": "dictation"}})
        s2 = dictation_preview("2")
        rec2 = s2["records"][-1] if s2["records"] else {}
        text2 = rec2.get("finalText", "")
        preview_text = next((x.get("text") for x in (s2["states"] or []) if x.get("state") == "preview"), None)
        check("2: the voice-input key started a session (tap)", s2["listening"])
        check("2: second tap → preview", s2["preview"] == "preview", f"reached {s2['preview']}")
        typed_early = [e["raw"] for e in s2["before"] if e.get("x") == KOTONE_TAG or e["kind"] == "CHAR"]
        check("2: nothing typed while in preview", s2["preview"] == "preview" and not typed_early, f"{typed_early[:6]}")
        check("2: confirm_send (the floating window's 发送) accepted", s2["confirmed"] is not None
              and "error" not in (s2["confirmed"] or {}), json.dumps(s2["confirmed"], ensure_ascii=False))
        check("2: one history record, outcome sent", len(s2["records"]) == 1 and rec2.get("outcome") == "sent",
              json.dumps([{k: r.get(k) for k in ("outcome", "error")} for r in s2["records"]], ensure_ascii=False))
        check(f"2: recognised “…{HEARD}”", HEARD in text2, f"“{text2}”")
        check("2: after confirming, the test window got exactly the preview text",
              text2 and s2["messages"] == [("team", text2)] and not s2["still_open"]
              and (preview_text is None or preview_text == text2), f"{s2['messages']} preview “{preview_text}”")
        expect_states("2", s2, ["listening", "transcribing", "processing", "preview", "sending", "success", "idle"])

        # 3. post-processing: the blocklist filter with “打野 → JG”
        print("\n== 3. post-processing (blocklist CSV 打野 → JG) ==")
        call("settings.update", {"patch": {"interactionMode": "push-to-talk",
                                           "postProcessing": {"activePipelineId": "e2e-replace"}}})
        s3 = push_to_talk("3")
        r3 = expect_sent("3", s3, REPLACED, "generic")
        check("3: the recognised text had 打野, the sent text has JG instead",
              HEARD in (r3.get("sourceText") or "") and "打野" not in r3.get("finalText", "")
              and r3.get("processDurationMs") is not None,
              f"source “{r3.get('sourceText')}” → final “{r3.get('finalText')}” ({r3.get('processDurationMs')} ms)")
        expect_states("3", s3, ["listening", "transcribing", "processing", "sending", "success", "idle"])

        # 4. history
        print("\n== 4. history ==")
        sessions = [r.get("sessionId") for r in (r1, rec2, r3) if r]
        listed = call("history.list", {"limit": 50}).get("records", [])
        by_id = {r.get("sessionId"): r for r in listed}
        check("4: history.list (the history page) has the 3 sessions, all sent",
              len(sessions) == 3 and all(by_id.get(i, {}).get("outcome") == "sent" for i in sessions),
              f"{len(listed)} records; {[(i, by_id.get(i, {}).get('outcome')) for i in sessions]}")
        check("4: written to the isolated home", os.path.exists(HISTORY) and len(history()) >= 3, HISTORY)

        # 6. her game_chat.send — the same typer
        print("\n== 6. game_chat.send ==")
        focus("6a")  # it types at once
        m = marks()
        env = call_raw("game_chat.send", {"text": GC_TOOL_TEXT, "send": True})
        time.sleep(1.0)
        s = collect(m, None)
        # outside agents can call it (since 2026-11-24; the body's tool since ADR 039 D2)
        check("6a: tools call game_chat.send typed into the test window",
              env.get("ok") and s["messages"] == [("team", GC_TOOL_TEXT)], f"{s['messages']} {env.get('error')}")

        mock = ThreadingHTTPServer(("127.0.0.1", MOCK_PORT), MockModel)
        threading.Thread(target=mock.serve_forever, daemon=True).start()
        call("connections.upsert", {"connection": {"id": "mock", "displayName": "mock", "kind": "remote",
                                                   "provider": "custom", "baseUrl": f"http://127.0.0.1:{MOCK_PORT}/v1",
                                                   "model": "mock"}, "apiKey": "sk-local-fixture"})
        mock_connection = True
        call("settings.update", {"patch": {"assistant": {"connectionId": "mock"}}})
        invoke("agent_new_conversation")
        focus("6b")
        m = marks()
        sent = invoke("agent_send", {"text": GC_ASK})
        cid = (sent.get("ok") or {}).get("conversationId")

        def chat_result():
            watch_foreground()
            conv = invoke("agent_conversation", {"id": cid}).get("ok") or {}
            return next((x for x in conv.get("messages", []) if x.get("role") == "tool"
                         and x.get("name") == "game_chat.send"), None)
        result = wait_until(chat_result, 40) if cid else None
        time.sleep(1.0)
        s = collect(m, None)
        check("6b: typed request accepted", cid, json.dumps(sent, ensure_ascii=False)[:200])
        check("6b: she called game_chat.send (mock model, no key)", MOCK_CALLS, f"{MOCK_CALLS}")
        check("6b: game_chat.send succeeded (typed; no runId: not written to the automation runs)",
              result and '"typed"' in result.get("content", "") and '"runId"' not in result.get("content", ""),
              (result or {}).get("content", "")[:200])
        check("6b: the test window got exactly her text", s["messages"] == [("team", GC_TEXT)] and not s["still_open"],
              f"{s['messages']}")

        # 7. console channel
        print("\n== 7. console channel ==")
        call("profiles.save", {"profile": CONSOLE_PROFILE})
        call("settings.update", {"patch": {"activeProfileId": CONSOLE_PROFILE["id"]}})
        s7 = push_to_talk("7a")
        expect_sent("7a", s7, REPLACED, CONSOLE_PROFILE["id"])
        skipped = [(e.get("data") or {}).get("detail") for e in s7["diag"] if e.get("activity") == "console_chat_skipped"]
        trigger = [e["raw"] for e in s7["events"] if e.get("key") == "F23"]
        check("7a: a profile with a console channel falls back to the keyboard without its game "
              "(console_chat_skipped, no F23)", skipped and not trigger, f"reason {skipped}, F23 events {trigger}")
        call("settings.update", {"patch": {"activeProfileId": "generic"}})
        call("profiles.delete", {"id": CONSOLE_PROFILE["id"]})
        skip("7b: console send (say_team via kotone_say.cfg + F23)",
             "needs a Steam-installed Source 2 game (Dota 2 / CS2) in front, started after Kotone's autoexec.cfg line; "
             "it writes into the game's cfg folder - not fakeable without touching a real game install")

        # 8. voice input plugin off: game_chat.send is the body's (ADR 039 D2), she and outside agents still type
        print("\n== 8. voice input plugin off: game_chat.send still works ==")
        call("settings.update", {"patch": {"plugins": {"voice-input": {"enabled": False}}}})
        spec = next((t for t in call("system.capabilities").get("tools", []) if t.get("name") == "game_chat.send"),
                    None)
        check("8a: voice input off: the tool service still lists game_chat.send, as the body's (no plugin)",
              spec is not None and "plugin" not in spec, json.dumps(spec, ensure_ascii=False)[:200])
        focus("8b")
        m = marks()
        typed = call_raw("game_chat.send", {"text": GC_OFF_TEXT, "send": True})
        time.sleep(1.0)
        s = collect(m, None)
        check("8b: voice input off: tools call game_chat.send typed into the test window",
              typed.get("ok") and s["messages"] == [("team", GC_OFF_TEXT)], f"{s['messages']} {typed.get('error')}")
        invoke("agent_new_conversation")  # her tool list is fixed per conversation (ADR 024)
        calls_before = len(MOCK_CALLS)
        focus("8c")
        m = marks()
        sent = invoke("agent_send", {"text": GC_ASK})
        cid8 = (sent.get("ok") or {}).get("conversationId")

        def chat_result_off():
            watch_foreground()
            conv = invoke("agent_conversation", {"id": cid8}).get("ok") or {}
            return next((x for x in conv.get("messages", []) if x.get("role") == "tool"
                         and x.get("name") == "game_chat.send"), None)
        result = wait_until(chat_result_off, 40) if cid8 else None
        time.sleep(1.0)
        s = collect(m, None)
        check("8c: voice input off: she still has game_chat.send (mock model called it), typed (no runId)",
              len(MOCK_CALLS) > calls_before and result and '"typed"' in result.get("content", "")
              and '"runId"' not in result.get("content", ""), (result or {}).get("content", "")[:200])
        check("8c: voice input off: the test window got exactly her text",
              s["messages"] == [("team", GC_TEXT)] and not s["still_open"], f"{s['messages']}")
        # 「自动化 → 记录」: sending to game chat is not an automation, nothing is written there (2026-11-27)
        runs = call("automation.runs").get("runs", [])
        chat_runs = [r for r in runs if (r.get("action") or {}).get("kind") == "chat"
                     or any(text in json.dumps(r.get("action"), ensure_ascii=False)
                            for text in (GC_TOOL_TEXT, GC_OFF_TEXT, GC_TEXT))]
        check("8d: none of the sends is in the automation runs (no kind: chat)", not chat_runs,
              json.dumps(chat_runs, ensure_ascii=False)[:300])
        call("settings.update", {"patch": {"plugins": {"voice-input": {"enabled": True}}}})
    finally:
        print("\n== clean-up ==")
        release_held()
        if app is not None:
            call_raw("runtime.stop", timeout_ms=10_000)
            if mock_connection:
                print("delete mock connection:", call_raw("connections.delete", {"id": "mock"}).get("ok"))
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
        if mock is not None:
            mock.shutdown()
            mock.server_close()
        sweep_credentials()

    after = player_snapshot()
    changed = [p for p in set(before) | set(after) if before.get(p) != after.get(p)]
    check("isolation: ~/.kotone config, history and model files unchanged", not changed, f"{changed[:5]}")
    failed = [n for n, r in RESULTS if r == "FAIL"]
    skipped = [n for n, r in RESULTS if r == "SKIP"]
    passed = len(RESULTS) - len(failed) - len(skipped)
    print(f"\n{passed}/{len(RESULTS) - len(skipped)} passed, {len(skipped)} skipped, {time.time() - started:.0f} s"
          + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
