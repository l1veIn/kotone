"""Kotone must not take the game's foreground (shared-resources audit section 2 #5, 2026-11-25).

The audit (code reading only) named three places that could take the focus away from the game mid-match:

1. a plugin's web window (`Host::open_web_window`, the 网易云音乐 login window). Who opens it: the 网易云音乐 page
   (the player clicked 登录 / 放) and anyone who calls the page tools `music.login_start` / `music.play` through the
   tool service or `kotone.exe mcp` (an external Agent: `Caller::Service` may call page tools). Old code: a new window
   is created with focus; an open one gets `set_focus` (tao `force_window_active`, which taps Alt into whatever is in
   front when Windows refuses).
2. an MCP App's `ui/open-link` opened the system browser without asking. Not here (this script never opens the
   player's browser): Playwright `apps/desktop/e2e/mcp-app-links.spec.ts`.
3. clicking the voice input overlay (with 允许交互并拖动 on) activated it: its window had no `focusable: false`.

Against a real Kotone in an isolated KOTONE_HOME (target/focus-steal-real/home), started with `--background`:

- a test window (target_window.ps1, stands in for the game) is in front; it logs focus changes and every key it gets.
- A1 an external Agent opens the login window (`kotone.exe tools call music.login_start`): the foreground stays the
  test window the whole time (sampled every 50 ms), the test window got no key (no Alt) and never lost focus, and the
  login window is there for the player to find (visible, behind the test window).
- A2 the same call again (the window is already open; the old code calls `set_focus`): the same checks.
- A3 the player's own click: Kotone's window in front (opened the way clicking a message pop-up opens it), then
  `music.login_start` from the page (what the 登录 button calls): the login window (still open behind) comes to the
  front. Then the login window is closed (`music.login_cancel`) and the script prints whether Kotone's UI thread
  still answers: 2026-11-25 it sometimes stopped answering after that, with the old build too — not what this
  script checks, so A and B each get a freshly started Kotone.
- B1 the overlay (visibility always, 允许交互并拖动 on, online recognition so it is running: DeepSeek recognition only
  needs a DeepSeek connection to count as ready; the one in the isolated config has no key and points at the local
  discard port, so nothing is sent anywhere) is shown.
- B2 one left click on the overlay: the overlay page gets it (pointerdown + click: its buttons work), and the
  foreground is back on the test window within 1 s after the release (Windows activates a clicked window; Kotone
  gives the foreground back; the old code left it on the overlay, so the game got no keys until clicked again).
- B3 dragging the overlay by 80 px: the page starts the drag, the overlay moves, the position is saved
  (`overlay.position` = custom), the foreground is back on the test window within 1 s after the release.
  (2026-11-25 tried and dropped: making the overlay `focusable: false` / WS_EX_NOACTIVATE — the foreground stayed,
  but WebView2 got no mouse events at all: buttons dead, no drag.)

System input: the mouse click / drag in B2 / B3, and Alt + SetForegroundWindow to give the test window its foreground
back after a step took it (like fg.ps1). Before every one of them the foreground is checked: it must be the test
window or a window of this isolated Kotone, otherwise the script stops. It only starts when a terminal is in front
and no game is running; at the end the terminal gets the foreground back. The cursor is put back where it was.
Nothing is typed.
Needs: a debug build with the embedded frontend for A3 / B3 (`pnpm -C apps/desktop build:apps`, `build:web`, then
`cargo build -p kotone-tauri --features tauri/custom-protocol`; without it those steps report SKIP); the exe is copied
to target/focus-steal-real/bin first. Opens one request to music.163.com (the login page). Takes target/tmp/real.lock.
usage: python -X utf8 -I scripts/manual-e2e/focus_steal_real.py [--exe PATH] [--only a|b] [--keep]
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
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "focus-steal-real")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")
TARGET = os.path.join(WORK, "target")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
SRC_EXE = (os.path.abspath(sys.argv[sys.argv.index("--exe") + 1]) if "--exe" in sys.argv
           else os.path.join(REPO, "target", "debug", "kotone.exe"))
ONLY = sys.argv[sys.argv.index("--only") + 1] if "--only" in sys.argv else ""
EXE = os.path.join(BIN, "kotone.exe")
DEVTOOLS = 9373
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={DEVTOOLS}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)
NO_WINDOW = 0x08000000
GAMES = ("league of legends", "leagueclient", "riotclient", "valorant", "genshinimpact", "yuanshen", "starrail",
         "zenlesszonezero", "dota2", "cs2", "wow", "overwatch", "pubg", "naraka", "eldenring")
TERMINALS = ("windowsterminal.exe", "openconsole.exe", "conhost.exe", "cmd.exe", "powershell.exe", "pwsh.exe",
             "mintty.exe", "wezterm-gui.exe", "alacritty.exe", "warp.exe", "code.exe")
LOGIN_TITLE = "登录网易云音乐"
OVERLAY_TITLE = "Kotone Overlay"
WS_EX_NOACTIVATE = 0x08000000
RESULTS = []

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
try:  # physical pixels for window rects and the cursor
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception:  # noqa: BLE001
    pass
user32.GetForegroundWindow.restype = wt.HWND
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.GetWindowThreadProcessId.restype = wt.DWORD
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.IsIconic.argtypes = [wt.HWND]
user32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(wt.RECT)]
user32.SetForegroundWindow.argtypes = [wt.HWND]
user32.IsWindow.argtypes = [wt.HWND]
user32.WindowFromPoint.argtypes = [wt.POINT]
user32.WindowFromPoint.restype = wt.HWND
user32.GetAncestor.argtypes = [wt.HWND, ctypes.c_uint]
user32.GetAncestor.restype = wt.HWND
user32.SendMessageTimeoutW.argtypes = [wt.HWND, ctypes.c_uint, wt.WPARAM, wt.LPARAM, ctypes.c_uint, ctypes.c_uint,
                                       ctypes.POINTER(ctypes.c_size_t)]
user32.GetWindowLongPtrW.argtypes = [wt.HWND, ctypes.c_int]
user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t
ENUM_PROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [ENUM_PROC, wt.LPARAM]


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wt.LONG), ("dy", wt.LONG), ("mouseData", wt.DWORD), ("dwFlags", wt.DWORD),
                ("time", wt.DWORD), ("dwExtraInfo", ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [("mi", MOUSEINPUT), ("pad", ctypes.c_byte * 32)]
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _U)]


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def skip(name, why):
    print(f"SKIP {name} — {why}", flush=True)


def hval(hwnd):
    return int(hwnd or 0)


def window_pid(hwnd):
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def process_path(pid):
    handle = kernel32.OpenProcess(0x1000, False, pid)
    if not handle:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
        return buf.value
    finally:
        kernel32.CloseHandle(handle)


def title_of(hwnd):
    buf = ctypes.create_unicode_buffer(512)
    user32.GetWindowTextW(hwnd, buf, 512)
    return buf.value


def describe(hwnd):
    if not hval(hwnd):
        return "none"
    pid = window_pid(hwnd)
    return f"{os.path.basename(process_path(pid)) or '?'} pid {pid} {hval(hwnd):#x} {title_of(hwnd)!r}"


def top_windows():
    """Top-level windows, top of the z-order first."""
    found = []
    user32.EnumWindows(ENUM_PROC(lambda h, _: found.append(hval(h)) or True), 0)
    return found


def find_window(pid, title):
    for h in top_windows():
        if window_pid(h) == pid and title_of(h) == title:
            return h
    return 0


def rect(hwnd):
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return r.left, r.top, r.right, r.bottom


def guard_start():
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, encoding="mbcs",
                         errors="replace").stdout.lower()
    running = [g for g in ("league of legends.exe", "leagueclient", "riotclient", "valorant", "cs2.exe",
                           "dota2.exe", "yuanshen", "genshinimpact") if g in out]
    if running:
        raise SystemExit(f"STOP: a game is running ({running}); not testing while the player plays")
    front = hval(user32.GetForegroundWindow())
    exe = os.path.basename(process_path(window_pid(front))).lower() if front else ""
    if exe not in TERMINALS:
        raise SystemExit(f"STOP: the test window takes the foreground, so this only starts when a terminal is in "
                         f"front (now: {describe(front)})")


def guard_input(app_pid, target_hwnd, strict=False):
    """Before any system input: the foreground must be the test window, this isolated Kotone, a terminal or none
    (`strict`: the test window only — the mouse steps measure what happens to it)."""
    front = hval(user32.GetForegroundWindow())
    exe = os.path.basename(process_path(window_pid(front))).lower() if front else ""
    # 没有前台窗口（Kotone 的窗口刚关掉、或者它卡住了）也行：这时按的 Alt 没有窗口收
    if front == target_hwnd or (not strict and (not front or window_pid(front) == app_pid or exe in TERMINALS)):
        return front
    raise SystemExit(f"STOP: before sending input the foreground is {describe(front)}, not the test window, the "
                     f"isolated Kotone or a terminal; not sending anything")


def cli(*args, timeout=30, stdin_text=None):
    out = subprocess.run([EXE, *args], capture_output=True, text=True, encoding="utf-8", errors="replace", env=ENV,
                         input=stdin_text, stdin=None if stdin_text is not None else subprocess.DEVNULL,
                         timeout=timeout, creationflags=NO_WINDOW)
    try:
        return out.returncode, json.loads(out.stdout)
    except json.JSONDecodeError:
        return out.returncode, {"raw": (out.stdout + out.stderr)[-500:]}


def tool(name, args=None, timeout_ms=15000):
    """`kotone.exe tools call`: what an external Agent does (Caller::Service)."""
    return cli("tools", "call", name, "--args-file", "-", "--timeout-ms", str(timeout_ms),
               timeout=timeout_ms / 1000 + 15, stdin_text=json.dumps(args or {}, ensure_ascii=False))


def wait_service(app, timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if app.poll() is not None:
            raise RuntimeError(f"kotone.exe exited early (code {app.returncode})")
        code, value = tool("app.status", timeout_ms=3000)
        if code == 0 and value.get("ok"):
            return value["result"]
        time.sleep(0.5)
    raise RuntimeError("the isolated instance's tool service did not come up")


class Sampler:
    """Every change of the foreground window, sampled every 50 ms."""

    def __init__(self):
        self.changes = []
        self.stop = threading.Event()
        self.t0 = time.time()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        last = None
        while not self.stop.is_set():
            hwnd = hval(user32.GetForegroundWindow())
            if hwnd != last:
                self.changes.append((time.time() - self.t0, hwnd, describe(hwnd)))
                last = hwnd
            time.sleep(0.05)

    def finish(self):
        self.stop.set()
        self.thread.join(2)
        return self.changes


class Target:
    """The test window (target_window.ps1): stands in for the game."""

    def __init__(self):
        shutil.rmtree(TARGET, ignore_errors=True)
        os.makedirs(TARGET)
        self.proc = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                                      os.path.join(HERE, "target_window.ps1"), "-Dir", TARGET],
                                     creationflags=NO_WINDOW)
        ready = os.path.join(TARGET, "ready.txt")
        deadline = time.time() + 20
        while not os.path.exists(ready) and time.time() < deadline:
            time.sleep(0.1)
        time.sleep(0.3)
        with open(ready, encoding="utf-8-sig") as f:
            self.hwnd = int(f.read().strip())
        self.mark = 0

    def events(self):
        try:
            with open(os.path.join(TARGET, "events.log"), encoding="utf-8") as f:
                return f.read().splitlines()
        except OSError:
            return []

    def since_mark(self):
        return self.events()[self.mark:]

    def set_mark(self):
        self.mark = len(self.events())

    def close(self):
        try:
            with open(os.path.join(TARGET, "control.txt"), "w", encoding="utf-8") as f:
                f.write("close")
            self.proc.wait(5)
        except Exception:  # noqa: BLE001
            self.proc.kill()


def tap_alt_and_focus(hwnd):
    """fg.ps1: tap Alt (lifts the foreground lock), then SetForegroundWindow."""
    user32.keybd_event(0x12, 0, 0, 0)
    user32.keybd_event(0x12, 0, 2, 0)
    user32.SetForegroundWindow(hwnd)
    time.sleep(0.3)


def give_back_focus(app_pid, target):
    """The test window gets the foreground back (after a step that took it)."""
    if hval(user32.GetForegroundWindow()) == target.hwnd:
        return True
    guard_input(app_pid, target.hwnd)
    tap_alt_and_focus(target.hwnd)
    time.sleep(0.5)
    target.set_mark()
    return hval(user32.GetForegroundWindow()) == target.hwnd


def foreground_checks(name, changes, target, app_pid, before_events):
    taken = [(t, d) for t, h, d in changes if h != target.hwnd]
    for t, _, d in changes:
        print(f"    {t:6.2f}s foreground: {d}")
    check(f"{name}: the foreground stayed the test window", not taken,
          "; ".join(f"{t:.2f}s {d}" for t, d in taken))
    events = before_events()
    lost = [e for e in events if "FOCUS lost" in e]
    keys = [e for e in events if " DOWN " in f" {e} " or " UP " in f" {e} "]
    check(f"{name}: the test window never lost focus", not lost, "; ".join(lost))
    check(f"{name}: the test window got no key from Kotone (no Alt tap)", not keys, "; ".join(keys))


def root_at(x, y):
    """The top-level window under the screen point (what a click there would hit)."""
    child = user32.WindowFromPoint(wt.POINT(x, y))
    return hval(user32.GetAncestor(child, 2)) if child else 0  # GA_ROOT


def responsive(hwnd, timeout_ms=1000):
    """Does the window's thread answer a message within timeout_ms (it is not hung)."""
    result = ctypes.c_size_t()
    return bool(user32.SendMessageTimeoutW(hwnd, 0, 0, 0, 0x0002, timeout_ms, ctypes.byref(result)))  # WM_NULL


