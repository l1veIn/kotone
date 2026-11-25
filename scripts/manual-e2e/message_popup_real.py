"""The message pop-up (ADR 039 D4) on a real Kotone: where it sits in the bottom-right corner, and whether clicking it
works (shared-resources audit section 2 #10, and a finding from the focus work; 2026-11-25).

1. #10 “右下角挤” (code reading only): the pop-up, the voice input overlay at its bottom-right preset, the bubble and the
   danmaku all use the bottom-right corner; only the pop-up and the bubble keep out of each other's way, and the pop-up
   and the overlay sit the same distance from the edges, so they overlap.
2. The pop-up window was `focusable: false` (WS_EX_NOACTIVATE). On the voice input overlay the focus work found that
   WebView2 then gets no mouse event at all, so clicking the pop-up (open the chat at that card) may not work either.

Setup: the fake LoL client timeline from game_events_real.py (0 s Lobby → 8 s ChampSelect → 16 s InProgress, loading →
60 s EndOfGame → 70 s Lobby). The only thing it makes Kotone do here is post the two auto cards (开局战绩 at loading,
这局总结 after the end), each with a pop-up. Isolated KOTONE_HOME=target/message-popup-real/home, `--background`, no AI
key (auto cards never call the model), input tools denied, the voice input overlay at its bottom-right preset with
visibility always (online recognition that cannot reach anything — a DeepSeek connection without a key, pointed at the
local discard port, as in focus_steal_real.py — so the runtime runs and the overlay shows), KOTONE_POPUP_CAPTURABLE=1 so
the corner can be screenshot. A test window (target_window.ps1) stands in for the game and is in front.

Checks:
M1 (#10) the overlay is shown at the bottom-right.
M2 the first pop-up shows; while it appears the foreground stays the test window, which never loses focus and gets no
   key (the pop-up must still never take the game's foreground).
M3 (#10) the pop-up and the overlay windows do not overlap (window rects, physical pixels); the corner is screenshot to
   target/plugin-shots/round8/corner-*.png.
M4 (click) one left click on the pop-up's card: the pop-up page gets it (pointerdown + click), Kotone comes to the front
   on the chat page with that card in view, the pop-up goes away, and the test window got no key from Kotone (opening
   the chat must not tap Alt into the game). Then the test window gets its foreground back (Alt + SetForegroundWindow,
   like fg.ps1).
M5 (#10, the corner reflows) the overlay is hidden (visibility never) before the second pop-up, which then sits in the
   corner; the overlay is shown again (visibility always): within 1 s the pop-up is out of its way (no overlap).
M6 (click ×) one click on the second pop-up's ×: it goes away and the foreground is back on the test window within 1 s
   after the release; no key.
System input: the mouse in M4 / M6 (only when the test window is in front and the pop-up is the window under the point)
and Alt + SetForegroundWindow after M4 (only when the foreground is the test window, this Kotone or a terminal). The
cursor is put back. Nothing is typed. Needs a debug build with the embedded frontend (`pnpm -C apps/desktop build:apps`,
`build:web`, then `cargo build -p kotone-tauri --features tauri/custom-protocol`); the exe is copied to
target/message-popup-real/bin first. Takes target/tmp/real.lock; only starts when a terminal is in front and no game
runs; at the end the terminal gets the foreground back.
usage: python -X utf8 -I scripts/manual-e2e/message_popup_real.py [--exe PATH] [--probe DIR] [--gamedata DIR] [--keep]
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import focus_steal_real as fs  # noqa: E402  (also makes this process per-monitor DPI aware: physical pixels)
import game_events_real as ge  # noqa: E402

lt, bt = ge.lt, ge.bt
user32 = fs.user32
REPO = fs.REPO
WORK = os.path.join(REPO, "target", "message-popup-real")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")
SHOTS = os.path.join(REPO, "target", "plugin-shots", "round8")
PORT = 9379
POPUP_TITLE = "Kotone 消息提示"
SETTINGS_TITLE = "Kotone 设置"
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def setup_paths():
    fs.WORK, fs.HOME, fs.BIN = WORK, HOME, BIN
    fs.TARGET = os.path.join(WORK, "target")
    fs.EXE = os.path.join(BIN, "kotone.exe")
    fs.DEVTOOLS = PORT
    bt.WORK, bt.HOME, bt.EXE = WORK, HOME, fs.EXE


def write_config():
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": lt.DENIED}, "matchRecap": False},
                   "sttEngine": "deepseek-asr", "activeModelId": "deepseek-asr",
                   "connections": [{"id": "message-popup-real", "displayName": "message-popup-real",
                                    "provider": "deepseek", "baseUrl": "http://127.0.0.1:9/",
                                    "model": "deepseek-chat"}],
                   "overlay": {"visibility": "always", "position": "bottom_right"}}, f)


def tool(name, args=None):
    code, value = fs.tool(name, args)
    if code != 0 or not value.get("ok"):
        raise RuntimeError(f"{name}: {json.dumps(value, ensure_ascii=False)[:300]}")
    return value["result"]


def lol_phase():
    try:
        playing = tool("game.state", {"game": "lol"}).get("playing") or []
    except RuntimeError:
        return None
    return playing[0]["phase"] if playing else None


def wait_until(predicate, timeout, step=0.2):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return None


def window(app_pid, title):
    """(hwnd, visible, rect) of this Kotone's window with that title, or None."""
    hwnd = fs.find_window(app_pid, title)
    if not hwnd:
        return None
    return hwnd, bool(user32.IsWindowVisible(hwnd)), fs.rect(hwnd)


