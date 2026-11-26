"""The floating window (悬浮窗, ADR 043 D6, revised 2026-11-26 evening) on a real Kotone: her avatar in the corner with a
progress ring, and next to it the mini chat (the chat page's components) when a card is pushed — over a "game" (a test
window) without ever taking its foreground; clicking buttons in it, the wheel, dragging it and × all leave the
foreground (and the keyboard) with the game; the ring stops while the mouse is over it and, with the mouse away, runs
out and hides the window by itself; the next appearance uses the dragged position; it is excluded from capture; it
sits above the voice input bar in the bottom-right corner. Replaces message_popup_real.py (the message pop-up merged
into the floating window).

Setup (like message_popup_real.py): the fake LoL client timeline from game_events_real.py (0 s Lobby → 8 s ChampSelect
→ 16 s InProgress, loading → 60 s EndOfGame → 70 s Lobby). What it makes Kotone do here are the LoL plugin's card
pushes (文字识别模型 at once — no local OCR model in the isolated home —, 选英雄 in champ select, 开局战绩 at loading,
这局总结 after the end), each popping the mini chat; F4–F8 wait for the 开局战绩 card and a still window first. Isolated
KOTONE_HOME=target/floating-real/home, `--background`, no AI key (pushes never call the model), input tools denied,
the voice input bar at its bottom-right preset with visibility always (online recognition that cannot reach anything —
a DeepSeek connection without a key, pointed at the local discard port, as in focus_steal_real.py — so the runtime runs
and the bar shows). A test window (target_window.ps1) stands in for the game and is in front the whole time.

Checks:
F1 the input bar is shown at the bottom-right.
F2 the 开局战绩 push pops the floating window in the mini state; while it appears the foreground stays the test window,
   which never loses focus and gets no key; the window is topmost and WS_EX_NOACTIVATE.
F3 capture: the window's display affinity is WDA_EXCLUDEFROMCAPTURE (with --shots it is made capturable instead and the
   corner is screenshot to target/plugin-shots/round9/floating-*.png).
F4 it does not overlap the input bar (stacked above it); the page shows her avatar (with the ring running) and the
   mini chat with the card.
F4b (run after F5, while the mini chat is tall) the transparent strip next to the tall mini chat (above her avatar)
   lets clicks through (the window keeps only the
   avatar and the mini chat, SetWindowRgn): the window under a point there is not the floating window; with the test
   window moved behind the floating window, a click there reaches the test window (MOUSE Left in its log) and not the
   floating page; the avatar itself still belongs to the floating window. The test window goes back afterwards.
F5 one click on 往后翻 / 往前翻 (or, without them, on the card's title line): the page gets the click (pointerdown +
   click), the ring stops while the mouse is over it, the foreground stays the test window, no key.
F6 the wheel over the list: the page gets the wheel event and the list scrolls (when it can); foreground stays, no
   key. If the wheel does not arrive, this FAILs: the up / down buttons in the mini window are the fallback.
F7 dragging the mini window's title line by (-160, -120): the window follows, the foreground stays the test window the
   whole time (sampled every 50 ms), no key; `assistant.bubble.position` is saved.
F8 × closes it; the foreground stays the test window; no key.
F9 the 这局总结 push pops it again, at the dragged position (same anchored corner, within 4 px), not stacked in the corner.
F10 with the mouse away the ring runs out and the window hides by itself (10 s for the mini chat, plus slack).
System input: the mouse in F4b–F8 (only when the test window is in front and — F5–F8 — the floating window, or — F4b —
the test window, is the window under the point; the cursor is put back); F4b moves / resizes the test window (not
activating it) and puts it back. Nothing is typed, no Alt is pressed. Needs a debug build with the embedded frontend
(`pnpm -C apps/desktop build:apps`, `build:web`, then `cargo build -p kotone-tauri --features tauri/custom-protocol`);
the exe is copied to target/floating-real/bin first. Takes target/tmp/real.lock; only starts when a terminal is in front
and no game runs; at the end the terminal gets the foreground back.
usage: python -X utf8 -I scripts/manual-e2e/floating_real.py [--exe PATH] [--probe DIR] [--gamedata DIR] [--shots] [--keep]
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
import focus_steal_real as fs  # noqa: E402  (also makes this process per-monitor DPI aware: physical pixels)
import game_events_real as ge  # noqa: E402

lt, bt = ge.lt, ge.bt
user32 = fs.user32
REPO = fs.REPO
WORK = os.path.join(REPO, "target", "floating-real")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")
SHOTS = os.path.join(REPO, "target", "plugin-shots", "round9")
PORT = 9381
FLOATING_TITLE = "Kotone 悬浮窗"
SHOOT = "--shots" in sys.argv
WDA_EXCLUDEFROMCAPTURE = 0x11
WHEEL = 0x0800
RESULTS = []
user32.GetWindowDisplayAffinity.argtypes = [fs.wt.HWND, ctypes.POINTER(fs.wt.DWORD)]
user32.SetWindowPos.argtypes = [fs.wt.HWND, fs.wt.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                                ctypes.c_uint]
SWP_NOZORDER, SWP_NOACTIVATE = 0x0004, 0x0010


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
                   "runAsAdminOnStart": False,
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": lt.DENIED}, "matchRecap": False},
                   "sttEngine": "deepseek-asr", "activeModelId": "deepseek-asr",
                   "connections": [{"id": "floating-real", "displayName": "floating-real",
                                    "provider": "deepseek", "baseUrl": "http://127.0.0.1:9/",
                                    "model": "deepseek-chat"}],
                   "overlay": {"visibility": "always", "position": "bottom_right"}}, f)


def read_config():
    with open(os.path.join(HOME, "config.json"), encoding="utf-8") as f:
        return json.load(f)


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


def shown(app_pid, title):
    w = window(app_pid, title)
    return w if w and w[1] else None


def overlaps(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def shot(name, rects):
    if not SHOOT:
        return
    try:
        from PIL import ImageGrab
    except ImportError:
        print("   (no Pillow: no screenshot)")
        return
    left = min(r[0] for r in rects) - 16
    top = min(r[1] for r in rects) - 16
    right = max(r[2] for r in rects) + 16
    bottom = max(r[3] for r in rects) + 16
    os.makedirs(SHOTS, exist_ok=True)
    path = os.path.join(SHOTS, f"floating-real-{name}.png")
    ImageGrab.grab(bbox=(left, top, right, bottom), all_screens=True).save(path)
    print(f"   screenshot {path}")


def log_text():
    try:
        return open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return ""


# 悬浮窗页面里记下收到的鼠标事件；报小窗、列表、按钮在窗口里的位置（CSS 像素）、缩放、列表滚到哪
FLOATING_WATCH = """
const w = window;
if (!w.__floatingTest) {
  w.__floatingTest = { events: [] };
  for (const type of ["pointerenter", "pointerdown", "pointerup", "click", "wheel"]) window.addEventListener(type, () => w.__floatingTest.events.push(type), { capture: true, passive: true });
}
const box = (sel) => { const r = document.querySelector(sel)?.getBoundingClientRect(); return r ? [r.left, r.top, r.width, r.height] : null; };
const list = document.querySelector('[data-testid=floating-list]');
const ring = document.querySelector('[data-testid=floating-ring]');
const seen = { events: w.__floatingTest.events.slice(), mode: document.querySelector('[data-testid=floating]')?.dataset.mode,
  turns: [...document.querySelectorAll('[data-testid=floating-turn]')].map((e) => e.dataset.push),
  avatar: box('[data-testid=floating-avatar]'), ring: ring ? { running: ring.dataset.running, left: Number(ring.dataset.left) } : null,
  head: box('[data-testid=floating-mini-head]'), list: box('[data-testid=floating-list]'),
  older: box('[data-testid=floating-older]'), newer: box('[data-testid=floating-newer]'),
  close: box('[data-testid=floating-close]'), title: box('[data-testid=push-from]'),
  scrollTop: list ? list.scrollTop : null, scrollable: list ? list.scrollHeight > list.clientHeight + 2 : false,
  dpr: window.devicePixelRatio };