def kotone_answers(app_pid, label):
    """Kotone's UI thread answers a message (it owns every Kotone window); printed after each step."""
    hwnd = find_window(app_pid, "Kotone 设置") or find_window(app_pid, OVERLAY_TITLE)
    ok = responsive(hwnd) if hwnd else None
    print(f"    [{label}] Kotone's UI thread answers: {ok}", flush=True)
    return ok


def mouse_abs(x, y, flags):
    vx, vy = user32.GetSystemMetrics(76), user32.GetSystemMetrics(77)
    vw, vh = user32.GetSystemMetrics(78), user32.GetSystemMetrics(79)
    inp = INPUT(type=0)
    inp.mi = MOUSEINPUT(dx=int((x - vx) * 65535 / max(vw - 1, 1)), dy=int((y - vy) * 65535 / max(vh - 1, 1)),
                        mouseData=0, dwFlags=flags | 0x0001 | 0x8000 | 0x4000, time=0, dwExtraInfo=0)
    user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


LEFTDOWN, LEFTUP = 0x0002, 0x0004

PAGE_EVAL = r"""
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:__PORT__");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes(__PAGE__));
try {
  const r = page ? await page.evaluate(async () => {
    try { return { ok: await (async () => { __BODY__ })() }; } catch (e) { return { error: String(e) }; }
  }) : { error: "page not found: " + browser.contexts().flatMap((c) => c.pages()).map((p) => p.url()).join(" ") };
  console.log(JSON.stringify(r));
} finally { await browser.close(); }
"""


