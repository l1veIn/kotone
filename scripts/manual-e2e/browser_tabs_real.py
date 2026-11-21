"""ADR 031 full browser, no model: tabs, address bar, back / forward, new windows → new tabs, her own tab,
page fullscreen, the download notice and a test extension — driven through the real UI commands
(`browser_navigate`, `browser_tab_*`, `browser_control`, `browser_extension_*`) and the debug-only `browser_probe`
(the same page code her `browser.*` tools use).

Real desktop app (debug build, `--background`: main window hidden → browser views hidden, like a player in a
full-screen game), isolated KOTONE_HOME=target/browser-tabs/home, no AI connection, muted (KOTONE_BROWSER_MUTE).
No OS input. Stops before starting if a game is in front.

usage: python -X utf8 -I scripts/manual-e2e/browser_tabs_real.py [--show]
       --show: also check page fullscreen (shows the main window for a few seconds; no keys, no mouse)
       (KOTONE_EXE=target/release/kotone.exe also works, without the `browser_probe` parts)
"""
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
WORK = os.path.join(REPO, "target", "browser-tabs")
HOME = os.path.join(WORK, "home")
EXE = os.environ.get("KOTONE_EXE") or os.path.join(REPO, "target", "debug", "kotone.exe")
DEBUG = "release" not in EXE.replace("\\", "/").split("/")
FIXTURE = os.path.join(HERE, "fixtures", "browser-extension-title")
PORT = 9339
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}",
           KOTONE_BROWSER_MUTE="1")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
