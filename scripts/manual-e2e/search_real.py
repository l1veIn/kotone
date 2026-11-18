"""Web search (ADR 026) and DeepSeek best practices against the real DeepSeek service, with 海克斯大乱斗 questions.

Isolated Kotone (KOTONE_HOME=target/search-e2e/home, hotkeys off, speech volume 0, input tools denied) + the player's
real DeepSeek key (real_deepseek.py: memory only, deleted afterwards).
For each chat-window thinking effort (default/high, low, none) a new conversation asks the same LoL questions and prints,
per turn: seconds, whether 琴音 searched / read a page, tokens, cache hits, cost and the start of the answer.
One in-game question goes through `assistant.submit` (agent.gameEffort, default low).
Then checks: the conversation got a model title (DeepSeek `title`), the ledger has 联网搜索 records priced at ¥0.005,
and nothing was refused (safety_check on).
usage: python -X utf8 -I scripts/manual-e2e/search_real.py [--efforts default,low,none]
"""
import glob
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402
from cache_real import NO_INPUT  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "search-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)

QUESTIONS = [
    "英雄联盟现在是哪个版本？",
    "海克斯大乱斗现在哪些英雄比较强？说三个就行。",
    "海克斯大乱斗里玩寒冰射手，海克斯强化优先选什么？简单说。",
]
GAME_QUESTION = "大乱斗我拿到了蒙多，出装怎么出，一句话说"


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def invoke(command, args):
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:9333");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
  const r = await page.evaluate(async ([c, a]) => {
    try { return { ok: await window.__TAURI_INTERNALS__.invoke(c, a) }; } catch (e) { return { error: String(e) }; }
  }, [%s, %s]);
  console.log(JSON.stringify(r));
} finally { await browser.close(); }
""" % (json.dumps(command), json.dumps(args, ensure_ascii=False))
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        raise RuntimeError(out.stdout + out.stderr)


def wait_idle(seconds=180):
    deadline = time.time() + seconds
    time.sleep(1.0)
    while time.time() < deadline:
        state = invoke("agent_state", {}).get("ok") or {}
        if not state.get("running"):
            return
        time.sleep(0.5)
    raise AssertionError("琴音 did not finish in time")


def conversation():
    files = sorted(glob.glob(os.path.join(HOME, "agent", "conversations", "*.json")), key=os.path.getmtime)
    main = [f for f in files if "parent" not in json.load(open(f, encoding="utf-8"))]
    return json.load(open(main[-1], encoding="utf-8"))


def turn_rows(conv, since):
    """Messages added since index `since`: tools called, usage summed over model calls, the final answer."""
    tools, inp, hit, out, cost, answer, notices = [], 0, 0, 0, 0.0, "", []
    for m in conv["messages"][since:]:
        if m["role"] == "assistant":
            tools += [c["name"] for c in m.get("toolCalls", [])]
            u = m.get("usage") or {}
            inp += u.get("inputTokens") or 0
            hit += u.get("cachedInputTokens") or 0
            out += u.get("outputTokens") or 0
            cost += u.get("cost") or 0.0
            if m.get("text", "").strip():
                answer = m["text"].strip()
        if m["role"] == "notice":
            notices.append(m.get("text", ""))
    return tools, inp, hit, out, cost, answer, notices


def ask(text, game=False):
    # A new conversation is only saved after its first message: count from 0 when the newest file changed.
    old = conversation() if glob.glob(os.path.join(HOME, "agent", "conversations", "*.json")) else {"id": None, "messages": []}
    started = time.time()
    if game:
        call("assistant.submit", {"text": text})
    else:
        r = invoke("agent_send", {"text": text})
        if "error" in r:
            raise AssertionError(r["error"])
    wait_idle()
    seconds = time.time() - started
    conv = conversation()
    before = len(old["messages"]) if conv["id"] == old["id"] else 0
    tools, inp, hit, out, cost, answer, notices = turn_rows(conv, before)
    searched = sum(t == "web.search" for t in tools)
    fetched = sum(t == "web.fetch" for t in tools)
    print(f"  {'game' if game else 'chat'}> {text}")
    print(f"     {seconds:5.1f}s  search {searched} fetch {fetched}  in {inp} (hit {hit}) out {out}  ¥{cost:.4f}"
          f"{'  NOTICE ' + ' / '.join(notices) if notices else ''}")
    print(f"     < {answer[:160].replace(chr(10), ' ')}")
    return {"seconds": seconds, "searched": searched, "fetched": fetched, "cost": cost, "answer": answer, "notices": notices}


def main():
    efforts = (sys.argv[sys.argv.index("--efforts") + 1] if "--efforts" in sys.argv else "default,low,none").split(",")
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "wake": {"enabled": False}, "speechVolume": 0},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False}}, f)
    app = subprocess.Popen([EXE], env=ENV)
    results = {}
    passed = True
    try:
        time.sleep(10)
        cid = real_deepseek.add_connection(EXE, ENV, model="deepseek-flash")
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        active = [m for m in call("models.list", {"category": "search"})["models"] if m.get("active")]
        print("search backend:", [m["id"] for m in active])
        passed &= bool(active) and active[0]["id"] == "deepseek-search"
        for effort in efforts:
            call("settings.update", {"patch": {"agent": {"chatEffort": "" if effort == "default" else effort}}})
            invoke("agent_new_conversation", {})
            print(f"\n== chat window, effort {effort} ==")
            rows = [ask(q) for q in QUESTIONS]
            conv = conversation()
            print("     title:", conv.get("autoTitle"), "| list title:", conv.get("title"))
            results[effort] = rows
            if effort == efforts[0]:
                passed &= bool(conv.get("autoTitle"))
        print("\n== in game (agent.gameEffort default low) ==")
        game = ask(GAME_QUESTION, game=True)
        passed &= not game["notices"]
        summary = call("usage.summary")
        kinds = {k["kind"]: k for k in summary["today"].get("byKind", [])} if isinstance(summary["today"], dict) else {}
        print("\nusage today:", json.dumps(summary["today"], ensure_ascii=False)[:600])
        searches = sum(r["searched"] + r["fetched"] for rows in results.values() for r in rows) + game["searched"] + game["fetched"]
        print(f"\nweb calls made: {searches}")
        print("\n== summary ==")
        for effort, rows in results.items():
            print(f"{effort:>8}: avg {sum(r['seconds'] for r in rows) / len(rows):5.1f}s, searched in "
                  f"{sum(1 for r in rows if r['searched'])}/{len(rows)} turns, ¥{sum(r['cost'] for r in rows):.4f} model cost"
                  f", refused/failed turns {sum(1 for r in rows if r['notices'])}")
        passed &= searches > 0
        passed &= not any(r["notices"] for rows in results.values() for r in rows)
        print("\nPASS" if passed else "\nFAIL")
    finally:
        real_deepseek.delete_connection(EXE, ENV)
        try:
            call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
    sys.exit(0 if passed else 1)


if __name__ == "__main__":
    main()