def page_eval(page, body):
    """Run `body` in one of Kotone's pages (`#/settings`, `#/overlay`) over the WebView2 debugging port."""
    code = (PAGE_EVAL.replace("__PORT__", str(DEVTOOLS)).replace("__PAGE__", json.dumps(page))
            .replace("__BODY__", body))
    try:
        out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                             encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"), creationflags=NO_WINDOW,
                             timeout=30)
    except subprocess.TimeoutExpired:
        return {"error": "no answer in 30 s (Kotone's UI thread hung?)"}
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        return {"error": (out.stdout + out.stderr)[-800:]}


def invoke(command, args):
    """A Kotone command from the settings page, over the WebView2 debugging port (no OS input)."""
    return page_eval("#/settings", "return await window.__TAURI_INTERNALS__.invoke(%s, %s);"
                     % (json.dumps(command), json.dumps(args)))


# 悬浮窗页面里记下收到的鼠标事件（watchingFor：装上多久了，页面重新加载过就会变小）
OVERLAY_WATCH = """
const w = window;
if (!w.__focusTest) {
  w.__focusTest = { events: [], since: Date.now() };
  for (const type of ["pointerenter", "pointerdown", "pointerup", "click"]) window.addEventListener(type, () => w.__focusTest.events.push(type), true);
}
const seen = { events: w.__focusTest.events.slice(), watchingFor: Date.now() - w.__focusTest.since };
w.__focusTest.events.length = 0;
return seen;
"""


