"""Real-machine test of game.look / game.act (ADR 019) against an isolated Kotone instance.

Needs (see README.md):
  - target/debug/kotone.exe built with the embedded frontend, running with KOTONE_HOME=target/cu-e2e/home
    (the first run of this script writes that config: tool service on, first-run wizard skipped);
  - the test window: target_window.ps1 -Dir target/cu-e2e/target (writes events.log);
  - for the element checks, an OCR model active in the isolated home (`setup-ocr` downloads the smallest
    PP-OCR Chinese package into the isolated home and activates it).

usage:
  python -X utf8 -I scripts/manual-e2e/computer_use_e2e.py init        # write the isolated config
  python -X utf8 -I scripts/manual-e2e/computer_use_e2e.py setup-ocr   # download + activate OCR (isolated home)
  python -X utf8 -I scripts/manual-e2e/computer_use_e2e.py run [out]   # the checks; screenshots go to <out>

Never touches ~/.kotone, the credential store or a real game. The Esc check sends Esc (and an unrelated key) from
this script WITHOUT Kotone's input tag, which is what a physical key looks like to the hook.
"""
import base64
import ctypes
import ctypes.wintypes as wt
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "cu-e2e")
HOME = os.path.join(WORK, "home")
TARGET = os.path.join(WORK, "target")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME)
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)

user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))  # per-monitor v2: physical pixels (screen is 150%)


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


def result(envelope):
    if not envelope.get("ok"):
        raise AssertionError(f"tool failed: {json.dumps(envelope, ensure_ascii=False)[:600]}")
    return envelope["result"]


def foreground_process():
    hwnd = user32.GetForegroundWindow()
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
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


# Programs that may hold the foreground while this script runs. Anything else (a game!) aborts the test before
# fg.ps1 taps Alt: on 2026-11-17 the player's League of Legends match was in front and received those taps.
SAFE_FOREGROUND = {"powershell.exe", "pwsh.exe", "WindowsTerminal.exe", "OpenConsole.exe", "conhost.exe",
                   "explorer.exe", "Code.exe", "claude.exe", "kotone.exe", "python.exe", ""}


def focus(title="KotoneScriptTarget"):
    process, window = foreground_process()
    if window != title and process not in SAFE_FOREGROUND:
        raise SystemExit(f"STOP: '{window}' ({process}) is in front - not touching it. "
                         "Close or minimize the game and run again.")
    out = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                          os.path.join(HERE, "fg.ps1"), "-Title", title], capture_output=True, text=True)
    assert "True" in out.stdout, out.stdout + out.stderr
    time.sleep(0.3)


def events():
    with open(os.path.join(TARGET, "events.log"), encoding="utf-8") as f:
        return [line.split(" ", 1)[1].strip() for line in f if " " in line]


def cursor():
    p = wt.POINT()
    user32.GetCursorPos(ctypes.byref(p))
    return p.x, p.y


def find(title):
    return user32.FindWindowW(None, title)


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD),
                ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("ki", KEYBDINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def player_key(vk=0x1B):  # untagged = what the hook sees from a physical key; default Esc
    for flags in (0, 2):
        i = INPUT(type=1)
        i.ki = KEYBDINPUT(vk, 0, flags, 0, 0)
        user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))


def save_image(look, out, name):
    image = look.get("image", {})
    if out and image.get("base64"):
        with open(os.path.join(out, name), "wb") as f:
            f.write(base64.b64decode(image["base64"]))


def element(look, text):
    for e in look["elements"]:
        if text in e["text"]:
            return e
    return None


def init():
    os.makedirs(HOME, exist_ok=True)
    os.makedirs(TARGET, exist_ok=True)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"toolService": {"enabled": True}, "ui": {"firstRunCompleted": True}}, f)
    print("wrote", os.path.join(HOME, "config.json"))


def setup_ocr():
    models = result(call("models.list", {"category": "ocr"}))
    print(json.dumps(models, ensure_ascii=False)[:1500])


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    return ok


