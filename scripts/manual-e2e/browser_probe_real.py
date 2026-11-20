"""ADR 031 smoke without a model: drive the built-in browser directly through the debug-only `browser_probe`
command (same URL checks and page code the background task uses).

Real desktop app (debug build, web UI embedded, `--background`: main window hidden → the browser view is hidden,
like a player in a full-screen game), isolated KOTONE_HOME=target/browser-probe/home, no AI connection needed,
muted (KOTONE_BROWSER_MUTE), every debugging-protocol call traced into kotone.log (KOTONE_BROWSER_TRACE).
No OS input. Stops before starting if a game is in front.

P1. open https://example.com, snapshot → “Example Domain”, a link with a ref;
P2. open https://cn.bing.com, type “晴天 周杰伦” into the search box and submit, snapshot → results;
    click the first result link → the page moves on;
P3. blocked addresses are refused (localhost, 127.0.0.1, file:).
usage: python -X utf8 -I scripts/manual-e2e/browser_probe_real.py
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
WORK = os.path.join(REPO, "target", "browser-probe")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
PORT = 9338
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}",
           KOTONE_BROWSER_MUTE="1", KOTONE_BROWSER_TRACE="1")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def guard():
    path = foreground_process()
    name = os.path.basename(path).lower()
    if any(g in name for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


def invoke(command, args):
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
""" % (PORT, json.dumps(command), json.dumps(args, ensure_ascii=False))
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        raise RuntimeError((out.stdout + out.stderr)[-2000:])


def probe(action, arg=None):
    started = time.time()
    r = invoke("browser_probe", {"action": action, "arg": arg})
    return r, time.time() - started


def wait_ui(timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            if "ok" in invoke("browser_state", {}):
                return
        except Exception:  # noqa: BLE001
            pass
        time.sleep(1)
    raise RuntimeError("settings window did not come up")


def content(r):
    return (r.get("ok") or {}).get("content", "")


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
        r, s = probe("open", "https://example.com")
        check("P1: example.com opens", "ok" in r and not (r.get("ok") or {}).get("warning"),
              f"{s:.1f} s {json.dumps(r, ensure_ascii=False)[:200]}")
        r, s = probe("snapshot")
        text = content(r)
        check("P1: snapshot reads the page", (r.get("ok") or {}).get("title") == "Example Domain"
              and "documentation examples" in text, f"{s:.1f} s {json.dumps(r, ensure_ascii=False)[:300]}")
        check("P1: links have refs", re.search(r"link .*\[ref=\d+", text), text[-300:])
        state = invoke("browser_state", {}).get("ok") or {}
        check("P1: browser state follows the page", state.get("site") == "example.com" and state.get("open")
              and state.get("title") == "Example Domain",
              json.dumps(state, ensure_ascii=False))

        guard()
        r, s = probe("open", "https://cn.bing.com")
        check("P2: bing opens", "ok" in r, f"{s:.1f} s {json.dumps(r, ensure_ascii=False)[:200]}")
        r, s = probe("snapshot")
        text = content(r)
        box = re.search(r"(?:searchbox|textbox|combobox)[^\n]*\[ref=(\d+)", text)
        check("P2: the search box has a ref", box, text[:400])
        if box:
            r, s = probe("type", {"ref": int(box.group(1)), "text": "晴天 周杰伦", "submit": True})
            check("P2: typed and submitted", "ok" in r, f"{s:.1f} s {json.dumps(r, ensure_ascii=False)[:200]}")
            time.sleep(2)
            r, s = probe("snapshot")
            text = content(r)
            check("P2: results page", "晴天" in text and "search" in (r.get("ok") or {}).get("url", ""),
                  (r.get("ok") or {}).get("url", ""))
            links = [int(m.group(1)) for m in re.finditer(r"link \"[^\"]*晴天[^\"]*\" \[ref=(\d+), url=https?://(?!cn\.bing|www\.bing)", text)]
            if links:
                before = (r.get("ok") or {}).get("url", "")
                r, s = probe("click", links[0])
                time.sleep(2)
                after = (invoke("browser_state", {}).get("ok") or {}).get("url", "")
                check("P2: clicking a result moves on", "ok" in r and after and after != before,
                      f"{s:.1f} s → {after}")
            else:
                check("P2: a result link to click", False, text[:600])

        # P4（只记录）：网易云《晴天》歌曲页上的“播放”，不登录能不能放、music 读不读得到
        guard()
        r, s = probe("open", "https://music.163.com/#/song?id=186016")
        print(f"   P4 open: {s:.1f} s {json.dumps(r, ensure_ascii=False)[:160]}")
        time.sleep(2)
        r, s = probe("snapshot")
        text = content(r)
        play = re.search(r"(?:button|link)[^\n]*\"播放\"[^\n]*\[ref=(\d+)", text)
        print(f"   P4 snapshot: {s:.1f} s, {len(text)} chars, play button: {bool(play)}, login on page: "
              f"{'登录' in text}")
        if play:
            r, s = probe("click", int(play.group(1)))
            print(f"   P4 click play: {json.dumps(r, ensure_ascii=False)[:160]}")
            time.sleep(4)
            r, _ = probe("music", "now")
            print(f"   P4 music now: {json.dumps(r, ensure_ascii=False)[:200]}")
            state = invoke("browser_state", {}).get("ok") or {}
            print(f"   P4 browser state media: {state.get('media')}")
            if (r.get("ok") or {}).get("playing"):
                r, _ = probe("music", "pause")
                print(f"   P4 music pause: {json.dumps(r, ensure_ascii=False)[:160]}")

        for bad in ["http://localhost:1420", "http://127.0.0.1", "file:///C:/Windows/win.ini", "http://tauri.localhost"]:
            r, _ = probe("open", bad)
            check(f"P3: refuses {bad}", "error" in r, json.dumps(r, ensure_ascii=False)[:120])
    finally:
        try:
            invoke("browser_control", {"action": "close"})
        except Exception:  # noqa: BLE001
            pass
        app.terminate()
        time.sleep(2)
    log = open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    traces = [l for l in log.splitlines() if "browser: cdp" in l]
    print(f"\ndebugging-protocol calls traced: {len(traces)}; first lines:")
    for line in traces[:12]:
        print("  ", line[:200])
    failed = [n for n, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