def embedded_frontend():
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{DEVTOOLS}/json", timeout=5) as r:
            pages = json.load(r)
    except Exception:  # noqa: BLE001
        return False, "no DevTools endpoint"
    urls = [p.get("url", "") for p in pages]
    return any(u.startswith("http://tauri.localhost") for u in urls), " ".join(urls)


def write_config():
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   # 在线识别：打开就在跑（不用下载模型），悬浮窗“常驻”时就显示出来。DeepSeek 识别有一条 DeepSeek
                   # 连接就算准备好了；这条没有 Key（凭据库里什么都不存）、地址是本机的 discard 端口，什么都发不出去
                   "sttEngine": "deepseek-asr", "activeModelId": "deepseek-asr",
                   "connections": [{"id": "focus-steal-real", "displayName": "focus-steal-real", "provider": "deepseek",
                                    "baseUrl": "http://127.0.0.1:9/", "model": "deepseek-chat"}],
                   "overlay": {"visibility": "always", "draggable": True, "clickThrough": False}}, f)


def read_config():
    with open(os.path.join(HOME, "config.json"), encoding="utf-8") as f:
        return json.load(f)


def part_web_window(app, target, ui):
    print("\n== A. a plugin's web window (网易云音乐 login) ==", flush=True)
    give_back_focus(app.pid, target)
    if not check("A0: the test window is in front", hval(user32.GetForegroundWindow()) == target.hwnd,
                 describe(user32.GetForegroundWindow())):
        return
    for step, label in (("A1", "an external Agent opens the login window"),
                        ("A2", "an external Agent asks again (window already open)")):
        print(f"  {step}: {label}", flush=True)
        target.set_mark()
        sampler = Sampler()
        code, value = tool("music.login_start")
        time.sleep(6)
        changes = sampler.finish()
        print(f"    music.login_start → {code} {json.dumps(value, ensure_ascii=False)[:200]}")
        check(f"{step}: music.login_start answered", code == 0 and value.get("ok"),
              json.dumps(value, ensure_ascii=False)[:300])
        foreground_checks(step, changes, target, app.pid, target.since_mark)
        login = find_window(app.pid, LOGIN_TITLE)
        if check(f"{step}: the login window exists", login, ""):
            order = top_windows()
            visible = user32.IsWindowVisible(login) and not user32.IsIconic(login)
            check(f"{step}: the login window is shown (the player finds it when they look)", visible)
            behind = login in order and target.hwnd in order and order.index(login) > order.index(target.hwnd)
            check(f"{step}: the login window is behind the test window", behind,
                  f"z-order index login {order.index(login) if login in order else '?'}, "
                  f"test window {order.index(target.hwnd) if target.hwnd in order else '?'}")
        kotone_answers(app.pid, f"after {step}")
        if not give_back_focus(app.pid, target):
            raise SystemExit("STOP: could not give the test window its foreground back")
    # A3：玩家自己点的（Kotone 在前面时点“登录”）照样拿到前面。登录窗口还开着（在后面）：不先关它，
    # 关网页登录窗口以后 Kotone 的界面线程有时会卡住（见最后一步，和这里测的无关）
    print("  A3: the player clicks 登录 in Kotone (the login window is still open, behind)", flush=True)
    if not ui:
        skip("A3", "no embedded frontend (build with --features tauri/custom-protocol)")
    else:
        result = invoke("auto_card_open", {"conversationId": "focus-steal-real", "key": "none"})
        deadline = time.time() + 15
        while time.time() < deadline and window_pid(user32.GetForegroundWindow()) != app.pid:
            time.sleep(0.1)
        check("A3: Kotone's window is in front (opened like clicking a message pop-up)",
              window_pid(user32.GetForegroundWindow()) == app.pid,
              f"{describe(user32.GetForegroundWindow())}; auto_card_open → {json.dumps(result, ensure_ascii=False)[:200]}")
        result = invoke("plugin_call", {"plugin": "music", "tool": "music.login_start", "arguments": {}})
        deadline = time.time() + 15
        login = 0
        while time.time() < deadline:
            login = find_window(app.pid, LOGIN_TITLE)
            if login and hval(user32.GetForegroundWindow()) == login:
                break
            time.sleep(0.1)
        check("A3: the login window comes to the front", login and hval(user32.GetForegroundWindow()) == login,
              f"foreground {describe(user32.GetForegroundWindow())}; plugin_call → "
              f"{json.dumps(result, ensure_ascii=False)[:200]}")
        kotone_answers(app.pid, "after A3")
    tool("music.login_cancel")
    time.sleep(1.5)
    if kotone_answers(app.pid, "after closing the login window") is False:
        print("    NOTE: Kotone's UI thread stopped answering after the login window was closed "
              "(music.login_cancel → close_web_window); part B starts a fresh Kotone", flush=True)
    give_back_focus(app.pid, target)


