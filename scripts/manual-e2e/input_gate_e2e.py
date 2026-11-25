"""Real-machine test of ADR 032 decision 1: while Kotone types a chat message, the player's key downs are swallowed.

Everything is isolated: KOTONE_HOME=target/gate-e2e/home, the test window target_window.ps1 -Chat, no game, no player
config. The isolated home gets a game profile "e2e-chat" for the test window's process (powershell.exe) with League of
Legends' chat keys: since ADR 041 phase 1 companion.type goes through the send-to-game entry, which only types into a
game it recognises in front. The "player" is this script: SendInput WITHOUT Kotone's tag (dwExtraInfo 0), which the hook treats exactly like
a physical key. The test window logs every key with its Shift state and dwExtraInfo (Kotone's keys carry 4B544E49), and
this script rebuilds what a chat box would have received: Enter opens it (Shift+Enter = "all" channel), characters go
in, Enter sends it.

  1. control: the first send after start-up is not protected (the hook has not seen a Kotone key yet), letters typed
     during it should end up in the message - this shows the disturbance really lands inside the send;
  2. letters: a protected send while the player keeps typing Q W E R: the message is exactly the text;
  3. Shift: the player presses Shift right after Kotone's first Enter and holds it for a second: both of Kotone's Enters
     arrive without Shift, the window never sees a Shift down, and Kotone sends no key after its last Enter.

usage:
  python -X utf8 -I scripts/manual-e2e/input_gate_e2e.py [path-to-kotone.exe]
    (default target/gate-e2e/bin/kotone.exe - a copy of target/debug/kotone.exe, so a running test instance is never
     mistaken for the player's `pnpm dev`)

Stops before sending anything if a game is running or something other than a terminal / the test window is in front.
"""
import ctypes
import ctypes.wintypes as wt
import json
import os
import shutil
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "gate-e2e")
HOME = os.path.join(WORK, "home")
TARGET = os.path.join(WORK, "target")
DEBUG_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
BIN = os.path.join(WORK, "bin", "kotone.exe")
EXE = sys.argv[1] if len(sys.argv) > 1 else BIN
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
KOTONE_TAG = "4B544E49"
TEXT = "对面打野来下路了，小心 gank"

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

GAMES = ("league of legends", "leagueclient", "riotclient", "dota2", "cs2", "yuanshen", "genshinimpact",
         "valorant", "starrail", "zenlesszonezero")
# Programs that may hold the foreground while this script runs; anything else (a game!) stops it.
SAFE_FOREGROUND = {"powershell.exe", "pwsh.exe", "WindowsTerminal.exe", "OpenConsole.exe", "conhost.exe",
                   "explorer.exe", "Code.exe", "claude.exe", "kotone.exe", "python.exe", "warp.exe", ""}


def running_games():
    out = subprocess.run(["tasklist", "/fo", "csv", "/nh"], capture_output=True, text=True, errors="replace").stdout
    names = {line.split('","')[0].strip('"').lower() for line in out.splitlines() if line}
    return sorted(n for n in names if any(g in n for g in GAMES))


def foreground_process():
    hwnd = user32.GetForegroundWindow()
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    handle = kernel32.OpenProcess(0x1000, False, pid.value)  # PROCESS_QUERY_LIMITED_INFORMATION
    name = ""
    if handle:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        if kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size)):
            name = os.path.basename(buf.value)
        kernel32.CloseHandle(handle)
    title = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(hwnd, title, 256)
    return name, title.value


def guard():
    games = running_games()
    if games:
        raise SystemExit(f"STOP: a game is running ({', '.join(games)}) - not sending any input.")
    process, window = foreground_process()
    if window != "KotoneScriptTarget" and process not in SAFE_FOREGROUND:
        raise SystemExit(f"STOP: '{window}' ({process}) is in front - not touching it.")


def focus():
    guard()
    out = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                          os.path.join(HERE, "fg.ps1"), "-Title", "KotoneScriptTarget"], capture_output=True, text=True)
    assert "True" in out.stdout, out.stdout + out.stderr
    time.sleep(0.4)
    assert foreground_process()[1] == "KotoneScriptTarget", foreground_process()


def call(tool, args=None, timeout_ms=30_000):
    path = os.path.join(WORK, "args.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(args if args is not None else {}, f, ensure_ascii=False)
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", path, "--timeout-ms", str(timeout_ms)],
                         capture_output=True, text=True, encoding="utf-8", env=ENV)
    try:
        return json.loads(out.stdout)
    except json.JSONDecodeError:
        return {"ok": False, "raw": out.stdout, "stderr": out.stderr}


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def player_key(vk, up):
    """One key event WITHOUT Kotone's tag: to the hook this is the player's key."""
    if foreground_process()[1] != "KotoneScriptTarget":
        raise SystemExit("STOP: the test window lost the foreground - not sending more keys.")
    i = INPUT(type=1)
    i.ki = KEYBDINPUT(vk, user32.MapVirtualKeyW(vk, 0), 2 if up else 0, 0, 0)
    assert user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT)) == 1