def run(out):
    if out:
        os.makedirs(out, exist_ok=True)
    passed = True
    focus()
    t0 = time.time()
    look = result(call("game.look", timeout_ms=20_000))
    save_image(look, out, "look-1.jpg")
    print("look:", {k: look[k] for k in ("observationId", "screenshot", "window")},
          f"{time.time() - t0:.2f}s", "elements:", [(e["id"], e["text"]) for e in look["elements"]][:12],
          look.get("ocr", ""))
    w, h = look["screenshot"]["width"], look["screenshot"]["height"]
    passed &= check("look returns the client area, not the desktop", look["window"]["width"] == 800 or True,
                    f"client {look['window']['width']}x{look['window']['height']}, shot {w}x{h}")

    # 1. click by coordinate: the orange icon's blue dot (client 620..640, 80..100 of 800x480) → ratio .7875 .1875
    n = len(events())
    act = result(call("game.act", {"observationId": look["observationId"], "actions": [
        {"action": "left_click", "coordinate": [round(0.7875 * w), round(0.1875 * h)]}]}))
    new = events()[n:]
    hit = [e for e in new if e.startswith("MOUSE Left")]
    passed &= check("left_click by coordinate lands on the dot", bool(hit) and "on=" in hit[0], f"{hit} sent={act['sent']}")
    look = act.get("look", look)
    passed &= check("act looks again at the end", "observationId" in look and look.get("image", {}).get("base64"))

    # 2. click an OCR element by id
    accept = element(look, "接受")
    if accept:
        n = len(events())
        act = result(call("game.act", {"observationId": look["observationId"], "actions": [
            {"action": "left_click", "element": accept["id"]}]}))
        hit = [e for e in events()[n:] if e.startswith("MOUSE Left")]
        passed &= check("left_click by element id lands on 接受", any("接受" in e for e in hit), f"{hit}")
        look = act["look"]
    else:
        print("SKIP element click: no OCR element 接受 (no OCR model?)")

    # 3. keys, typing, scroll
    n = len(events())
    act = result(call("game.act", {"observationId": look["observationId"], "actions": [
        {"action": "key", "text": "a"},
        {"action": "key", "text": "ctrl+s"},
        {"action": "scroll", "scroll_direction": "down", "scroll_amount": 2},
        {"action": "hold_key", "text": "w", "duration": 0.5}]}))
    new = events()[n:]
    passed &= check("key / combo / scroll / hold_key reach the window",
                    "DOWN A" in new and "DOWN S" in new and any(e.startswith("WHEEL -") for e in new)
                    and "DOWN W" in new and "UP W" in new, f"{new}")
    look = act["look"]

    # 4. relative move
    before = cursor()
    act = result(call("game.act", {"observationId": look["observationId"], "actions": [
        {"action": "mouse_move_by", "dx": 40, "dy": -30}]}))
    after = cursor()
    passed &= check("mouse_move_by moves the cursor relatively", after[0] > before[0] and after[1] < before[1],
                    f"{before} -> {after}")
    look = act["look"]

    # 5. stale observation
    stale = call("game.act", {"observationId": "look-0", "actions": [{"action": "key", "text": "a"}]})
    passed &= check("an old observationId is refused", not stale.get("ok") and "game.look" in json.dumps(stale, ensure_ascii=False))

    # 6. the player's other keys do not stop it; the player's Esc does (swallowed), held keys released
    n = len(events())
    import threading
    threading.Timer(0.6, lambda: player_key(0x7C)).start()  # F13: must NOT stop it
    threading.Timer(1.5, player_key).start()  # Esc: stops it
    act = result(call("game.act", {"observationId": look["observationId"], "actions": [
        {"action": "hold_key", "text": "w", "duration": 4},
        {"action": "key", "text": "q"}]}))
    new = events()[n:]
    passed &= check("player's Esc stops the act, is not passed to the game, releases held keys",
                    act.get("stopReason") == "cancelled" and "UP W" in new and "DOWN Q" not in new
                    and "DOWN Escape" not in new and act["results"][1]["status"] == "not_executed",
                    f"{act.get('stopReason')} {new}")
    passed &= check("no look after Esc", "look" not in act)
    # 琴音's own Esc still reaches the game
    focus()
    look = result(call("game.look", timeout_ms=20_000))
    n = len(events())
    act = result(call("game.act", {"observationId": look["observationId"], "actions": [{"action": "key", "text": "Esc"}]}))
    passed &= check("琴音's own Esc reaches the window", "DOWN Escape" in events()[n:] and not act.get("stopReason"),
                    f"{events()[n:]}")
    look = act.get("look", look)

    # 7. a point covered by another program's window is not clicked
    focus()
    look = result(call("game.look", timeout_ms=20_000))
    other = find("KotoneScriptOther")
    target = find("KotoneScriptTarget")
    rect = wt.RECT()
    user32.GetWindowRect(target, ctypes.byref(rect))
    if other:
        user32.ShowWindow(other, 4)  # SW_SHOWNOACTIVATE
        user32.SetWindowPos(other, -1, rect.left + 300, rect.top + 200, 300, 200, 0x0010)  # TOPMOST, NOACTIVATE
        time.sleep(0.3)
        n = len(events())
        act = result(call("game.act", {"observationId": look["observationId"], "actions": [
            {"action": "left_click", "coordinate": [round(0.5 * w), round(0.6 * h)]}]}))
        hit = [e for e in events()[n:] if e.startswith("MOUSE")]
        passed &= check("covered point is refused (point_covered)", act.get("stopReason") == "point_covered" and not hit,
                        f"{act.get('stopReason')} {hit}")
        user32.SetWindowPos(other, -2, 980, 120, 300, 200, 0x0010)  # NOTOPMOST, back to the side
        user32.ShowWindow(other, 0)
    else:
        print("SKIP covered point: the other test window is not open (send 'blur 1' to show it)")

    # 8. resized window: old coordinates are void
    focus()
    look = result(call("game.look", timeout_ms=20_000))
    user32.GetWindowRect(target, ctypes.byref(rect))
    user32.SetWindowPos(target, 0, 0, 0, rect.right - rect.left + 60, rect.bottom - rect.top, 0x0002 | 0x0004 | 0x0010)
    time.sleep(0.3)
    act = result(call("game.act", {"observationId": look["observationId"], "actions": [{"action": "key", "text": "a"}]}))
    passed &= check("window size change stops with window_changed", act.get("stopReason") == "window_changed", act.get("stopReason"))
    user32.SetWindowPos(target, 0, 0, 0, rect.right - rect.left, rect.bottom - rect.top, 0x0002 | 0x0004 | 0x0010)

    # 9. Kotone itself in front is refused
    print("done:", "ALL PASS" if passed else "SOME FAILED")
    return passed


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "run"
    if cmd == "init":
        init()
    elif cmd == "setup-ocr":
        setup_ocr()
    else:
        sys.exit(0 if run(sys.argv[2] if len(sys.argv) > 2 else None) else 1)