def returned_checks(name, changes, released_at, target):
    """After a click / drag on the overlay: the foreground may go to the overlay while the button is down (Windows
    activates the window that is clicked), but it is back on the test window within 1 s after the release."""
    for t, _, d in changes:
        print(f"    {t:6.2f}s foreground: {d}")
    away = [(t, h) for t, h, _ in changes if h != target.hwnd]
    back_at = next((t for t, h, _ in changes if h == target.hwnd and t >= released_at), None)
    last = changes[-1][1] if changes else 0
    if away:
        gone = away[0][0]
        print(f"    foreground away from the test window for {((back_at or gone) - gone) * 1000:.0f} ms")
    check(f"{name}: the foreground is back on the test window within 1 s after the release",
          last == target.hwnd and (not away or (back_at is not None and back_at - released_at <= 1.0)),
          "; ".join(f"{t:.2f}s {d}" for t, _, d in changes))
    keys = [e for e in target.since_mark() if " DOWN " in f" {e} " or " UP " in f" {e} "]
    check(f"{name}: the test window got no key from Kotone", not keys, "; ".join(keys))


def part_overlay(app, target, ui):
    print("\n== B. the voice input overlay (允许交互并拖动 on) ==", flush=True)
    deadline = time.time() + 20
    overlay = 0
    while time.time() < deadline:
        overlay = find_window(app.pid, OVERLAY_TITLE)
        if overlay and user32.IsWindowVisible(overlay):
            break
        time.sleep(0.2)
    code, status = tool("runtime.status")
    print(f"    runtime.status → {json.dumps(status.get('result', status), ensure_ascii=False)[:200]}")
    if not check("B1: the overlay is shown", overlay and user32.IsWindowVisible(overlay),
                 describe(overlay) if overlay else "no overlay window"):
        return
    ex = user32.GetWindowLongPtrW(overlay, -20)
    print(f"    overlay exstyle {ex:#x} (WS_EX_NOACTIVATE: {bool(ex & WS_EX_NOACTIVATE)})")
    answers = responsive(overlay)
    if not check("B1: Kotone's UI thread answers before anything is clicked", answers,
                 "" if answers else "it is hung already: whatever hung it happened before this part"):
        return
    left, top, right, bottom = rect(overlay)
    cx, cy = (left + right) // 2, (top + bottom) // 2
    print(f"    overlay {describe(overlay)} at {left},{top} – {right},{bottom}")
    if ui:
        page_eval("#/overlay", OVERLAY_WATCH)
        seen = page_eval("#/overlay", "document.documentElement.click();" + OVERLAY_WATCH).get("ok") or {}
        if not check("B1: the overlay page records its events (a click from the page itself)",
                     "click" in seen.get("events", []), json.dumps(seen)):
            ui = False
    cursor = wt.POINT()
    user32.GetCursorPos(ctypes.byref(cursor))
    try:
        give_back_focus(app.pid, target)
        # B2：点一下（鼠标先移上去：页面记下玩家在用的窗口）
        target.set_mark()
        sampler = Sampler()
        guard_input(app.pid, target.hwnd, strict=True)
        if root_at(cx, cy) != overlay:
            raise SystemExit(f"STOP: the window under the overlay's centre is {describe(root_at(cx, cy))}; not clicking")
        mouse_abs(cx, cy, 0)
        time.sleep(0.3)
        mouse_abs(cx, cy, LEFTDOWN)
        time.sleep(0.08)
        mouse_abs(cx, cy, LEFTUP)
        released = time.time() - sampler.t0
        time.sleep(1.5)
        returned_checks("B2 (click)", sampler.finish(), released, target)
        now = find_window(app.pid, OVERLAY_TITLE)
        print(f"    overlay after the click: {describe(now) if now else 'gone'} (same window: {now == overlay}, "
              f"visible: {bool(now and user32.IsWindowVisible(now))}, its thread answers: "
              f"{responsive(overlay) if now else '-'})")
        if ui:
            # 点在条上没有按钮的地方：页面收到按下就开始拖（按钮只在预览 / 出错时才有）
            seen = page_eval("#/overlay", OVERLAY_WATCH).get("ok") or {}
            check("B2: the overlay page got the press (its buttons and drag can work)",
                  "pointerdown" in seen.get("events", []), json.dumps(seen))
        if not give_back_focus(app.pid, target):
            raise SystemExit("STOP: could not give the test window its foreground back")
        # B3：拖 80 像素（要页面：按下时页面调 drag_overlay）
        if not ui:
            skip("B3", "no embedded frontend (the page starts the drag)")
            return
        time.sleep(3.5)  # 上一次按下没移动时，页面 3 秒后撤销“手动拖动”
        target.set_mark()
        sampler = Sampler()
        guard_input(app.pid, target.hwnd, strict=True)
        if root_at(cx, cy) != overlay:
            raise SystemExit(f"STOP: the window under the overlay's centre is {describe(root_at(cx, cy))}; not dragging")
        mouse_abs(cx + 5, cy, 0)
        time.sleep(0.3)
        mouse_abs(cx, cy, 0)
        time.sleep(0.15)
        mouse_abs(cx, cy, LEFTDOWN)
        time.sleep(0.4)
        for i in range(1, 11):
            mouse_abs(cx - 8 * i, cy, 0)
            time.sleep(0.04)
        time.sleep(0.15)
        mouse_abs(cx - 80, cy, LEFTUP)
        released = time.time() - sampler.t0
        time.sleep(1.5)
        moved = rect(overlay)
        seen = page_eval("#/overlay", OVERLAY_WATCH).get("ok") or {}
        check("B3: the overlay page got the press", "pointerdown" in seen.get("events", []), json.dumps(seen))
        check("B3: the overlay moved about 80 px left", abs((left - moved[0]) - 80) <= 12 and moved[1] == top,
              f"{(left, top)} → {(moved[0], moved[1])}")
        config = read_config()
        # 悬浮窗的设置已经搬进语音输入插件（plugins.voice-input.settings.overlay），旧配置在顶层
        saved = (config.get("plugins", {}).get("voice-input", {}).get("settings", {}).get("overlay")
                 or config.get("overlay", {}))
        check("B3: the new position is saved (overlay.position = custom)", saved.get("position") == "custom",
              json.dumps(saved))
        returned_checks("B3 (drag)", sampler.finish(), released, target)
    finally:
        user32.SetCursorPos(cursor.x, cursor.y)


