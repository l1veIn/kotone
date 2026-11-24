"""`kotone.exe --background` must not take the foreground (2026-11-25).

Found while testing auto cards: after `kotone.exe --background` the hidden "Kotone 设置" window became the foreground
window (`GetForegroundWindow` returned it), i.e. it took focus away from the terminal. When Kotone starts like this
while the player is in a game (an external Agent's `kotone_launch`, `kotone-cli app launch`), the game would lose
focus. Cause: the main window was created with `focus: true` (the default), so WebView2 moved keyboard focus into the
still-hidden window (wry `MoveFocus`), which activated it. Fix: `"focus": false` for `main` in tauri.conf.json, and
every place that shows the window (normal start, tray, second instance, clicking a message pop-up) goes through
`show_and_focus`: show, bring to front, and hand the keyboard focus to the page (the desktop app uses Tauri's
`unstable` multi-webview mode, where activating the window does not focus its WebView by itself).

What this checks, against a real debug Kotone in an isolated KOTONE_HOME (target/background-focus-real/home):

- default (`--background`): note the foreground window (hwnd + process) → start kotone.exe --background → sample the
  foreground every 50 ms until the tool service answers and 8 s more (WebView2 and the page finish loading) → the
  foreground never belonged to Kotone (or its WebView2 processes), it is the same window as before, the settings
  window exists, is hidden and is not the active window of its thread;
- `--open`: the same, then open the settings window the way clicking a message pop-up does (`auto_card_open`, sent
  over the WebView2 debugging port; same `show_and_focus` as the tray) → the window shows up, is the foreground window
  and the keyboard focus is in its WebView;
- `--normal`: start without `--background` → the settings window shows up, becomes the foreground window, and the
  keyboard focus is in its WebView.

`--open` and `--normal` bring a Kotone window to the front, so they only run when the foreground is a terminal; the
instance is closed right after. Sends no keys and no mouse input (Kotone itself may tap Alt inside `set_focus` when
Windows refuses the foreground, see tao `force_window_active`). Stops before starting if a game is running or in
front, or if the foreground already is a Kotone window. Takes target/tmp/real.lock. The exe is copied to
target/background-focus-real/bin first (a running exe would block other builds); the work directory is deleted
afterwards (`--keep` keeps it).
Prerequisites: `pnpm -C apps/desktop build:web`, then `cargo build -p kotone-tauri --features tauri/custom-protocol`
(the default mode works with any debug build; `--open` needs the embedded settings page); node + Playwright from
apps/desktop for `--open`.
usage: python -X utf8 -I scripts/manual-e2e/background_focus_real.py [--normal | --open] [--exe PATH] [--keep]
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
WORK = os.path.join(REPO, "target", "background-focus-real")
HOME = os.path.join(WORK, "home")
BIN = os.path.join(WORK, "bin")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
SRC_EXE = (os.path.abspath(sys.argv[sys.argv.index("--exe") + 1]) if "--exe" in sys.argv
           else os.path.join(REPO, "target", "debug", "kotone.exe"))
EXE = os.path.join(BIN, "kotone.exe")
NORMAL = "--normal" in sys.argv
OPEN = "--open" in sys.argv
DEVTOOLS = 9371
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
if OPEN:
    ENV["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={DEVTOOLS}"
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_EXE", None)
NO_WINDOW = 0x08000000  # CREATE_NO_WINDOW: the debug build is a console program; no console window may pop up
SETTLE = 8.0
GAMES = ("league of legends", "leagueclient", "riotclient", "valorant", "genshinimpact", "yuanshen", "starrail",
         "zenlesszonezero", "dota2", "cs2", "wow", "overwatch", "pubg", "naraka", "eldenring", "steam")
TERMINALS = ("windowsterminal.exe", "openconsole.exe", "conhost.exe", "cmd.exe", "powershell.exe", "pwsh.exe",
             "mintty.exe", "wezterm-gui.exe", "alacritty.exe", "warp.exe", "code.exe")
RESULTS = []

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
user32.GetForegroundWindow.restype = wt.HWND
user32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
user32.GetWindowThreadProcessId.restype = wt.DWORD
user32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [wt.HWND]
user32.IsChild.argtypes = [wt.HWND, wt.HWND]
ENUM_PROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
user32.EnumWindows.argtypes = [ENUM_PROC, wt.LPARAM]


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("flags", wt.DWORD), ("hwndActive", wt.HWND), ("hwndFocus", wt.HWND),
                ("hwndCapture", wt.HWND), ("hwndMenuOwner", wt.HWND), ("hwndMoveSize", wt.HWND),
                ("hwndCaret", wt.HWND), ("rcCaret", wt.RECT)]


user32.GetGUIThreadInfo.argtypes = [wt.DWORD, ctypes.POINTER(GUITHREADINFO)]


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def hval(hwnd):
    return int(hwnd or 0)


def window_pid(hwnd):
    pid = wt.DWORD()
    tid = user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value, tid


def process_path(pid):
    handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
        return buf.value
    finally:
        kernel32.CloseHandle(handle)


def describe(hwnd):
    """(hwnd, pid, exe name, title, class) of a window; hwnd 0 = no foreground window."""
    if not hval(hwnd):
        return {"hwnd": 0, "pid": 0, "exe": "", "title": "", "class": ""}
    pid, _ = window_pid(hwnd)
    title = ctypes.create_unicode_buffer(512)
    user32.GetWindowTextW(hwnd, title, 512)
    cls = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, cls, 256)
    return {"hwnd": hval(hwnd), "pid": pid, "exe": os.path.basename(process_path(pid)), "title": title.value,
            "class": cls.value}


def show(info):
    return f"{info['exe'] or '?'} pid {info['pid']} hwnd {info['hwnd']:#x} {info['class']!r} {info['title']!r}"


def foreground():
    return describe(user32.GetForegroundWindow())


def is_kotone(info, app_pid):
    exe = info["exe"].lower()
    return info["pid"] == app_pid or exe in ("kotone.exe", "msedgewebview2.exe")


def top_windows(pid):
    found = []

    def collect(hwnd, _):
        if window_pid(hwnd)[0] == pid:
            found.append(hwnd)
        return True

    user32.EnumWindows(ENUM_PROC(collect), 0)
    return found


def settings_window(pid):
    for hwnd in top_windows(pid):
        if describe(hwnd)["title"] == "Kotone 设置":
            return hwnd
    return None


def thread_info(tid):
    info = GUITHREADINFO(cbSize=ctypes.sizeof(GUITHREADINFO))
    if not user32.GetGUIThreadInfo(tid, ctypes.byref(info)):
        return None
    return info


def guard():
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, encoding="mbcs",
                         errors="replace").stdout.lower()
    running = [g for g in ("league of legends.exe", "leagueclient", "riotclient", "valorant", "cs2.exe",
                           "dota2.exe", "yuanshen", "genshinimpact") if g in out]
    if running:
        raise SystemExit(f"STOP: a game is running ({running}); not testing while the player plays")
    front = foreground()
    path = process_path(front["pid"]) if front["pid"] else ""
    if any(g in front["exe"].lower() for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({show(front)}); not testing")
    if front["exe"].lower() in ("kotone.exe", "msedgewebview2.exe"):
        raise SystemExit(f"STOP: a Kotone window is already in front ({show(front)}); the result would mean nothing")
    if (NORMAL or OPEN) and front["exe"].lower() not in TERMINALS:
        raise SystemExit(f"STOP: --normal / --open bring Kotone to the front; they only run when a terminal is in "
                         f"front (now: {show(front)})")
    return front


def cli(*args, timeout=30):
    out = subprocess.run([EXE, *args], capture_output=True, text=True, encoding="utf-8", errors="replace", env=ENV,
                         stdin=subprocess.DEVNULL, timeout=timeout, creationflags=NO_WINDOW)
    try:
        return out.returncode, json.loads(out.stdout)
    except json.JSONDecodeError:
        return out.returncode, {"raw": out.stdout[-500:]}


def wait_service(app, timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if app.poll() is not None:
            raise RuntimeError(f"kotone.exe exited early (code {app.returncode})")
        code, value = cli("tools", "call", "app.status", "--timeout-ms", "3000")
        if code == 0 and value.get("ok"):
            return value["result"]
        time.sleep(0.5)
    raise RuntimeError("the isolated instance's tool service did not come up")


class Sampler:
    """Samples the foreground window every 50 ms and keeps every change."""

    def __init__(self):
        self.changes = []
        self.stop = threading.Event()
        self.start = time.time()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        last = None
        while not self.stop.is_set():
            hwnd = hval(user32.GetForegroundWindow())
            if hwnd != last:
                self.changes.append((time.time() - self.start, describe(hwnd)))
                last = hwnd
            time.sleep(0.05)

    def finish(self):
        self.stop.set()
        self.thread.join(2)
        return self.changes


def write_config():
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}}}, f)


def check_background(app, before, changes):
    taken = [(t, info) for t, info in changes if is_kotone(info, app.pid)]
    for t, info in changes:
        print(f"  {t:6.2f}s foreground: {show(info)}")
    check("the foreground never was a Kotone window", not taken,
          "; ".join(f"{t:.2f}s {show(info)}" for t, info in taken))
    after = foreground()
    check("the foreground is the same window as before", after["hwnd"] == before["hwnd"],
          f"before {show(before)}, after {show(after)}")
    main = settings_window(app.pid)
    if not check("the settings window exists", main is not None,
                 "" if main else "; ".join(show(describe(h)) for h in top_windows(app.pid))):
        return
    check("the settings window stays hidden", not user32.IsWindowVisible(main))
    info = thread_info(window_pid(main)[1])
    active = hval(info.hwndActive) if info else -1
    check("the settings window is not its thread's active window", active != hval(main),
          f"active {active:#x}, settings {hval(main):#x}")


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


def invoke(command, args):
    """A Kotone command from the (hidden) settings page, over the WebView2 debugging port. No OS input."""
    body = "return await window.__TAURI_INTERNALS__.invoke(%s, %s);" % (json.dumps(command), json.dumps(args))
    code = PAGE_EVAL.replace("__PORT__", str(DEVTOOLS)).replace("__BODY__", body)
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"), creationflags=NO_WINDOW)
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        return {"error": (out.stdout + out.stderr)[-800:]}


def check_shown(app):
    deadline = time.time() + 20
    main = front = None
    while time.time() < deadline:
        main = settings_window(app.pid)
        front = foreground()
        if main and front["hwnd"] == hval(main):
            break
        time.sleep(0.1)
    if not check("the settings window exists", main is not None,
                 "" if main else "; ".join(show(describe(h)) for h in top_windows(app.pid))):
        return
    check("the settings window is visible", user32.IsWindowVisible(main))
    check("the settings window is the foreground window", front["hwnd"] == hval(main), f"foreground {show(front)}")
    # 键盘焦点进了 WebView（设置窗口的子窗口，属于 msedgewebview2 进程）：玩家一打开就能打字、按快捷键。
    # 只停在设置窗口本身不算：那样页面收不到键，要先点一下。
    start, last, focus = time.time(), None, 0
    while time.time() < start + SETTLE:
        info = thread_info(0)
        focus = hval(info.hwndFocus) if info else 0
        if focus != last:
            print(f"  {time.time() - start:5.2f}s keyboard focus: {show(describe(focus)) if focus else 'none'}")
            last = focus
        if focus and user32.IsChild(main, focus):
            break
        time.sleep(0.05)
    check("the keyboard focus is in the WebView inside the settings window", focus and user32.IsChild(main, focus),
          show(describe(focus)) if focus else "no focus")


def main():
    before = guard()
    if not os.path.exists(SRC_EXE):
        raise SystemExit(f"missing {SRC_EXE}: build it first (see the docstring)")
    try:
        os.makedirs(LOCK)
    except FileExistsError:
        raise SystemExit(f"{LOCK} exists: another real-machine test is running (remove it if stale)")
    app = None
    try:
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(BIN)
        shutil.copy2(SRC_EXE, EXE)
        write_config()
        mode = ("--normal (no --background)" if NORMAL else
                "--background, then open the settings window" if OPEN else "--background")
        print(f"exe: {SRC_EXE}\nmode: {mode}\nforeground before: {show(before)}", flush=True)
        sampler = Sampler()
        app = subprocess.Popen([EXE] if NORMAL else [EXE, "--background"], env=ENV, stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=NO_WINDOW)
        status = wait_service(app)
        ready = time.time() - sampler.start
        print(f"tool service up after {ready:.1f}s: {status.get('appVersion')} pid {status.get('pid')}", flush=True)
        if NORMAL:
            sampler.finish()
            check_shown(app)
        else:
            time.sleep(SETTLE)
            check_background(app, before, sampler.finish())
        if OPEN:
            # 和点消息提示一样（auto_card_open → show_and_focus，托盘走的也是它）；对话 id 是假的，窗口照样打开
            result = invoke("auto_card_open", {"conversationId": "background-focus-real", "key": "none"})
            print("  auto_card_open →", json.dumps(result, ensure_ascii=False)[:300], flush=True)
            check("auto_card_open ran from the settings page", "ok" in result,
                  json.dumps(result, ensure_ascii=False)[:300])
            check_shown(app)
    finally:
        if app is not None:
            cli("tools", "call", "app.quit")
            try:
                app.wait(15)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait(10)
        if "--keep" not in sys.argv:
            for _ in range(10):
                shutil.rmtree(WORK, ignore_errors=True)
                if not os.path.exists(WORK):
                    break
                time.sleep(1)
            print("cleaned up", WORK if not os.path.exists(WORK) else f"(could not delete {WORK})")
        os.rmdir(LOCK)
    failed = [n for n, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; FAILED: {failed}" if failed else ""))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
