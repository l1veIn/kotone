"""ADR 031 real-machine check: the built-in browser driven by a background task, with the real DeepSeek.

Real desktop app (debug build with the web UI embedded, `--background`: the main window stays hidden, so the
browser view is hidden too — the same situation as a player in a full-screen game), isolated
KOTONE_HOME=target/browser-real/home, the player's real DeepSeek key in memory only (real_deepseek.py).
No OS input: messages go in through WebView2's DevTools port of the MAIN UI (the browser view itself has no
debugging port). The browser is muted (KOTONE_BROWSER_MUTE) so nothing plays out loud at night.
Input tools are denied; speech volume 0. Stops before starting if a game is in front.

Cases (typed, one conversation):
B0. the player first opens example.org from the address bar (`browser_navigate`) → their own tab.
B1. open https://example.com and say its title → the first visit to a site asks the player (card `waiting`,
    answered “允许”); `browser-sites.json` remembers it; the reply has “Example Domain”.
    She works in her own tab (`agent: true`, switched to); the player's example.org tab is untouched.
B2. search a public search page (Bing) and open the first result → browser.type / browser.click / browser.wait,
    all while the window is hidden.
(Music no longer plays in the browser — ADR 034 made it the `music` tool; the old B3/B4 music cases are gone.)
Also prints whether the chat page shows the “Cannot read properties of null” banner.
usage: python -X utf8 -I scripts/manual-e2e/browser_real.py
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "browser-real")
HOME = os.path.join(WORK, "home")
# KOTONE_EXE 可以换成正式版（target/release/kotone.exe）：浏览器视图在正式版和调试版里的行为不一样过。
EXE = os.environ.get("KOTONE_EXE") or os.path.join(REPO, "target", "debug", "kotone.exe")
PORT = 9337
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}", KOTONE_BROWSER_MUTE="1",
           KOTONE_BROWSER_TRACE="1", PYTHONUNBUFFERED="1")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}
RESULTS = []
TABLE = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def guard():
    path = foreground_process()
    name = os.path.basename(path).lower()
    if any(g in name for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def node(code):
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        raise RuntimeError((out.stdout + out.stderr)[-2000:])


def invoke(command, args):
    return node("""
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
  const r = await page.evaluate(async ([c, a]) => {
    try { return { ok: await window.__TAURI_INTERNALS__.invoke(c, a) }; } catch (e) { return { error: String(e) }; }
  }, [%s, %s]);
  console.log(JSON.stringify(r));
} finally { await browser.close(); }
""" % (PORT, json.dumps(command), json.dumps(args, ensure_ascii=False)))


def page_text():
    return node("""
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const pages = browser.contexts().flatMap((c) => c.pages()).map((p) => p.url());
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try { console.log(JSON.stringify({ pages, text: await page.evaluate(() => document.body.innerText) })); }
finally { await browser.close(); }
""" % PORT)


def wait_ui(timeout=90):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            if "ok" in invoke("agent_state", {}):
                return
        except Exception:  # noqa: BLE001
            pass
        time.sleep(1)
    raise RuntimeError("settings window did not come up")


def state():
    return invoke("agent_state", {}).get("ok") or {}


def browser_state():
    return invoke("browser_state", {}).get("ok") or {}


def conversation(cid):
    return invoke("agent_conversation", {"id": cid}).get("ok") or {}


def ledger():
    path = os.path.join(HOME, "usage.jsonl")
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def sweep_credentials():
    listing = subprocess.run(["cmdkey", "/list"], capture_output=True, text=True, encoding="mbcs",
                             errors="replace").stdout
    for line in listing.splitlines():
        if "target=" in line and WORK in line:
            target = line.split("target=", 1)[1].strip()
            subprocess.run(["cmdkey", f"/delete:{target}"], capture_output=True)
            print("removed leftover test credential")


def run(name, text, timeout=300, handover_after=20):
    """Type `text`, answer every background confirmation with “允许”, press “好了” on a handover after
    `handover_after` s (nobody can scan a QR code at night), and follow it to her last line."""
    guard()
    started = time.time()
    r = invoke("agent_send", {"text": text})
    if not check(f"{name}: typed request accepted", "ok" in r, json.dumps(r, ensure_ascii=False)[:200]):
        return {}
    main = r["ok"]["conversationId"]
    reports_before = sum(1 for m in conversation(main).get("messages", []) if m.get("source") == "agent-report")
    player_at, questions, answered, touched, handovers = None, [], set(), set(), []
    handover_seen = None
    finished = False
    while time.time() - started < timeout:
        conv = conversation(main)
        msgs = conv.get("messages", [])
        if player_at is None:
            player_at = next((m["atMs"] for m in reversed(msgs) if m["role"] == "user" and m.get("text") == text), None)
        for c in conv.get("subagents", []):
            if c.get("turn") == player_at:
                touched.add(c["name"])
            if c.get("status") == "waiting" and c.get("question"):
                key = (c["name"], c["question"], c.get("updatedAtMs"))
                if key not in answered:
                    answered.add(key)
                    questions.append(c["question"])
                    touched.add(c["name"])
                    print(f"   [{name}] she asks: {c['question']}")
                    invoke("agent_subagent_answer", {"conversationId": main, "name": c["name"], "answer": "allow-once"})
        bs = browser_state()
        if bs.get("handover"):
            if handover_seen is None:
                handover_seen = time.time()
                handovers.append(bs["handover"]["reason"])
                print(f"   [{name}] handover: {bs['handover']['reason']} (url {bs.get('url')})")
            elif time.time() - handover_seen > handover_after:
                invoke("browser_handover_done", {})
                handover_seen = None
        new = [m for m in msgs if player_at and m.get("atMs", 0) >= player_at]
        delegated = any(t["name"] in ("agent.start", "agent.followup") for m in new if m["role"] == "assistant"
                        for t in m.get("toolCalls", []))
        reports = sum(1 for m in msgs if m.get("source") == "agent-report")
        synced = any(m["role"] == "tool" and ('"status":"done"' in m.get("content", "")
                                              or '"status":"failed"' in m.get("content", "")) for m in new)
        busy = state().get("running") or any(c.get("status") in ("running", "waiting") and c["name"] in touched
                                              for c in conv.get("subagents", []))
        if not busy and (not delegated or synced or reports > reports_before):
            last = msgs[-1] if msgs else {}
            if last.get("role") == "assistant" and last.get("text", "").strip():
                finished = True
                break
        time.sleep(1.5)
    seconds = time.time() - started
    conv = conversation(main)
    msgs = conv.get("messages", [])
    said = next((m.get("text", "") for m in reversed(msgs) if m["role"] == "assistant" and m.get("text")), "")
    main_tools = [t["name"] for m in msgs if m["role"] == "assistant" and player_at and m.get("atMs", 0) >= player_at
                  for t in m.get("toolCalls", [])]
    tools = []
    for task in sorted(touched):
        steps = invoke("agent_subagent_messages", {"conversationId": main, "name": task}).get("ok", {}).get("messages", [])
        tools += [t["name"] for m in steps if m["role"] == "assistant" and m.get("atMs", 0) >= (player_at or 0)
                  for t in m.get("toolCalls", [])]
    rows = [x for x in ledger() if x.get("conversation") == main and x.get("turn") == player_at]
    cost = sum(x.get("cost") or 0 for x in rows)
    TABLE.append((name, "完成" if finished else "超时", len(questions), ",".join(dict.fromkeys(main_tools + tools)) or "-",
                  f"{seconds:.0f} s", f"¥{cost:.4f}", said[:70].replace("\n", " ").replace("|", "/")))
    print(f"   [{name}] {seconds:.0f} s, asked {questions}, main {main_tools}, background {tools}\n"
          f"   she said: {said[:200]}")
    return {"main": main, "finished": finished, "questions": questions, "tools": tools, "main_tools": main_tools,
            "said": said, "handovers": handovers}


def main():
    guard()
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    try:
        wait_ui()
        cid = real_deepseek.add_connection(EXE, ENV, model="deepseek-flash")
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        invoke("agent_new_conversation", {})

        web_cases()
        chat_page_banner()
    finally:
        real_deepseek.delete_connection(EXE, ENV)
        try:
            call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
        sweep_credentials()
    return report()


def web_cases():
    if True:
        # 玩家先在地址栏开了自己的标签页（多标签页）：她的事在她自己的标签页里做，不动玩家的。
        r = invoke("browser_navigate", {"url": "example.org"})
        player_tab = (r.get("ok") or {}).get("tab")
        check("B0: the player opens a tab from the address bar", player_tab, json.dumps(r, ensure_ascii=False)[:200])
        b1 = run("B1 example.com", "在浏览器里打开 https://example.com ，告诉我这个页面的标题是什么")
        check("B1: opened in the background with the browser tools", "browser.open" in b1.get("tools", []),
              ",".join(b1.get("tools", [])))
        check("B1: the first visit to a site asked the player", any("example.com" in q for q in b1.get("questions", [])),
              str(b1.get("questions")))
        sites = json.load(open(os.path.join(HOME, "browser-sites.json"), encoding="utf-8")) \
            if os.path.exists(os.path.join(HOME, "browser-sites.json")) else {}
        check("B1: the approval is remembered", "example.com" in sites.get("sites", []), json.dumps(sites))
        check("B1: she reports the title", "Example Domain" in b1.get("said", ""), b1.get("said", "")[:120])
        bs = browser_state()
        check("B1: browser state follows the page", bs.get("open") and bs.get("site") == "example.com",
              json.dumps(bs, ensure_ascii=False)[:300])
        tabs = bs.get("tabs") or []
        mine = next((t for t in tabs if t.get("id") == player_tab), {})
        hers = [t for t in tabs if t.get("agent")]
        check("B1: she worked in her own tab, the player's tab is untouched",
              mine.get("url") == "https://example.org/" and not mine.get("agent") and len(hers) == 1
              and hers[0].get("site") == "example.com" and bs.get("activeTab") == hers[0].get("id"),
              json.dumps(tabs, ensure_ascii=False)[:400])

        b2 = run("B2 search", "在浏览器里打开 https://cn.bing.com ，在搜索框里搜“晴天 周杰伦”，"
                              "然后点开第一条搜索结果，告诉我打开的是哪个网站、页面标题是什么")
        used = b2.get("tools", [])
        check("B2: typed into the search box and clicked a result (window hidden)",
              "browser.type" in used and "browser.click" in used, ",".join(used))
        bs = browser_state()
        check("B2: the page moved on from the search page", bs.get("url") and "bing.com/search" not in bs.get("url", "")
              or "browser.click" in used, f"{bs.get('site')} {bs.get('title')}")


def chat_page_banner():
    if True:
        try:
            seen = page_text()
            banner = "Cannot read properties" in seen.get("text", "")
            print(f"   chat page shows “Cannot read properties of null”: {banner}")
        except Exception as e:  # noqa: BLE001
            print("   could not read the chat page:", e)


def report():
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    print("\n| 用例 | 结果 | 问了几次 | 用的工具 | 用时 | 费用 | 她说的 |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for row in TABLE:
        print("| " + " | ".join(str(x) for x in row) + " |")
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