def give_back_to(hwnd, app_pid):
    """At the end: the window that was in front before (the terminal) gets the foreground back, if nothing else took
    it meanwhile (the foreground is none, the test window or this Kotone)."""
    front = hval(user32.GetForegroundWindow())
    if not hwnd or front == hwnd or not user32.IsWindow(hwnd):
        return
    if front and window_pid(front) != app_pid and title_of(front) != "KotoneScriptTarget":
        return
    tap_alt_and_focus(hwnd)


def start_kotone(part):
    """A fresh isolated Kotone (`--background`); returns (process, embedded frontend?). Each part gets its own
    WebView2 data folder and debugging port: WebView2 hosts that share a data folder share one browser process, so a
    new Kotone would otherwise end up in the previous (killed) instance's browser process, with its old pages."""
    global DEVTOOLS
    DEVTOOLS = 9373 + part
    ENV["WEBVIEW2_USER_DATA_FOLDER"] = os.path.join(WORK, f"webview-{part}")
    ENV["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={DEVTOOLS}"
    app = subprocess.Popen([EXE, "--background"], env=ENV, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, creationflags=NO_WINDOW)
    status = wait_service(app)
    print(f"tool service up: {status.get('appVersion')} pid {status.get('pid')}", flush=True)
    time.sleep(4)  # WebView2 和页面加载完
    ui, urls = embedded_frontend()
    print(f"embedded frontend: {ui} ({urls[:200]})", flush=True)
    return app, ui


def stop_kotone(app):
    cli("tools", "call", "app.quit")
    try:
        app.wait(15)
    except subprocess.TimeoutExpired:
        print("    (Kotone did not quit in 15 s: killed)", flush=True)
        app.kill()
        app.wait(10)


def main():
    guard_start()
    started_from = hval(user32.GetForegroundWindow())
    if not os.path.exists(SRC_EXE):
        raise SystemExit(f"missing {SRC_EXE}: build it first (see the docstring)")
    try:
        os.makedirs(LOCK)
    except FileExistsError:
        raise SystemExit(f"{LOCK} exists: another real-machine test is running (remove it if stale)")
    app = target = None
    try:
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        shutil.copy2(SRC_EXE, EXE)
        write_config()
        print(f"exe: {SRC_EXE}", flush=True)
        # A 和 B 各用一个新起的 Kotone：关掉网页登录窗口以后 Kotone 的界面线程有时会卡住（A 里会打出来，
        # 和这里测的抢焦点无关，改之前的版本也一样），别让它带坏 B
        parts = [(name, fn) for name, fn in (("a", part_web_window), ("b", part_overlay)) if ONLY in ("", name)]
        for index, (name, part) in enumerate(parts):
            app, ui = start_kotone(index)
            if target is None:
                guard_start()  # 起测试窗口之前前台还要是终端
                target = Target()
                time.sleep(0.5)
                print(f"test window: {describe(target.hwnd)}; foreground {describe(user32.GetForegroundWindow())}",
                      flush=True)
            part(app, target, ui)
            stop_kotone(app)
            app = None
    finally:
        if target is not None:
            target.close()
            time.sleep(0.5)
            give_back_to(started_from, app.pid if app is not None else -1)
        if app is not None:
            stop_kotone(app)
        if "--keep" not in sys.argv:
            for _ in range(10):
                shutil.rmtree(WORK, ignore_errors=True)
                if not os.path.exists(WORK):
                    break
                time.sleep(1)
            print("cleaned up", WORK if not os.path.exists(WORK) else f"(could not delete {WORK})")
        try:
            os.rmdir(LOCK)
        except FileNotFoundError:
            print(f"(someone else removed {LOCK} while this ran)")
    failed = [n for n, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; FAILED: {failed}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
