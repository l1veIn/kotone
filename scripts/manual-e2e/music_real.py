"""ADR 034 real-machine check: music as a tool — the real app, the real NetEase servers, the real DeepSeek model.

Same setup as background_task_real.py (debug build with the web UI embedded, isolated KOTONE_HOME=target/music-real/home,
the player's real DeepSeek key in memory only, input tools denied, speech volume 0). The NetEase login of this isolated
home is stored under its own credential service (kotone@<dir>), never the player's real one. Music volume 12.

Without a login (default, `--background`, no window):
M1. `music_state`: NetEase, not logged in, nothing playing.
M2. `music_search 孤勇者`: results within a few seconds; the 陈奕迅 original is marked vip; covers are full.
M3. the Music page (sidebar → 音乐) shows the scan card; screenshot.
M4. `music_login_start`: a QR (SVG) on the page; `music_login_cancel` puts it away.
M5. `music_play` while logged out refuses with a scan hint and starts the QR (D5).
M6. typed “我要听孤勇者” → she uses `music` directly (no agent.start) and asks the player to scan.

`--login` (window shown, only these cases; the player scans with the NetEase Cloud Music app; waits up to 3 minutes):
L1. after the scan the account shows up; `music_home` has daily picks and playlists (我喜欢的音乐 first).
L2. typed “放每日推荐” → playing within ~15 s, position moves, the sidebar mini player shows it.
L3. typed “暂停一下” / “继续放” / “这是什么歌” → `music`, paused / playing again / she names it.
L4. typed “我要听孤勇者” → playing; when the account isn't VIP the original is skipped and she says why.
L5. `music_control stop`; screenshots of the playing page. The test home is logged out at the end.
usage: python -X utf8 -I scripts/manual-e2e/music_real.py [--login]
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import background_task_real as bt  # noqa: E402
import real_deepseek  # noqa: E402

REPO = bt.REPO
bt.WORK = os.path.join(REPO, "target", "music-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9338
bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
              WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}")
bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
check = bt.check
SHOTS = os.path.join(bt.WORK, "shots")
LOGIN = "--login" in sys.argv


def ui(actions, shot=None):
    """Run Playwright `actions` (JS, `page` in scope) on the settings window; optional screenshot."""
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
  const out = await (async () => { %s })();
  %s
  console.log(JSON.stringify({ ok: out ?? null }));
} catch (e) { console.log(JSON.stringify({ error: String(e) })); }
finally { await browser.close(); }
""" % (bt.PORT, actions, f"await page.screenshot({{ path: {json.dumps(os.path.join(SHOTS, shot))} }});" if shot else "")
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        return {"error": (out.stdout + out.stderr)[-500:]}


def music_state():
    return bt.invoke("music_state", {}).get("ok") or {}


def main_tools(main, text):
    msgs = bt.conversation(main).get("messages", [])
    start = next((i for i, m in enumerate(msgs) if m["role"] == "user" and m.get("text") == text), 0)
    return [c["name"] for m in msgs[start:] if m["role"] == "assistant" for c in m.get("toolCalls", [])]


def tool_results(main, text, name):
    msgs = bt.conversation(main).get("messages", [])
    start = next((i for i, m in enumerate(msgs) if m["role"] == "user" and m.get("text") == text), 0)
    return [m.get("content", "") for m in msgs[start:] if m["role"] == "tool" and m.get("name") == name]


def say(name, text, timeout=180):
    r = bt.run_case(name, text, answer="deny", timeout=timeout)
    tools = main_tools(r["main"], text) if r else []
    print(f"   [{name}] main tools: {tools}")
    return r or {}, tools


def logged_out_cases():
    s = music_state()
    check("M1: NetEase, not logged in, nothing playing",
          s.get("provider") == "netease" and s.get("account") is None and s["player"]["status"] == "idle",
          json.dumps({k: s.get(k) for k in ("account", "login", "quality")}, ensure_ascii=False)[:200])

    t = time.time()
    r = bt.invoke("music_search", {"query": "孤勇者", "offset": 0})
    tracks = (r.get("ok") or {}).get("tracks", [])
    took = time.time() - t
    check("M2: search answers in a few seconds", tracks and took < 5, f"{len(tracks)} tracks in {took:.1f} s")
    original = next((x for x in tracks if "陈奕迅" in x.get("artists", []) and x["name"] == "孤勇者"), None)
    check("M2: the 陈奕迅 original is marked vip", original and original["playable"] == "vip",
          json.dumps(original, ensure_ascii=False)[:160] if original else "not found")
    full = [x for x in tracks if x["playable"] == "full"]
    check("M2: some versions are fully playable", full, f"{len(full)} full")

    r = ui('await page.getByTestId("settings-nav-music").click(); await page.getByTestId("music-page").waitFor();'
           'await page.waitForTimeout(800); return await page.getByTestId("music-login-card").isVisible();',
           "m3-logged-out.png")
    check("M3: the Music page shows the scan card", r.get("ok") is True, json.dumps(r, ensure_ascii=False)[:200])

    r = bt.invoke("music_login_start", {})
    login = (r.get("ok") or {}).get("login", {})
    check("M4: a QR code to scan", login.get("stage") == "waiting" and (login.get("qrSvg") or "").startswith("<svg"),
          json.dumps({"stage": login.get("stage"), "qr": (login.get("qrSvg") or "")[:30]}, ensure_ascii=False))
    r = ui('await page.getByTestId("music-login-qr").waitFor({ timeout: 5000 }); return true;', "m4-qr.png")
    check("M4: the QR is on the page", r.get("ok") is True, json.dumps(r, ensure_ascii=False)[:200])
    r = bt.invoke("music_login_cancel", {})
    check("M4: cancel puts it away", ((r.get("ok") or {}).get("login") or {}).get("stage") == "idle")

    if full:
        r = bt.invoke("music_play", {"tracks": full[:1], "index": 0})
        check("M5: playing while logged out asks for a scan (D5)", "error" in r and "扫" in r["error"], r.get("error", "")[:120])
        check("M5: …and the QR is up", music_state().get("login", {}).get("stage") == "waiting")
        bt.invoke("music_login_cancel", {})

    text = "我要听孤勇者"
    r, tools = say("M6", text)
    check("M6: she used music directly (no background)", "music" in tools and "agent.start" not in tools, ",".join(tools))
    results = tool_results(r.get("main", ""), text, "music")
    check("M6: the tool said the player must log in", any("need_login" in x for x in results), (results or ["-"])[-1][:160])
    said = r.get("said", "")
    check("M6: she asks the player to scan", any(w in said for w in ("扫", "登录")), said[:120])
    bt.invoke("music_login_cancel", {})