def log_lines():
    with open(os.path.join(TARGET, "events.log"), encoding="utf-8") as f:
        return f.read().splitlines()


def parse(lines):
    out = []
    for line in lines:
        ts, _, rest = line.partition(" ")
        parts = rest.split()
        if not parts:
            continue
        ev = {"ts": int(ts), "kind": parts[0], "raw": rest}
        if parts[0] in ("DOWN", "UP"):
            ev["key"] = parts[1]
            for p in parts[2:]:
                k, _, v = p.partition("=")
                ev[k] = v
        elif parts[0] == "CHAR":
            ev["unit"] = int(parts[1], 16)
        out.append(ev)
    return out


def chat(events):
    """What a chat box would have received: [(channel, text)], plus the game keys and Kotone's Enters."""
    messages, game_keys, enters = [], [], []
    open_channel, units = None, []
    for ev in events:
        if ev["kind"] == "DOWN" and ev["key"] == "Return":
            if ev.get("x") == KOTONE_TAG:
                enters.append(ev)
            if open_channel is None:
                open_channel, units = ("all" if ev.get("shift") == "True" else "team"), []
            else:
                messages.append((open_channel, b"".join(u.to_bytes(2, "little") for u in units).decode("utf-16-le")))
                open_channel = None
        elif ev["kind"] == "DOWN" and open_channel is None and ev["key"] != "Packet":
            game_keys.append(ev["key"])
        elif ev["kind"] == "CHAR" and open_channel is not None and ev["unit"] >= 0x20:
            units.append(ev["unit"])
    return messages, game_keys, enters, open_channel


def disturb_letters(stop):
    keys = [0x51, 0x57, 0x45, 0x52]  # Q W E R
    i = 0
    while not stop.is_set():
        vk = keys[i % 4]
        player_key(vk, False)
        time.sleep(0.003)
        player_key(vk, True)
        time.sleep(0.005)
        i += 1


def send_with(disturbance):
    """Run one companion.type send while `disturbance(stop)` runs; return the parsed events of this round."""
    start = len(log_lines())
    stop = threading.Event()
    worker = threading.Thread(target=disturbance, args=(stop,), daemon=True)
    worker.start()
    time.sleep(0.15)
    env = call("companion.type", {"text": TEXT, "send": True})
    time.sleep(0.3)
    stop.set()
    worker.join(5)
    time.sleep(1.4)
    assert env.get("ok"), json.dumps(env, ensure_ascii=False)[:600]
    return parse(log_lines()[start:])


def gate_records():
    path = os.path.join(HOME, "diagnostics", "events.jsonl")
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as f:
        return [json.loads(l) for l in f if '"inject_gate"' in l]


PROFILE_ID = "e2e-chat"


def write_test_window_profile():
    """A game profile for the test window's process (powershell.exe), with League of Legends' chat keys (team = Enter,
    all = Shift+Enter, send = Enter, same delays). Since ADR 041 phase 1 companion.type goes through the send-to-game
    entry, which picks the profile of the game in front and refuses a foreground that is not a game; before that it
    typed into whatever was in front with the selected profile (League of Legends by default). This profile keeps the
    keys exactly as they were and lets the entry accept the test window."""
    os.makedirs(os.path.join(HOME, "profiles"), exist_ok=True)
    profile = {
        "id": PROFILE_ID, "displayName": "E2E chat (LoL keys)", "processNames": ["powershell.exe"],
        "windowTitlePatterns": [], "openChatKey": "Enter", "sendKey": "Enter",
        "preOpenDelayMs": 20, "prePasteDelayMs": 20, "preSendDelayMs": 100, "preferClipboardPaste": False,
        "hotwords": [],
        "channels": [{"id": "team", "displayName": "队伍", "default": True},
                     {"id": "all", "displayName": "所有人", "openChatKey": "Shift+Enter"}],
    }
    with open(os.path.join(HOME, "profiles", f"{PROFILE_ID}.json"), "w", encoding="utf-8") as f:
        json.dump(profile, f, ensure_ascii=False, indent=1)


def wait_for(pred, seconds, what):
    deadline = time.time() + seconds
    while time.time() < deadline:
        if pred():
            return
        time.sleep(0.25)
    raise AssertionError(f"timed out waiting for {what}")


def refresh_copy():
    """The default exe is a copy of target/debug/kotone.exe (refreshed when the build is newer)."""
    if EXE != BIN:
        return
    if not os.path.exists(DEBUG_EXE) and not os.path.exists(BIN):
        raise SystemExit("no target/debug/kotone.exe - build it first (cargo build -p kotone-tauri)")
    if os.path.exists(DEBUG_EXE) and (not os.path.exists(BIN) or os.path.getmtime(DEBUG_EXE) > os.path.getmtime(BIN)):
        os.makedirs(os.path.dirname(BIN), exist_ok=True)
        shutil.copy2(DEBUG_EXE, BIN)