def overlaps(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def shot(name, rects):
    left = min(r[0] for r in rects) - 16
    top = min(r[1] for r in rects) - 16
    right = max(r[2] for r in rects) + 16
    bottom = max(r[3] for r in rects) + 16
    try:
        from PIL import ImageGrab
    except ImportError:
        print("   (no Pillow: no screenshot)")
        return
    os.makedirs(SHOTS, exist_ok=True)
    path = os.path.join(SHOTS, f"corner-{name}.png")
    ImageGrab.grab(bbox=(left, top, right, bottom), all_screens=True).save(path)
    print(f"   screenshot {path}")


def log_text():
    try:
        return open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return ""


# 小窗页面里记下收到的鼠标事件；顺便报卡片和 × 在窗口里的位置（CSS 像素）和缩放
POPUP_WATCH = """
const w = window;
if (!w.__popupTest) {
  w.__popupTest = { events: [] };
  for (const type of ["pointerenter", "pointerdown", "pointerup", "click"]) window.addEventListener(type, () => w.__popupTest.events.push(type), true);
}
const box = (sel) => { const r = document.querySelector(sel)?.getBoundingClientRect(); return r ? [r.left, r.top, r.width, r.height] : null; };
const seen = { events: w.__popupTest.events.slice(), card: box('[data-testid=message-popup]'),
  close: box('[data-testid=message-popup-close]'), dpr: window.devicePixelRatio };
w.__popupTest.events.length = 0;
return seen;
"""

CARD_IN_VIEW = """
const card = document.querySelector('[data-testid=auto-card][data-key="%s"]');
if (!card) return { found: false, cards: [...document.querySelectorAll('[data-testid=auto-card]')].map((c) => c.dataset.key) };
const r = card.getBoundingClientRect();
return { found: true, inView: r.bottom > 0 && r.top < window.innerHeight && r.height > 0, top: r.top, height: r.height,
  viewport: window.innerHeight };
"""


def popup_page(body=POPUP_WATCH):
    return fs.page_eval("#/message-popup", body).get("ok") or {}


def click_at(app_pid, target, x, y, what, popup):
    """One left click at (x, y) on the pop-up (the mouse goes there first and rests 0.4 s). Returns the release time
    on the sampler's clock."""
    fs.guard_input(app_pid, target.hwnd, strict=True)
    under = fs.root_at(x, y)
    if under != popup:
        raise SystemExit(f"STOP: the window under {what} ({x},{y}) is {fs.describe(under)}, not the pop-up; not clicking")
    fs.mouse_abs(x, y, 0)
    time.sleep(0.4)
    fs.guard_input(app_pid, target.hwnd, strict=True)
    fs.mouse_abs(x, y, fs.LEFTDOWN)
    time.sleep(0.08)
    fs.mouse_abs(x, y, fs.LEFTUP)
    return time.time()


def keys_since_mark(target):
    return [e for e in target.since_mark() if " DOWN " in f" {e} " or " UP " in f" {e} "]


def run(app, target, ui):
    pid = app.pid
    # M1：悬浮窗在右下角
    overlay = wait_until(lambda: (lambda w: w if w and w[1] else None)(window(pid, fs.OVERLAY_TITLE)), 30)
    if not check("M1: the voice input overlay is shown", overlay, str(overlay[2]) if overlay else "no visible overlay window"):
        return
    menu = wait_until(lambda: lol_phase() == "menu", 40, 1)
    check("setup: the fake client is connected (lol in menu)", menu, str(lol_phase()))
    fs.give_back_focus(pid, target)
    if not check("setup: the test window is in front", fs.hval(user32.GetForegroundWindow()) == target.hwnd,
                 fs.describe(user32.GetForegroundWindow())):
        return

    # M2：第一个消息提示出来时，前台一直是测试窗口
    target.set_mark()
    sampler = fs.Sampler()
    lt.REC.t0 = time.time()
    first = wait_until(lambda: (lambda w: w if w and w[1] else None)(window(pid, POPUP_TITLE)), 60)
    time.sleep(1.0)  # 滑入 180 ms；再多看一会儿前台
    changes = sampler.finish()
    skipped = next((line for line in log_text().splitlines() if "消息提示：没弹" in line), None)
    if not check("M2: the first message pop-up shows", first, str(first[2]) if first else skipped or log_text()[-600:]):
        return
    popup = first[0]
    ex = user32.GetWindowLongPtrW(popup, -20)
    print(f"   pop-up {fs.describe(popup)} at {first[2]}, exstyle {ex:#x} "
          f"(WS_EX_NOACTIVATE: {bool(ex & fs.WS_EX_NOACTIVATE)}, topmost: {bool(ex & 0x8)})")
    fs.foreground_checks("M2", changes, target, pid, target.since_mark)

    # M3：和悬浮窗不叠
    overlay = window(pid, fs.OVERLAY_TITLE)
    popup_rect = fs.rect(popup)
    check("M3: the pop-up and the overlay do not overlap", not overlaps(popup_rect, overlay[2]),
          f"pop-up {popup_rect}, overlay {overlay[2]}")
    shot("first", [popup_rect, overlay[2]])

    # M4：点卡片
    seen = popup_page() if ui else {}
    print(f"   pop-up page: {json.dumps(seen)}")
    cursor = fs.wt.POINT()
    user32.GetCursorPos(fs.ctypes.byref(cursor))
    close_offset = None
    try:
        if seen.get("close") and seen.get("dpr"):
            x, y, w, h = seen["close"]
            close_offset = ((x + w / 2) * seen["dpr"], (y + h / 2) * seen["dpr"])
        left, top, right, bottom = fs.rect(popup)
        if seen.get("card") and seen.get("dpr"):
            x, y, w, h = seen["card"]
            # 卡片左边三分之一（头像和字那里），离 × 远一点
            cx, cy = int(left + (x + w / 3) * seen["dpr"]), int(top + (y + h / 2) * seen["dpr"])
        else:
            cx, cy = (left + right) // 2, (top + bottom) // 2
        key = next((c["card"]["key"] for c in conversation_cards() if c["card"]["key"].startswith("opening:")), "")
        target.set_mark()
        sampler = fs.Sampler()
        released = click_at(pid, target, cx, cy, "the card", popup) - sampler.t0
        front = wait_until(lambda: (lambda h: h if fs.window_pid(h) == pid and fs.title_of(h) == SETTINGS_TITLE
                                    else None)(fs.hval(user32.GetForegroundWindow())), 5)
        time.sleep(1.0)
        changes = sampler.finish()
        for t, _, d in changes:
            print(f"    {t:6.2f}s foreground: {d}")
        if ui:
            seen = popup_page()
            check("M4: the pop-up page got the click (pointerdown + click)",
                  "pointerdown" in seen.get("events", []) and "click" in seen.get("events", []), json.dumps(seen))
        check("M4: Kotone's window comes to the front", front, fs.describe(user32.GetForegroundWindow()))
        now = window(pid, POPUP_TITLE)
        check("M4: the pop-up went away", now and not now[1], str(now))
        if ui and front:
            view = fs.page_eval("#/settings", CARD_IN_VIEW % key).get("ok") or {}
            check(f"M4: the chat shows that card ({key})", view.get("found") and view.get("inView"), json.dumps(view))
        keys = keys_since_mark(target)
        check("M4: the test window got no key from Kotone (no Alt tap while opening the chat)", not keys,
              "; ".join(keys))
        print(f"   released at {released:.2f}s")
        if not fs.give_back_focus(pid, target):
            raise SystemExit("STOP: could not give the test window its foreground back")

        # M5：悬浮窗先藏起来，第二个消息提示贴着角落；悬浮窗再出来时，消息提示让开
        tool("settings.update", {"patch": {"overlay": {"visibility": "never"}}})
        hidden = wait_until(lambda: (lambda w: w is None or not w[1])(window(pid, fs.OVERLAY_TITLE)), 10)
        check("M5: the overlay is hidden (visibility never)", hidden, str(window(pid, fs.OVERLAY_TITLE)))
        fs.give_back_focus(pid, target)
        second = wait_until(lambda: (lambda w: w if w and w[1] else None)(window(pid, POPUP_TITLE)), 90)
        if not check("M5: the second message pop-up shows", second, str(second[2]) if second else log_text()[-600:]):
            return
        time.sleep(0.4)
        alone = fs.rect(popup)
        print(f"   second pop-up at {alone} (overlay hidden)")
        tool("settings.update", {"patch": {"overlay": {"visibility": "always"}}})
        shown = wait_until(lambda: (lambda w: w if w and w[1] else None)(window(pid, fs.OVERLAY_TITLE)), 5, 0.05)
        if check("M5: the overlay is shown again", shown, str(window(pid, fs.OVERLAY_TITLE))):
            apart = wait_until(lambda: not overlaps(fs.rect(popup), fs.rect(shown[0])), 1.0, 0.05)
            check("M5: within 1 s the pop-up is out of the overlay's way", apart,
                  f"pop-up {fs.rect(popup)}, overlay {fs.rect(shown[0])}")
            time.sleep(0.6)  # 悬浮窗的胶囊淡入完再截
            shot("reflow", [fs.rect(popup), fs.rect(shown[0])])
        # M6：点 ×
        if not close_offset:
            print("SKIP M6 — no position for × (no embedded frontend)")
            return
        if not (window(pid, POPUP_TITLE) or (0, False))[1]:
            check("M6: the second pop-up is still there to click ×", False, "it went away by itself already")
            return
        left, top, _, _ = fs.rect(popup)
        x, y = int(left + close_offset[0]), int(top + close_offset[1])
        target.set_mark()
        sampler = fs.Sampler()
        released = click_at(pid, target, x, y, "×", popup) - sampler.t0
        gone = wait_until(lambda: not (window(pid, POPUP_TITLE) or (0, False))[1], 2, 0.05)
        time.sleep(1.5)
        check("M6: × closes the pop-up", gone, str(window(pid, POPUP_TITLE)))
        fs.returned_checks("M6 (×)", sampler.finish(), released, target)
    finally:
        user32.SetCursorPos(cursor.x, cursor.y)


def conversation_cards():
    """Auto cards in the current conversation (via the settings page)."""
    state = fs.invoke("agent_state", {}).get("ok") or {}
    cid = state.get("current")
    if not cid:
        return []
    conv = fs.invoke("agent_conversation", {"id": cid}).get("ok") or {}
    return [m for m in conv.get("messages", []) if m.get("role") == "notice" and m.get("card")]


def main():
    fs.guard_start()
    started_from = fs.hval(user32.GetForegroundWindow())
    src = (os.path.abspath(sys.argv[sys.argv.index("--exe") + 1]) if "--exe" in sys.argv
           else os.path.join(REPO, "target", "debug", "kotone.exe"))
    if not os.path.exists(src):
        raise SystemExit(f"missing {src}: build it first (see the docstring)")
    try:
        os.makedirs(fs.LOCK)
    except FileExistsError:
        raise SystemExit(f"{fs.LOCK} exists: another real-machine test is running (remove it if stale)")
    setup_paths()
    app = target = server = None
    try:
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        shutil.copy2(src, fs.EXE)
        write_config()
        lt.REC = ge.Timeline(lt.PROBE, lt.GAMEDATA)
        server, port = lt.serve()
        env = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
                   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}",
                   KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}",
                   KOTONE_POPUP_CAPTURABLE="1")
        for name in ("KOTONE_TOOL_SERVICE_DIR", "KOTONE_EXE"):
            env.pop(name, None)
        fs.ENV = bt.ENV = env
        print(f"exe: {src}", flush=True)
        app = subprocess.Popen([fs.EXE, "--background"], env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=fs.NO_WINDOW)
        status = fs.wait_service(app)
        print(f"tool service up: {status.get('appVersion')} pid {status.get('pid')}", flush=True)
        time.sleep(4)
        ui, urls = fs.embedded_frontend()
        print(f"embedded frontend: {ui} ({urls[:160]})", flush=True)
        fs.guard_start()
        target = fs.Target()
        time.sleep(0.5)
        print(f"test window: {fs.describe(target.hwnd)}", flush=True)
        run(app, target, ui)
    finally:
        if target is not None:
            target.close()
            time.sleep(0.5)
            fs.give_back_to(started_from, app.pid if app is not None else -1)
        if app is not None:
            fs.stop_kotone(app)
        if server is not None:
            server.shutdown()
        lines = [line for line in log_text().splitlines() if "消息提示" in line or "auto card" in line]
        print("log:\n  " + "\n  ".join(lines[-12:]))
        if "--keep" not in sys.argv:
            for _ in range(10):
                shutil.rmtree(WORK, ignore_errors=True)
                if not os.path.exists(WORK):
                    break
                time.sleep(1)
        try:
            os.rmdir(fs.LOCK)
        except FileNotFoundError:
            print(f"(someone else removed {fs.LOCK} while this ran)")
    failed = [n for n, ok in RESULTS + fs.RESULTS if not ok]
    total = len(RESULTS) + len(fs.RESULTS)
    print(f"\n{total - len(failed)}/{total} passed" + (f"; FAILED: {failed}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