def logged_in_cases():
    ui('await page.getByTestId("settings-nav-music").click(); await page.getByTestId("music-page").waitFor(); return true;')
    bt.invoke("music_login_start", {})
    ui('await page.getByTestId("music-login-qr").waitFor({ timeout: 5000 }); return true;', "l0-scan-me.png")
    print("\n>>> 请用网易云音乐 App 扫一扫 Kotone 窗口里音乐页的二维码，并在手机上确认（最多等 3 分钟）\n")
    account = bt.wait_until(lambda: music_state().get("account"), 200, 2)
    if not check("L1: logged in after the scan", account, json.dumps(account, ensure_ascii=False)[:120] if account else "timed out"):
        return
    print(f"   account: {account.get('nickname')} vip={account.get('vip')}")
    home = bt.invoke("music_home", {}).get("ok") or {}
    lists = home.get("playlists", [])
    check("L1: daily picks and playlists", home.get("daily") and lists, f"{len(home.get('daily', []))} daily, {len(lists)} playlists")
    check("L1: 我喜欢的音乐 first", lists and lists[0].get("liked"), lists[0].get("name") if lists else "-")

    text = "放每日推荐"
    started = time.time()
    r, tools = say("L2", text)
    playing = bt.wait_until(lambda: music_state()["player"]["status"] == "playing" and music_state()["player"], 20, 0.5)
    check("L2: daily picks are playing", "music" in tools and playing,
          f"{time.time() - started:.0f} s: {(playing or {}).get('track', {}).get('name') if playing else music_state()['player']}")
    if playing:
        p0 = music_state()["player"]["positionMs"]
        time.sleep(3)
        p1 = music_state()["player"]["positionMs"]
        check("L2: the position moves", p1 > p0, f"{p0} → {p1} ms")
        r2 = ui('return await page.getByTestId("music-player-title").innerText();', "l2-playing.png")
        check("L2: the player bar shows the song", r2.get("ok"), json.dumps(r2, ensure_ascii=False)[:120])

    for name, text, want in [("L3 pause", "暂停一下", "paused"), ("L3 resume", "继续放", "playing")]:
        r, tools = say(name, text)
        status = bt.wait_until(lambda: music_state()["player"]["status"] == want, 5, 0.5)
        check(f"{name}: {want}", "music" in tools and status, f"{music_state()['player']['status']} / {tools}")
    r, tools = say("L3 now", "这是什么歌")
    title = (music_state()["player"].get("track") or {}).get("name", "")
    check("L3: she names the song", "music" in tools and title and title in r.get("said", ""), f"{title} / {r.get('said', '')[:80]}")

    text = "我要听孤勇者"
    r, tools = say("L4", text)
    track = bt.wait_until(lambda: music_state()["player"]["status"] == "playing" and music_state()["player"]["track"], 20, 0.5)
    results = tool_results(r.get("main", ""), text, "music")
    print(f"   L4 tool result: {(results or ['-'])[-1][:200]}")
    check("L4: 孤勇者 is playing", track and "孤勇者" in track.get("name", ""), json.dumps(track, ensure_ascii=False)[:120])
    if not account.get("vip"):
        check("L4: not VIP → the original was skipped and she says why", any("VIP" in x or "vip" in x for x in results)
              and ("VIP" in r.get("said", "") or "会员" in r.get("said", "")), r.get("said", "")[:120])

    bt.invoke("music_control", {"action": "stop"})
    check("L5: stopped", music_state()["player"]["status"] in ("idle", "paused"), music_state()["player"]["status"])
    bt.invoke("music_logout", {})


def main():
    bt.guard()
    shutil.rmtree(bt.WORK, ignore_errors=True)
    os.makedirs(bt.HOME)
    os.makedirs(SHOTS)
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "music": {"volume": 12},
                   "agent": {"permissions": {"mode": "allow-all", "rules": bt.NO_INPUT}}}, f)
    args = [bt.EXE] if LOGIN else [bt.EXE, "--background"]
    app = subprocess.Popen(args, env=bt.ENV)
    try:
        bt.wait_ui()
        cid = real_deepseek.add_connection(bt.EXE, bt.ENV, model="deepseek-flash")
        bt.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        bt.invoke("agent_new_conversation", {})
        # --login 只跑登录以后的（没登录的那几项会弹出又收起二维码，玩家容易扫错）
        if LOGIN:
            logged_in_cases()
        else:
            logged_out_cases()
    finally:
        try:
            bt.invoke("music_control", {"action": "stop"})
            bt.invoke("music_logout", {})
        except Exception:  # noqa: BLE001
            pass
        real_deepseek.delete_connection(bt.EXE, bt.ENV)
        try:
            bt.call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
        bt.sweep_credentials()
    print(f"\nscreenshots: {SHOTS}")
    return bt.finish()


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