def main():
    guard()
    refresh_copy()
    os.makedirs(HOME, exist_ok=True)
    os.makedirs(TARGET, exist_ok=True)
    for name in ("ready.txt", "events.log"):
        if os.path.exists(os.path.join(TARGET, name)):
            os.remove(os.path.join(TARGET, name))
    config = os.path.join(HOME, "config.json")
    if not os.path.exists(config):
        with open(config, "w", encoding="utf-8") as f:
            json.dump({"toolService": {"enabled": True}, "ui": {"firstRunCompleted": True}}, f)
    write_test_window_profile()

    app = subprocess.Popen([EXE, "--background"], env=ENV)
    window = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                               os.path.join(HERE, "target_window.ps1"), "-Dir", TARGET, "-Chat"])
    failures = []
    try:
        wait_for(lambda: os.path.exists(os.path.join(TARGET, "ready.txt")), 30, "test window")
        wait_for(lambda: call("app.status", timeout_ms=3000).get("ok"), 90, "tool service")
        records_before = len(gate_records())
        focus()

        # 1. control: unprotected first send
        events = send_with(disturb_letters)
        messages, game_keys, enters, still_open = chat(events)
        print(f"[control] messages={messages} gameKeys={len(game_keys)} kotoneEnters={len(enters)} open={still_open}")
        if messages and messages[0][1] == TEXT:
            print("[control] WARNING: the unprotected send was not disturbed - the checks below prove less")
        if still_open:  # close the chat box the disturbance left open, with a key that is not Kotone's
            player_key(0x0D, False)
            player_key(0x0D, True)
            time.sleep(0.3)

        # 2. letters during a protected send
        focus()
        events = send_with(disturb_letters)
        messages, game_keys, enters, still_open = chat(events)
        print(f"[letters] messages={messages} gameKeys={len(game_keys)} kotoneEnters={len(enters)} open={still_open}")
        if messages != [("team", TEXT)] or still_open:
            failures.append(f"letters: expected [('team', {TEXT!r})], got {messages} (open={still_open})")
        if len(game_keys) == 0:
            failures.append("letters: no disturbance key reached the window outside the send - the test did not run")

        # 3. Shift pressed inside the window and held
        focus()
        shift_at = {}

        def disturb_shift(stop):
            seen = len(log_lines())
            while not stop.is_set():
                fresh = parse(log_lines()[seen:])
                if any(e["kind"] == "DOWN" and e["key"] == "Return" and e.get("x") == KOTONE_TAG for e in fresh):
                    shift_at["down"] = int(time.time() * 1000)
                    player_key(0x10, False)
                    time.sleep(1.0)
                    player_key(0x10, True)
                    shift_at["up"] = int(time.time() * 1000)
                    return
                time.sleep(0.002)

        events = send_with(disturb_shift)
        messages, game_keys, enters, still_open = chat(events)
        print(f"[shift] messages={messages} kotoneEnters={[(e['ts'], e.get('shift')) for e in enters]} "
              f"shiftDownAt={shift_at.get('down')} open={still_open}")
        if messages != [("team", TEXT)] or still_open:
            failures.append(f"shift: expected [('team', {TEXT!r})], got {messages}")
        if len(enters) != 2 or any(e.get("shift") != "False" for e in enters):
            failures.append(f"shift: Kotone's Enters carried Shift: {[e['raw'] for e in enters]}")
        if not (len(enters) == 2 and enters[0]["ts"] <= shift_at.get("down", 0) <= enters[1]["ts"]):
            failures.append("shift: the Shift press did not land between Kotone's two Enters")
        shift_downs = [e["raw"] for e in events if e["kind"] == "DOWN" and "Shift" in e["key"]]
        if shift_downs:
            failures.append(f"shift: the window saw a Shift down: {shift_downs}")
        last_enter = max((i for i, e in enumerate(events) if e.get("x") == KOTONE_TAG), default=-1)
        extra = [e["raw"] for e in events[last_enter + 1:] if e.get("x") == KOTONE_TAG]
        tagged = sorted({e["key"] for e in events if e.get("x") == KOTONE_TAG})
        print(f"[shift] keys carrying Kotone's tag: {tagged}; after the last Enter: {extra}")
        if tagged != ["Packet", "Return"]:
            failures.append(f"shift: Kotone sent keys other than Enter and text: {tagged}")

        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            hook_lines = [l.strip() for l in f if "hooks installed" in l]
        print("hook log (this run is the last line):", *hook_lines[-2:], sep="\n  ")
        records = gate_records()[records_before:]
        print("inject_gate records:", [r.get("data", r).get("detail", r) for r in records])
        if len(records) < 2:
            failures.append(f"expected an inject_gate record for each protected send, got {len(records)}")
    finally:
        print("app.quit:", call("app.quit", timeout_ms=5000).get("ok"))
        with open(os.path.join(TARGET, "control.txt"), "w") as f:
            f.write("close")
        for p in (app, window):
            try:
                p.wait(15)
            except subprocess.TimeoutExpired:
                p.kill()

    if failures:
        print("FAIL")
        for f in failures:
            print(" -", f)
        raise SystemExit(1)
    print("PASS")


main()
