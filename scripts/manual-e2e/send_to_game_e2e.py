"""Real-machine reproduction for ADR 041 phase 1 (send to game): what `game_chat.send` does today when the player
holds Shift, when two sends overlap, with a multi-line text and with a channel.

Written BEFORE the fix (ADR 041 phase 1, first part): today every case below is expected to FAIL; after phase 1
they should all PASS, and this script becomes the real-machine regression test.

Everything is isolated: KOTONE_HOME=target/send-to-game-e2e/home, the test window target_window.ps1 -Chat, no game,
no player config. The isolated home has one game profile "e2e-chat" that matches powershell.exe (the test window's
process) with League of Legends' chat keys: team = Enter, all = Shift+Enter, send = Enter. The "player" is this
script: SendInput WITHOUT Kotone's tag (dwExtraInfo 0), which the hook treats exactly like a physical key. The test
window logs every key with its Shift state and dwExtraInfo (Kotone's keys carry 4B544E49); this script rebuilds what a
chat box would have received: Enter opens it (Shift+Enter = "all"), characters go in, Enter sends it.

`game_chat.send` is called straight over the tool service's HTTP API (descriptor in <home>/run), so two calls can
start within a millisecond of each other (a `kotone.exe tools call` process start would add 100+ ms of jitter).

  0. warm-up: one plain send (the first send after start-up is not protected by ADR 032's gate; this primes it);
  1. shift-short: the player holds Shift, game_chat.send is called, Shift is let go 300 ms later.
     Expected (ADR 041 D5): Kotone waits for Shift to come up, the message lands in the TEAM channel.
     Suspected today (audit #2): Kotone's Enter arrives as Shift+Enter -> the ALL channel;
  2. shift-long: Shift held for 2.5 s (longer than the wait limit).
     Expected: game_chat.send fails with a reason about the held key and Kotone presses nothing.
     Suspected today: typed into the ALL channel;
  3. overlap: two game_chat.send calls start at the same moment.
     Expected (ADR 041 D4): two intact team messages, one after the other.
     Suspected today (audit #3): the two Enter-text-Enter sequences interleave;
  4. multi-line: "line one\\nline two". Expected (D1/D5): two messages in order. Today: rejected ("含有换行");
  5. channel: {"channel": "all"}. Expected (D1/D6): one message in the ALL channel and a result that says
     route/channel/sent. Today: there is no channel parameter.

usage:
  python -X utf8 -I scripts/manual-e2e/send_to_game_e2e.py [path-to-kotone.exe]
    (default target/send-to-game-e2e/bin/kotone.exe, refreshed from target/debug/kotone.exe - a copy, so a running
     test instance is never mistaken for the player's `pnpm dev`)

Stops before sending anything if a game is running or something other than a terminal / the test window is in front,
and checks the test window is in front again right before every send and every "player" key.
Not covered: the voice-input path (needs a microphone or VB-CABLE, see voice_input_real.py), game.act / automation
programs (computer_use_e2e.py), a real game.
"""
import ctypes
import ctypes.wintypes as wt
import glob
import json
import os
import shutil
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "send-to-game-e2e")
HOME = os.path.join(WORK, "home")
TARGET = os.path.join(WORK, "target")
DEBUG_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
BIN = os.path.join(WORK, "bin", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
KOTONE_TAG = "4B544E49"
PROFILE_ID = "e2e-chat"

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

GAMES = ("league of legends", "leagueclient", "riotclient", "dota2", "cs2", "yuanshen", "genshinimpact",
         "valorant", "starrail", "zenlesszonezero")
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


def test_window_in_front():
    if foreground_process()[1] != "KotoneScriptTarget":
        raise SystemExit(f"STOP: the test window is not in front ({foreground_process()}) - not sending.")


def focus():
    guard()
    out = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                          os.path.join(HERE, "fg.ps1"), "-Title", "KotoneScriptTarget"], capture_output=True, text=True)
    assert "True" in out.stdout, out.stdout + out.stderr
    time.sleep(0.4)
    test_window_in_front()


# ---------- the tool service, straight over HTTP ----------

def descriptor():
    files = glob.glob(os.path.join(HOME, "run", "tool-service-s*.json"))
    if not files:
        return None
    with open(files[0], encoding="utf-8") as f:
        return json.load(f)


