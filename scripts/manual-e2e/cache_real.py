"""Prefix-cache hit rate, cost ledger and compaction against the real DeepSeek service.

Isolated Kotone (KOTONE_HOME=target/cache-e2e/home, hotkeys off, speech volume 0) + the player's real DeepSeek key
(real_deepseek.py: memory only, deleted afterwards).
1. Chat window: several turns in one conversation. In-game entry (`assistant.submit`, same conversation): several calls.
   Prints input / cache-hit tokens, hit rate and cost of every model request (from the stored conversation).
2. Compaction: lowers agent.compactAtTokens, tells 琴音 a fact, chats until a checkpoint appears, then asks for the fact.
3. usage.summary (the ledger).
usage: python -X utf8 -I scripts/manual-e2e/cache_real.py [--model deepseek-flash] [--skip-compaction]
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

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "cache-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)

CHAT = [
    "你好，我叫阿杰，最近在玩英雄联盟，主玩打野。",
    "我装了哪些游戏？",
    "打野前期怎么规划路线比较好？简单说两句。",
    "帮我记一下：我习惯先刷红再去拿河蟹。",
    "我刚才说我习惯先刷什么？",
]
GAME = [
    "琴音，我这把玩的是盲僧",
    "提醒我一下五分钟以后买真眼",
    "刚刚让你提醒我什么来着",
    "好，没事了，谢谢",
]


# 真机测试不让琴音按键：会往前台窗口打字、点鼠标的工具一律禁止（2026-11-18 voice_chat_real 里她选了打字，
# 字和回车进了桌面）。
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny"}


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


def wait_idle(seconds=120):
    deadline = time.time() + seconds
    time.sleep(1.5)
    while time.time() < deadline:
        state = invoke("agent_state", {}).get("ok") or {}
        if not state.get("running"):
            return
        time.sleep(1)
    raise AssertionError("琴音 did not finish in time")


def conversation():
    files = sorted(glob.glob(os.path.join(HOME, "agent", "conversations", "*.json")), key=os.path.getmtime)
    main = [f for f in files if "parent" not in json.load(open(f, encoding="utf-8"))]
    return json.load(open(main[-1], encoding="utf-8"))


def requests_of(conv):
    rows = []
    for m in conv["messages"]:
        usage = m.get("usage")
        if not usage:
            continue
        kind = "compaction" if m["role"] == "notice" else "chat"
        inp, hit = usage.get("inputTokens") or 0, usage.get("cachedInputTokens") or 0
        rows.append({"kind": kind, "input": inp, "hit": hit, "out": usage.get("outputTokens"),
                     "rate": (hit / inp) if inp else None, "cost": usage.get("cost"), "cur": usage.get("currency")})
    return rows


def print_rows(rows, title):
    print(f"\n== {title} ==")
    for i, r in enumerate(rows, 1):
        rate = f"{r['rate'] * 100:5.1f}%" if r["rate"] is not None else "  —  "
        cost = f"{r['cost']:.5f} {r['cur']}" if r["cost"] is not None else "—"
        print(f"{i:2d} {r['kind']:<10} input {r['input']:>6}  hit {r['hit']:>6}  {rate}  out {r['out']!s:>5}  {cost}")
    inp = sum(r["input"] for r in rows)
    hit = sum(r["hit"] for r in rows)
    later = rows[1:]
    later_rate = (sum(r["hit"] for r in later) / max(1, sum(r["input"] for r in later))) if later else 0
    print(f"total input {inp}, hit {hit} ({hit / max(1, inp) * 100:.1f}%); excluding the first request {later_rate * 100:.1f}%")


def answers(conv, n=1):
    texts = [m["text"] for m in conv["messages"] if m["role"] == "assistant" and m.get("text", "").strip()]
    return texts[-n:]


def main():
    model = sys.argv[sys.argv.index("--model") + 1] if "--model" in sys.argv else "deepseek-flash"
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "wake": {"enabled": False}, "speechVolume": 0},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False}}, f)
    app = subprocess.Popen([EXE], env=ENV)
    try:
        time.sleep(10)
        cid = real_deepseek.add_connection(EXE, ENV, model=model)
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        invoke("agent_new_conversation", {})
        for text in CHAT:
            r = invoke("agent_send", {"text": text})
            assert "ok" in r, r
            wait_idle()
            print("chat>", text, "\n   <", answers(conversation())[0][:120].replace("\n", " "))
        for text in GAME:
            call("assistant.submit", {"text": text})
            wait_idle()
            print("game>", text, "\n   <", answers(conversation())[0][:120].replace("\n", " "))
        conv = conversation()
        print_rows(requests_of(conv), f"{model}: {len(CHAT)} chat turns + {len(GAME)} in-game turns")

        if "--skip-compaction" not in sys.argv:
            # 先攒一段较长的历史（一个要记住的事实 + 几个详细回答），再把阈值调到刚好超过，触发一次整理。
            for text in ["我的召唤师名字叫“星河下的猫”，记住这个。",
                         "给我讲讲打野怎么反蹲，分几点说，详细一点。",
                         "再讲讲怎么控龙，也详细一点。",
                         "还有怎么帮下路，详细一点。"]:
                invoke("agent_send", {"text": text})
                wait_idle(180)
            used = requests_of(conversation())[-1]["input"]
            call("settings.update", {"patch": {"agent": {"compactAtTokens": used + 300}}})
            invoke("agent_send", {"text": "好的，谢谢。"})
            wait_idle(180)
            invoke("agent_send", {"text": "我的召唤师名字叫什么？我打野习惯先刷哪个buff？"})
            wait_idle()
            invoke("agent_send", {"text": "再说一遍控龙最要紧的一点。"})
            wait_idle()
            conv = conversation()
            checkpoints = [m for m in conv["messages"] if m.get("role") == "notice" and m.get("compaction")]
            print_rows(requests_of(conv), "after compaction")
            print("\ncheckpoint found:", bool(checkpoints))
            if checkpoints:
                print("summary (first 800 chars):\n", checkpoints[-1]["compaction"][:800])
            final = answers(conv, 2)[0]
            print("\nanswer about the facts:", final)
            print("remembers the name:", "星河下的猫" in final, "| remembers the red buff:", "红" in final)
        print("\nusage.summary today:", json.dumps(call("usage.summary")["today"], ensure_ascii=False))
    finally:
        real_deepseek.delete_connection(EXE, ENV)
        try:
            call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()


if __name__ == "__main__":
    main()