w.__floatingTest.events.length = 0;
return seen;
"""


def floating_page():
    return fs.page_eval("#/floating", FLOATING_WATCH).get("ok") or {}


def at(floating_hwnd, box, fx=0.5, fy=0.5, dpr=1.0):
    """Screen point (physical) inside a CSS box [left, top, width, height] of the floating page."""
    left, top, _, _ = fs.rect(floating_hwnd)
    x, y, w, h = box
    return int(left + (x + w * fx) * dpr), int(top + (y + h * fy) * dpr)


def guard_point(app_pid, target, x, y, what, floating_hwnd):
    fs.guard_input(app_pid, target.hwnd, strict=True)
    under = fs.root_at(x, y)
    if under != floating_hwnd:
        raise SystemExit(f"STOP: the window under {what} ({x},{y}) is {fs.describe(under)}, not the floating window; "
                         f"not sending input")


def click_at(app_pid, target, x, y, what, floating_hwnd):
    guard_point(app_pid, target, x, y, what, floating_hwnd)
    fs.mouse_abs(x, y, 0)
    time.sleep(0.4)
    guard_point(app_pid, target, x, y, what, floating_hwnd)
    fs.mouse_abs(x, y, fs.LEFTDOWN)
    time.sleep(0.08)
    fs.mouse_abs(x, y, fs.LEFTUP)
    return time.time()


def wheel_at(app_pid, target, x, y, delta, floating_hwnd):
    guard_point(app_pid, target, x, y, "the list", floating_hwnd)
    fs.mouse_abs(x, y, 0)
    time.sleep(0.3)
    guard_point(app_pid, target, x, y, "the list", floating_hwnd)
    vx, vy = user32.GetSystemMetrics(76), user32.GetSystemMetrics(77)
    vw, vh = user32.GetSystemMetrics(78), user32.GetSystemMetrics(79)
    inp = fs.INPUT(type=0)
    inp.mi = fs.MOUSEINPUT(dx=int((x - vx) * 65535 / max(vw - 1, 1)), dy=int((y - vy) * 65535 / max(vh - 1, 1)),
                           mouseData=ctypes.c_uint32(delta).value, dwFlags=WHEEL | 0x8000 | 0x4000, time=0,
                           dwExtraInfo=0)
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(fs.INPUT))


def drag_by(app_pid, target, x, y, dx, dy, floating_hwnd):
    guard_point(app_pid, target, x, y, "the title line", floating_hwnd)
    fs.mouse_abs(x, y, 0)
    time.sleep(0.4)
    guard_point(app_pid, target, x, y, "the title line", floating_hwnd)
    fs.mouse_abs(x, y, fs.LEFTDOWN)
    time.sleep(0.1)
    steps = 16
    for i in range(1, steps + 1):
        fs.mouse_abs(int(x + dx * i / steps), int(y + dy * i / steps), 0)
        time.sleep(0.03)
    time.sleep(0.2)
    fs.mouse_abs(x + dx, y + dy, fs.LEFTUP)
    return time.time()


def click_through(pid, target, floating, seen, dpr):
    """F4b: the transparent strip beside the tall mini chat (above / below her avatar) is not part of the window."""
    avatar = seen.get("avatar")
    left, top, right, bottom = fs.rect(floating)
    if not avatar:
        check("F4b: the page reports her avatar box", False, json.dumps(seen)[:200])
        return
    ax, ay, aw, ah = avatar
    height_css = (bottom - top) / dpr
    pad = 8  # 头像那一块四周带着阴影那 8（Floating.svelte 的 HIT_PAD）
    if ay > height_css / 2:  # 头像在下边的角：上面那一条
        y_css = (ay - pad) / 2
        room = ay - pad
    else:  # 头像在上边的角：下面那一条
        y_css = (ay + ah + pad + height_css) / 2
        room = height_css - (ay + ah + pad)
    if room < 24:
        print(f"   SKIP F4b: the mini chat is not taller than the avatar (room {room:.0f} css px)")
        return
    x, y = int(left + (ax + aw / 2) * dpr), int(top + y_css * dpr)
    under = fs.root_at(x, y)
    check("F4b: the transparent strip beside the mini chat is not the floating window (clicks go through)",
          under != floating, f"({x},{y}) is {fs.describe(under)}")
    cx, cy = at(floating, avatar, dpr=dpr)
    check("F4b: her avatar still belongs to the floating window", fs.root_at(cx, cy) == floating,
          f"({cx},{cy}) is {fs.describe(fs.root_at(cx, cy))}")
    # 测试窗口挪到悬浮窗后面（不激活、不改叠放次序），在那一条上点一下：点到的是测试窗口
    before = fs.rect(target.hwnd)
    moved = user32.SetWindowPos(target.hwnd, None, left - 40, top - 40, (right - left) + 80, (bottom - top) + 80,
                                SWP_NOZORDER | SWP_NOACTIVATE)
    try:
        time.sleep(0.6)
        if not moved or fs.root_at(x, y) != target.hwnd:
            check("F4b: the test window is behind the strip", False, f"({x},{y}) is {fs.describe(fs.root_at(x, y))}")
            return
        floating_page()  # 清掉页面里记下的事件
        target.set_mark()
        fs.guard_input(pid, target.hwnd, strict=True)
        fs.mouse_abs(x, y, 0)
        time.sleep(0.3)
        fs.guard_input(pid, target.hwnd, strict=True)
        if fs.root_at(x, y) != target.hwnd:
            raise SystemExit(f"STOP: the window under ({x},{y}) is {fs.describe(fs.root_at(x, y))}; not clicking")
        fs.mouse_abs(x, y, fs.LEFTDOWN)
        time.sleep(0.08)
        fs.mouse_abs(x, y, fs.LEFTUP)
        time.sleep(0.6)
        got = [e for e in target.since_mark() if " MOUSE " in f" {e} "]
        page = floating_page()
        check("F4b: a click on the strip reaches the test window behind, not the floating page",
              got and "pointerdown" not in page.get("events", []),
              f"test window: {got}; floating page: {page.get('events')}")
        check("F4b: the foreground stayed the test window",
              fs.hval(user32.GetForegroundWindow()) == target.hwnd, fs.describe(user32.GetForegroundWindow()))
    finally:
        user32.SetWindowPos(target.hwnd, None, before[0], before[1], before[2] - before[0], before[3] - before[1],
                            SWP_NOZORDER | SWP_NOACTIVATE)
        time.sleep(0.4)


def keys_since_mark(target):
    return [e for e in target.since_mark() if " DOWN " in f" {e} " or " UP " in f" {e} "]


def no_change_checks(name, sampler, target):
    changes = sampler.finish()
    taken = [(t, d) for t, h, d in changes if h != target.hwnd]
    check(f"{name}: the foreground stayed the test window", not taken, "; ".join(f"{t:.2f}s {d}" for t, d in taken))
    keys = keys_since_mark(target)
    check(f"{name}: the test window got no key from Kotone", not keys, "; ".join(keys))


def run(app, target, ui):
    pid = app.pid
    bar = wait_until(lambda: shown(pid, fs.OVERLAY_TITLE), 30)
    if not check("F1: the voice input bar is shown", bar, str(bar[2]) if bar else "no visible input bar"):
        return
    menu = wait_until(lambda: bt_phase() == "menu", 40, 1)
    check("setup: the fake client is connected (lol in menu)", menu, str(bt_phase()))
    fs.give_back_focus(pid, target)
    if not check("setup: the test window is in front", fs.hval(user32.GetForegroundWindow()) == target.hwnd,
                 fs.describe(user32.GetForegroundWindow())):
        return

    # F2：开局战绩推来，弹出小窗；前台一直是测试窗口
    target.set_mark()
    sampler = fs.Sampler()
    lt.REC.t0 = time.time()
    first = wait_until(lambda: shown(pid, FLOATING_TITLE), 60)
    if first and ui:
        wait_until(lambda: floating_page().get("mode") == "mini", 5)
    time.sleep(1.0)
    changes = sampler.finish()
    skipped = next((line for line in log_text().splitlines() if "悬浮窗：没弹" in line), None)
    if not check("F2: the 开局战绩 push pops the floating window", first, str(first[2]) if first else skipped or log_text()[-600:]):
        return
    floating = first[0]
    ex = user32.GetWindowLongPtrW(floating, -20)
    check("F2: topmost and WS_EX_NOACTIVATE", ex & 0x8 and ex & fs.WS_EX_NOACTIVATE, hex(ex))
    fs.foreground_checks("F2", changes, target, pid, target.since_mark)

    # F3：不进截图
    affinity = fs.wt.DWORD()
    user32.GetWindowDisplayAffinity(floating, ctypes.byref(affinity))
    if SHOOT:
        print(f"   --shots: capturable (affinity {affinity.value:#x})")
    else:
        check("F3: excluded from capture (WDA_EXCLUDEFROMCAPTURE)", affinity.value == WDA_EXCLUDEFROMCAPTURE,
              hex(affinity.value))

    # 等开局战绩推来、窗口不再变大（前面还有文字识别模型、选英雄两张卡片；同一张小窗里接着往下加）
    if ui:
        opened = wait_until(lambda: any(":opening:" in (k or "") for k in floating_page().get("turns", [])), 40, 0.5)
        print(f"   the 开局战绩 card is in the mini chat: {bool(opened)}")
    still, last = 0, None
    for _ in range(40):
        now = fs.rect(floating)
        still = still + 1 if now == last else 0
        last = now
        if still >= 6:
            break
        time.sleep(0.25)

    # F4：和输入条不叠，页面里是那张卡片
    bar = window(pid, fs.OVERLAY_TITLE)
    rect = fs.rect(floating)
    check("F4: the floating window and the input bar do not overlap", not overlaps(rect, bar[2]),
          f"floating {rect}, input bar {bar[2]}")
    shot("mini", [rect, bar[2]])
    seen = floating_page() if ui else {}
    print(f"   floating page: {json.dumps(seen, ensure_ascii=False)}")
    if ui:
        check("F4: her avatar is in the corner with the ring running", seen.get("avatar") and (seen.get("ring") or {}).get("running") == "true",
              json.dumps({"avatar": seen.get("avatar"), "ring": seen.get("ring")}))
        check("F4: the mini chat shows the 开局战绩 card", seen.get("mode") == "mini" and any(":opening:" in (k or "") for k in seen.get("turns", [])),
              json.dumps({"mode": seen.get("mode"), "turns": seen.get("turns")}))
    if not ui:
        print("SKIP F5–F9 — no embedded frontend (positions inside the page are unknown)")
        return
    dpr = seen.get("dpr") or 1.0
    cursor = fs.wt.POINT()
    user32.GetCursorPos(ctypes.byref(cursor))
    try:
        # F5：点小窗里的按钮
        button = seen.get("newer") or seen.get("older") or seen.get("title")
        target.set_mark()
        sampler = fs.Sampler()
        x, y = at(floating, button, dpr=dpr)
        click_at(pid, target, x, y, "a button in the mini window", floating)
        time.sleep(0.6)
        after = floating_page()
        check("F5: the page got the click (pointerdown + click)",
              "pointerdown" in after.get("events", []) and "click" in after.get("events", []), json.dumps(after.get("events")))
        ring = after.get("ring") or {}
        time.sleep(0.5)
        later = floating_page().get("ring") or {}
        check("F5: the ring stops while the mouse is over it", ring.get("running") == "false" and later.get("left") == ring.get("left"),
              f"{ring} → {later}")
        no_change_checks("F5", sampler, target)

        # F4b（F5 以后：鼠标在小窗上、进度条停着；移开以后至少还剩 2 秒，F6 又停在上面）：旁边透明的那一条点得到下面的窗口
        click_through(pid, target, floating, seen, dpr)

        # F6：滚轮
        if after.get("list"):
            before = after.get("scrollTop")
            target.set_mark()
            sampler = fs.Sampler()
            x, y = at(floating, after["list"], dpr=dpr)
            wheel_at(pid, target, x, y, 120 if (before or 0) > 0 else -120, floating)
            time.sleep(0.8)
            wheeled = floating_page()
            # 列表能滚时看它滚了没有（2026-11-27 真机：滚了一格 100 像素，页面上却没记到 wheel 事件——滚动是 Chromium
            # 自己做的）；不能滚时只能看页面收没收到 wheel
            seen_wheel = "wheel" in wheeled.get("events", [])
            scrolled = wheeled.get("scrollTop") != before
            check("F6: the wheel reaches the non-focusable window and scrolls the list (else: use the up / down buttons)",
                  scrolled if after.get("scrollable") else seen_wheel,
                  f"wheel event seen {seen_wheel}, scrollTop {before} → {wheeled.get('scrollTop')}, "
                  f"scrollable {after.get('scrollable')}")
            no_change_checks("F6", sampler, target)

        # F7：拖上面那一行
        head = floating_page().get("head")
        start = fs.rect(floating)
        target.set_mark()
        sampler = fs.Sampler()
        x, y = at(floating, head, fx=0.3, dpr=dpr)
        drag_by(pid, target, x, y, -160, -120, floating)
        time.sleep(0.8)
        moved = fs.rect(floating)
        check("F7: the window followed the drag (-160, -120)",
              abs((moved[0] - start[0]) + 160) <= 6 and abs((moved[1] - start[1]) + 120) <= 6, f"{start} → {moved}")
        no_change_checks("F7", sampler, target)
        position = wait_until(lambda: (read_config().get("assistant", {}).get("bubble", {}) or {}).get("position"), 5)
        check("F7: assistant.bubble.position is saved", position, json.dumps(position))
        dragged = moved

        # F8：×
        close = floating_page().get("close")
        target.set_mark()
        sampler = fs.Sampler()
        x, y = at(floating, close, dpr=dpr)
        click_at(pid, target, x, y, "×", floating)
        gone = wait_until(lambda: not (window(pid, FLOATING_TITLE) or (0, False))[1], 2, 0.05)
        time.sleep(0.8)
        check("F8: × closes the floating window", gone, str(window(pid, FLOATING_TITLE)))
        no_change_checks("F8", sampler, target)
    finally:
        user32.SetCursorPos(cursor.x, cursor.y)

    # F9：这局总结推来：在拖到的地方出现
    second = wait_until(lambda: shown(pid, FLOATING_TITLE), 90)
    if not check("F9: the 这局总结 push pops it again", second, log_text()[-600:]):
        return
    time.sleep(1.0)
    again = fs.rect(floating)
    corner = (position or {}).get("corner", "")
    xs = (again[2], dragged[2]) if corner.endswith("right") else (again[0], dragged[0])
    ys = (again[3], dragged[3]) if corner.startswith("bottom") else (again[1], dragged[1])
    check(f"F9: it appears at the dragged position ({corner} corner within 4 px)",
          abs(xs[0] - xs[1]) <= 4 and abs(ys[0] - ys[1]) <= 4, f"dragged {dragged}, now {again}")
    shot("dragged", [again])

    # F10：鼠标不在上面：进度条走完，自己收起（聊天小窗 10 秒，从最后一次变化算）
    cursor = fs.wt.POINT()
    user32.GetCursorPos(ctypes.byref(cursor))
    over = fs.root_at(cursor.x, cursor.y) == floating
    if over:
        print("   (the cursor is over the floating window: the ring waits; skipping F10)")
        return
    started = time.time()
    gone = wait_until(lambda: not (window(pid, FLOATING_TITLE) or (0, False))[1], 20, 0.1)
    check("F10: with the mouse away the ring runs out and the window hides by itself",
          gone, f"after {time.time() - started:.1f}s: {window(pid, FLOATING_TITLE)}")
    if gone:
        print(f"   hid {time.time() - started:.1f}s after it was seen again")


def bt_phase():
    try:
        code, value = fs.tool("game.state", {"game": "lol"})
        playing = (value.get("result") or {}).get("playing") or [] if code == 0 and value.get("ok") else []
    except Exception:  # noqa: BLE001
        return None
    return playing[0]["phase"] if playing else None


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
                   KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}")
        if SHOOT:
            env["KOTONE_FLOATING_CAPTURABLE"] = "1"
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
        lines = [line for line in log_text().splitlines() if "悬浮窗" in line or "push:" in line]
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