def call(tool, args=None, timeout_ms=30_000):
    d = descriptor()
    if d is None:
        return {"ok": False, "error": "no descriptor"}
    body = json.dumps({"tool": tool, "arguments": args if args is not None else {}, "timeoutMs": timeout_ms},
                      ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(f"http://{d['address']}/v1/tools/call", data=body, method="POST",
                                 headers={"Authorization": f"Bearer {d['token']}",
                                          "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout_ms / 1000 + 5) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        try:
            return json.loads(e.read().decode("utf-8"))
        except Exception:  # noqa: BLE001
            return {"ok": False, "status": e.code}
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "error": repr(e)}


def send(args):
    test_window_in_front()
    return call("game_chat.send", args)


# ---------- the "player" ----------

class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


SHIFT_DOWN = [False]


def player_key(vk, up):
    """One key event WITHOUT Kotone's tag: to the hook this is the player's key. Key-ups are always sent (never leave
    a key stuck); downs only while the test window is in front."""
    if not up and foreground_process()[1] != "KotoneScriptTarget":
        raise SystemExit("STOP: the test window lost the foreground - not sending more keys.")
    i = INPUT(type=1)
    i.ki = KEYBDINPUT(vk, user32.MapVirtualKeyW(vk, 0), 2 if up else 0, 0, 0)
    assert user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT)) == 1
    if vk == 0x10:
        SHIFT_DOWN[0] = not up


# ---------- what the chat box received ----------

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
    """[(channel, text)] a chat box would have received, the characters typed while it was closed (they went to the
    game), Kotone's tagged key downs, and whether the box was left open."""
    messages, stray, tagged = [], [], []
    open_channel, units = None, []
    for ev in events:
        if ev["kind"] == "DOWN" and ev.get("x") == KOTONE_TAG and ev["key"] != "Packet":
            tagged.append(f"{ev['key']}{'+shift' if ev.get('shift') == 'True' else ''}")
        if ev["kind"] == "DOWN" and ev["key"] == "Return":
            if open_channel is None:
                open_channel, units = ("all" if ev.get("shift") == "True" else "team"), []
            else:
                messages.append((open_channel, b"".join(u.to_bytes(2, "little") for u in units).decode("utf-16-le")))
                open_channel = None
        elif ev["kind"] == "CHAR" and ev["unit"] >= 0x20:
            (units if open_channel is not None else stray).append(ev["unit"])
    stray_text = b"".join(u.to_bytes(2, "little") for u in stray).decode("utf-16-le", errors="replace")
    return messages, stray_text, tagged, open_channel


def round_of(action, settle=1.2):
    """Run `action()` (which sends), wait for the window to log everything, return (action's value, chat(...))."""
    start = len(log_lines())
    value = action()
    time.sleep(settle)
    return value, chat(parse(log_lines()[start:]))


def close_if_open(still_open):
    if still_open:  # close a chat box the round left open, with a key that is not Kotone's
        player_key(0x0D, False)
        player_key(0x0D, True)
        time.sleep(0.3)


def brief(envelope):
    if envelope.get("ok"):
        return {"ok": True, "result": envelope.get("result")}
    return {"ok": False, "error": envelope.get("error")}


# ---------- setup ----------

def prepare_exe():
    if len(sys.argv) > 1:
        return os.path.abspath(sys.argv[1])
    if not os.path.exists(DEBUG_EXE) and not os.path.exists(BIN):
        raise SystemExit("no target/debug/kotone.exe - build it first (cargo build -p kotone-tauri)")
    if os.path.exists(DEBUG_EXE) and (not os.path.exists(BIN) or os.path.getmtime(DEBUG_EXE) > os.path.getmtime(BIN)):
        os.makedirs(os.path.dirname(BIN), exist_ok=True)
        shutil.copy2(DEBUG_EXE, BIN)
    return BIN


def write_home():
    os.makedirs(os.path.join(HOME, "profiles"), exist_ok=True)
    os.makedirs(TARGET, exist_ok=True)
    for name in ("ready.txt", "events.log", "control.txt"):
        if os.path.exists(os.path.join(TARGET, name)):
            os.remove(os.path.join(TARGET, name))
    config = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "legacyUpgrade": "notNeeded"},
        "adminPromptDismissed": True,
        "toolService": {"enabled": True},
        "activeProfileId": PROFILE_ID,
        "assistant": {"speechVolume": 0, "wake": {"enabled": False}},
    }
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, ensure_ascii=False, indent=1)
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