RESULTS = []
MARK = "[KOTONE-EXT] "


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def guard():
    path = foreground_process()
    name = os.path.basename(path).lower()
    if any(g in name for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


def invoke(command, args=None):
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
  const r = await page.evaluate(async ([c, a]) => {
    try { return { ok: await window.__TAURI_INTERNALS__.invoke(c, a) }; } catch (e) { return { error: String(e) }; }
  }, [%s, %s]);
  console.log(JSON.stringify(r));
} finally { await browser.close(); }
""" % (PORT, json.dumps(command), json.dumps(args or {}, ensure_ascii=False))
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        raise RuntimeError((out.stdout + out.stderr)[-2000:])


def probe(action, arg=None):
    return invoke("browser_probe", {"action": action, "arg": arg})


def state():
    return invoke("browser_state").get("ok") or {}


def short(value):
    return json.dumps(value, ensure_ascii=False)[:400]


def wait_for(predicate, timeout=20):
    """Poll browser_state until predicate(state) holds; returns the last state."""
    deadline = time.time() + timeout
    s = state()
    while time.time() < deadline:
        if predicate(s):
            return s
        time.sleep(0.7)
        s = state()
    return s


def tab(s, tab_id):
    return next((t for t in s.get("tabs", []) if t["id"] == tab_id), {})


def wait_ui(timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            if "ok" in invoke("browser_state"):
                return
        except Exception:  # noqa: BLE001
            pass
        time.sleep(1)
    raise RuntimeError("settings window did not come up")


def main():
    guard()
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True,
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    try:
        wait_ui()
        run_cases()
    finally:
        try:
            for t in state().get("tabs", []):
                invoke("browser_tab_close", {"id": t["id"]})
        except Exception:  # noqa: BLE001
            pass
        app.terminate()
        time.sleep(2)
    log = open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    lines = [l for l in log.splitlines() if "browser:" in l and "cdp" not in l]
    print(f"\nbrowser log lines: {len(lines)}; last ones:")
    for line in lines[-25:]:
        print("  ", line[:220])
    failed = [n for n, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


def run_cases():
    # T0 插件：浏览器还没开时加进去（下次打开时装）；不是插件的文件夹被拒绝
    r = invoke("browser_extension_add", {"path": FIXTURE})
    ext = r.get("ok") or {}
    check("T0: the test extension is added before the browser opens", ext.get("name") == "Kotone test: title marker"
          and ext.get("enabled") and not ext.get("installed"), short(r))
    r = invoke("browser_extension_add", {"path": HERE})
    check("T0: a folder without manifest.json is refused", "error" in r, short(r))
    listed = invoke("browser_extensions").get("ok") or []
    check("T0: the extension is listed", [e.get("id") for e in listed] == [ext.get("id")], short(listed))

    # T1 地址栏：网址 → 新开标签页打开；插件装上并跑起来（标题前加了标记）
    guard()
    r = invoke("browser_navigate", {"url": "example.com"})
    first = (r.get("ok") or {}).get("tab")
    s = wait_for(lambda s: s.get("url") == "https://example.com/" and not s.get("loading"))
    check("T1: the address bar opens a URL in a new tab", first and len(s.get("tabs", [])) == 1
          and s.get("activeTab") == first and s.get("site") == "example.com", short(s))
    s = wait_for(lambda s: (s.get("title") or "").startswith(MARK), timeout=25)
    check("T1: the extension runs on the page (title marked)", (s.get("title") or "").startswith(MARK),
          str(s.get("title")))
    listed = invoke("browser_extensions").get("ok") or []
    check("T1: the extension is installed in the browser", listed and listed[0].get("installed"), short(listed))
    if DEBUG:
        marked = probe("eval", "document.documentElement.dataset.kotoneExt || null")
        check("T1: the content script marked the document", marked.get("ok") == "1", short(marked))

    # T2 地址栏：搜索词 → 必应
    r = invoke("browser_navigate", {"url": "晴天 周杰伦"})
    s = wait_for(lambda s: (s.get("url") or "").startswith("https://cn.bing.com/search") and not s.get("loading"))
    check("T2: words in the address bar search Bing", (s.get("url") or "").startswith("https://cn.bing.com/search?q="),
          str(s.get("url")))
    s = wait_for(lambda s: tab(s, first).get("canGoBack"))
    check("T2: the tab can go back", tab(s, first).get("canGoBack"), short(tab(s, first)))
    r = invoke("browser_navigate", {"url": "http://127.0.0.1:1420"})
    check("T2: the address bar refuses local addresses", "error" in r, short(r))

    # T3 后退 / 前进
    invoke("browser_control", {"action": "back"})
    s = wait_for(lambda s: s.get("url") == "https://example.com/" and tab(s, first).get("canGoForward"))
    check("T3: back returns to example.com", s.get("url") == "https://example.com/"
          and tab(s, first).get("canGoForward"), short(tab(s, first)))
    check("T3: no blank page before the first page (cannot go back further)", not tab(s, first).get("canGoBack"),
          short(tab(s, first)))
    invoke("browser_control", {"action": "forward"})
    s = wait_for(lambda s: (s.get("url") or "").startswith("https://cn.bing.com/search"))
    check("T3: forward goes to the search again", (s.get("url") or "").startswith("https://cn.bing.com/search"),
          str(s.get("url")))
    r = invoke("browser_control", {"action": "stop"})
    check("T3: stop is accepted", "ok" in r, short(r))

    # T4 第二个标签页：新开、切换、关掉
    guard()
    r = invoke("browser_tab_new", {"url": "example.org"})
    second = (r.get("ok") or {}).get("tab")
    s = wait_for(lambda s: s.get("activeTab") == second and s.get("url") == "https://example.org/")
    check("T4: a second tab opens and becomes current", len(s.get("tabs", [])) == 2 and s.get("activeTab") == second
          and s.get("url") == "https://example.org/", short(s))
    invoke("browser_tab_select", {"id": first})
    s = state()
    check("T4: switching back shows the first tab", s.get("activeTab") == first
          and (s.get("url") or "").startswith("https://cn.bing.com/search"), str(s.get("url")))
    check("T4: the other tab keeps its page", tab(s, second).get("url") == "https://example.org/", short(tab(s, second)))
    invoke("browser_tab_close", {"id": second})
    s = state()
    check("T4: closing a tab", [t["id"] for t in s.get("tabs", [])] == [first] and s.get("activeTab") == first,
          short(s.get("tabs")))
    r = invoke("browser_tab_select", {"id": "nope"})
    check("T4: an unknown tab is refused", "error" in r, short(r))
    sites = json.load(open(os.path.join(HOME, "browser-sites.json"), encoding="utf-8"))
    check("T4: sites the player opened are approved for her",
          {"example.com", "example.org", "cn.bing.com"} <= set(sites.get("sites", [])), short(sites))

    if DEBUG:
        her_tab_cases(first)

    # T7 上限 8 个
    before = len(state().get("tabs", []))
    made = []
    for _ in range(8 - before):
        r = invoke("browser_tab_new", {})
        made.append((r.get("ok") or {}).get("tab"))
    r = invoke("browser_tab_new", {})
    check("T7: the ninth tab is refused", "error" in r and "8" in r["error"], short(r))
    for t in made:
        if t:
            invoke("browser_tab_close", {"id": t})
    check("T7: extra tabs closed", len(state().get("tabs", [])) == before, str(len(state().get("tabs", []))))

    # T10 插件开关、删除
    invoke("browser_tab_select", {"id": first})
    invoke("browser_navigate", {"url": "https://example.com/"})
    wait_for(lambda s: s.get("url") == "https://example.com/" and not s.get("loading"))
    r = invoke("browser_extension_toggle", {"id": ext.get("id"), "enabled": False})
    check("T10: the extension can be turned off", "ok" in r and not r["ok"].get("enabled"), short(r))
    s = wait_for(lambda s: s.get("title") == "Example Domain", timeout=15)
    check("T10: turned off, the page is unmarked after reload", s.get("title") == "Example Domain", str(s.get("title")))
    r = invoke("browser_extension_toggle", {"id": ext.get("id"), "enabled": True})
    s = wait_for(lambda s: (s.get("title") or "").startswith(MARK), timeout=15)
    check("T10: turned on again, it runs again", (s.get("title") or "").startswith(MARK), str(s.get("title")))
    r = invoke("browser_extension_remove", {"id": ext.get("id")})
    check("T10: the extension can be removed", "ok" in r and invoke("browser_extensions").get("ok") == [], short(r))
    cfg = json.load(open(os.path.join(HOME, "config.json"), encoding="utf-8"))
    check("T10: the settings no longer list it", cfg.get("browser", {}).get("extensions") == [],
          short(cfg.get("browser")))

    # T11 全部关掉
    for t in state().get("tabs", []):
        invoke("browser_tab_close", {"id": t["id"]})
    s = state()
    check("T11: closing every tab closes the browser", not s.get("open") and s.get("tabs") == []
          and s.get("activeTab") is None, short(s))


def her_tab_cases(first):
    # T5 她的标签页：第一次 browser.open 新开一个并切过去；玩家的标签页不动
    guard()
    player_url = state().get("url")
    r = probe("open", "https://example.com/?her=1")
    s = state()
    hers = next((t["id"] for t in s.get("tabs", []) if t.get("agent")), None)
    check("T5: her first open makes her own tab and switches to it", "ok" in r and hers and hers != first
          and s.get("activeTab") == hers, short(s.get("tabs")))
    check("T5: the player's tab is untouched", tab(s, first).get("url") == player_url and not tab(s, first).get("agent"),
          short(tab(s, first)))
    invoke("browser_tab_select", {"id": first})
    r = probe("open", "https://example.org/?her=2")
    s = state()
    check("T5: later opens stay in her tab, the player keeps looking at theirs",
          tab(s, hers).get("url") == "https://example.org/?her=2" and s.get("activeTab") == first
          and s.get("url") == player_url, short(s.get("tabs")))
    r = probe("snapshot")
    check("T5: she reads her tab while the player looks at another", (r.get("ok") or {}).get("url", "")
          .startswith("https://example.org/?her=2"), short(r)[:200])

    # T6 她的页面里点开 target=_blank 链接 → 新标签页，还是她的
    invoke("browser_tab_select", {"id": hers})
    probe("eval", "(() => { const a = document.createElement('a'); a.href = 'https://example.com/?from=blank';"
                  " a.target = '_blank'; a.textContent = 'open in new window'; document.body.prepend(a); return true; })()")
    snap = probe("snapshot")
    m = re.search(r'link "open in new window" \[ref=(\d+)', (snap.get("ok") or {}).get("content", ""))
    if check("T6: the test link has a ref", m, short(snap)[:300]):
        probe("click", int(m.group(1)))
        s = wait_for(lambda s: any("from=blank" in (t.get("url") or "") for t in s.get("tabs", [])))
        new = next((t for t in s.get("tabs", []) if "from=blank" in (t.get("url") or "")), {})
        check("T6: target=_blank opened a new tab (hers, current)", new and new.get("agent")
              and s.get("activeTab") == new.get("id") and tab(s, hers).get("url") == "https://example.org/?her=2",
              short(s.get("tabs")))
        hers = new.get("id") or hers

    # T8 下载被拦下，提示玩家一句，过一会儿自己消失
    probe("eval", "(() => { const a = document.createElement('a'); a.href = 'data:text/plain,kotone';"
                  " a.download = 'kotone-test.txt'; a.textContent = 'download test file'; document.body.prepend(a);"
                  " return true; })()")
    snap = probe("snapshot")
    m = re.search(r'link "download test file" \[ref=(\d+)', (snap.get("ok") or {}).get("content", ""))
    if check("T8: the download link has a ref", m, short(snap)[:300]):
        probe("click", int(m.group(1)))
        s = wait_for(lambda s: s.get("notice"), timeout=10)
        check("T8: a refused download shows a notice", "下载" in (s.get("notice") or "")
              and not s.get("error"), str(s.get("notice")))
        s = wait_for(lambda s: not s.get("notice"), timeout=15)
        check("T8: the notice goes away by itself", not s.get("notice"), str(s.get("notice")))
        downloads = os.path.join(os.path.expanduser("~"), "Downloads")
        check("T8: nothing was downloaded", not os.path.isdir(downloads) or not any(
            f.startswith("kotone-test") for f in os.listdir(downloads)))

    # T9 页面全屏（视频的全屏按钮）→ fullscreenElement。浏览器只把全屏给看得见、有焦点的页面
    # （隐藏着时报 “TypeError: not granted”），所以要 --show：主窗口会出现几秒。
    if "--show" not in sys.argv:
        print("SKIP T9: page fullscreen needs a visible, focused page — run with --show")
    else:
        fullscreen_case(hers)

    # 关掉她的标签页：玩家的还在，她下次再开新的
    invoke("browser_tab_close", {"id": hers})
    s = state()
    check("T5: closing her tab leaves the player's tab", first in [t["id"] for t in s.get("tabs", [])]
          and not any(t.get("agent") for t in s.get("tabs", [])), short(s.get("tabs")))
    for t in s.get("tabs", []):
        if t["id"] != first:
            invoke("browser_tab_close", {"id": t["id"]})


def fullscreen_case(hers):
    probe("eval", "(() => { const b = document.createElement('button'); b.textContent = 'go fullscreen';"
                  " b.onclick = () => document.documentElement.requestFullscreen().then(() => window.__fs = 'ok',"
                  " (e) => window.__fs = String(e)); document.body.prepend(b);"
                  " return true; })()")
    snap = probe("snapshot")
    m = re.search(r'button "go fullscreen" \[ref=(\d+)', (snap.get("ok") or {}).get("content", ""))
    if check("T9: the fullscreen button has a ref", m, short(snap)[:300]):
        # 网页隐藏、没焦点时浏览器不给全屏（“not granted”）：给视图一块可见的区域，
        invoke("browser_set_bounds", {"x": 0, "y": 0, "width": 800, "height": 600, "visible": True})
        # 显示主窗口、把焦点给网页（不按键、不动鼠标）。
        guard()
        invoke("plugin:window|show", {"label": "main"})
        time.sleep(1)
        probe("focus")
        time.sleep(1)
        # 全屏要玩家亲手点（页面内 click() 不算）：经浏览器的输入管线点（不是系统输入）。
        probe("mouse", int(m.group(1)))
        s = wait_for(lambda s: tab(s, hers).get("fullscreenElement"), timeout=8)
        fs = probe("eval", "({ result: window.__fs || null, element: !!document.fullscreenElement,"
                           " visible: document.visibilityState, enabled: document.fullscreenEnabled })")
        check("T9: page fullscreen is reported", tab(s, hers).get("fullscreenElement"),
              f"{short(tab(s, hers))} page: {short(fs)}")
        probe("eval", "document.exitFullscreen().then(() => true, () => false)")
        s = wait_for(lambda s: not tab(s, hers).get("fullscreenElement"), timeout=8)
        check("T9: leaving fullscreen is reported", not tab(s, hers).get("fullscreenElement"), short(tab(s, hers)))
        invoke("browser_set_bounds", {"x": 0, "y": 0, "width": 0, "height": 0, "visible": False})
        invoke("plugin:window|hide", {"label": "main"})


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