def main():
    guard()
    exe = prepare_exe()
    write_home()
    app = subprocess.Popen([exe, "--background"], env=ENV)
    window = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                               os.path.join(HERE, "target_window.ps1"), "-Dir", TARGET, "-Chat"])
    results = []  # (case, passed, detail)
    try:
        wait_for(lambda: os.path.exists(os.path.join(TARGET, "ready.txt")), 30, "test window")
        wait_for(lambda: call("app.status", timeout_ms=3000).get("ok"), 90, "tool service")

        # 0. warm-up (also primes ADR 032's gate)
        focus()
        env, (messages, stray, tagged, still_open) = round_of(lambda: send({"text": "预热", "send": True}))
        print(f"[warm-up] {brief(env)} messages={messages} stray={stray!r} tagged={tagged}")
        close_if_open(still_open)
        if messages != [("team", "预热")]:
            raise SystemExit("warm-up did not reach the test window as one team message - the checks below mean "
                             "nothing (is the exe running at low integrity? see README 准备 step 2)")

        # 1. Shift held, let go after 300 ms
        focus()
        text = "按住 Shift 也发到队伍"

        def shift_short():
            player_key(0x10, False)
            time.sleep(0.05)
            releaser = threading.Timer(0.3, lambda: player_key(0x10, True))
            releaser.start()
            try:
                return send({"text": text, "send": True})
            finally:
                releaser.join()

        env, (messages, stray, tagged, still_open) = round_of(shift_short)
        print(f"[shift-short] {brief(env)} messages={messages} stray={stray!r} tagged={tagged} open={still_open}")
        close_if_open(still_open)
        results.append(("shift-short", env.get("ok") and messages == [("team", text)] and not still_open,
                        f"expected [('team', {text!r})], got {messages}"))

        # 2. Shift held for 2.5 s (longer than the wait limit)
        focus()
        text = "按住不放就不发"

        def shift_long():
            pressed = time.time()
            player_key(0x10, False)
            time.sleep(0.05)
            try:
                return send({"text": text, "send": True})
            finally:
                time.sleep(max(0.0, pressed + 2.5 - time.time()))
                player_key(0x10, True)

        start = time.time()
        env, (messages, stray, tagged, still_open) = round_of(shift_long)
        print(f"[shift-long] {brief(env)} after {time.time() - start:.1f}s messages={messages} tagged={tagged} "
              f"open={still_open}")
        close_if_open(still_open)
        results.append(("shift-long", (not env.get("ok")) and not tagged,
                         f"expected a failure and no Kotone key, got ok={env.get('ok')} tagged={tagged} "
                         f"messages={messages}"))

        # 3. two sends at the same moment
        focus()
        texts = ("第一条：我去下路", "第二条：小龙要刷了")
        barrier = threading.Barrier(2)
        out = {}

        def overlapped():
            def one(i):
                barrier.wait()
                out[i] = send({"text": texts[i], "send": True})
            workers = [threading.Thread(target=one, args=(i,)) for i in (0, 1)]
            for w in workers:
                w.start()
            for w in workers:
                w.join(30)
            return out

        _, (messages, stray, tagged, still_open) = round_of(overlapped, settle=1.5)
        print(f"[overlap] {[brief(out.get(i, {})) for i in (0, 1)]}")
        print(f"[overlap] messages={messages} stray={stray!r} tagged={tagged} open={still_open}")
        close_if_open(still_open)
        results.append(("overlap", sorted(messages) == sorted(("team", t) for t in texts) and not stray
                        and not still_open, f"expected both texts as two team messages, got {messages}, "
                        f"stray={stray!r}, open={still_open}"))

        # 4. multi-line
        focus()
        lines = ("第一行：亚索 2/0/1", "第二行：劫 0/2/0")
        env, (messages, stray, tagged, still_open) = round_of(lambda: send({"text": "\n".join(lines), "send": True}))
        print(f"[multi-line] {brief(env)} messages={messages} stray={stray!r} open={still_open}")
        close_if_open(still_open)
        results.append(("multi-line", env.get("ok") and messages == [("team", t) for t in lines],
                        f"expected two team messages in order, got ok={env.get('ok')} {messages}"))

        # 5. channel
        focus()
        text = "全体：gg"
        env, (messages, stray, tagged, still_open) = round_of(
            lambda: send({"text": text, "send": True, "channel": "all"}))
        print(f"[channel] {brief(env)} messages={messages} tagged={tagged} open={still_open}")
        close_if_open(still_open)
        said = json.dumps(env.get("result"), ensure_ascii=False) if env.get("ok") else ""
        results.append(("channel", env.get("ok") and messages == [("all", text)] and '"all"' in said,
                        f"expected [('all', {text!r})] and a result naming the channel, got ok={env.get('ok')} "
                        f"{messages} result={said[:200]}"))
    finally:
        if SHIFT_DOWN[0]:
            player_key(0x10, True)
        print("app.quit:", call("app.quit", timeout_ms=5000).get("ok"))
        with open(os.path.join(TARGET, "control.txt"), "w") as f:
            f.write("close")
        for p in (app, window):
            try:
                p.wait(15)
            except subprocess.TimeoutExpired:
                p.kill()

    print()
    for case, passed, detail in results:
        print(f"{'PASS' if passed else 'FAIL'} {case}" + ("" if passed else f": {detail}"))
    if not all(passed for _, passed, _ in results):
        raise SystemExit(1)
    print("PASS")


main()
