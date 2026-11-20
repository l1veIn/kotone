"""ADR 029 phase 2 real-machine check: a typed request the main conversation hands to the background, which
reports back on its own.

Real desktop app (debug build with the web UI embedded, `--background`: no window, no focus change), isolated
KOTONE_HOME=target/background-real/home, the player's real DeepSeek key in memory only (real_deepseek.py).
No OS input: messages go in through WebView2's DevTools port (Playwright connectOverCDP → `agent_send`, i.e. typed
in the chat page). Input tools are denied; speech volume 0. Stops before starting if a game is in front.

Checks:
0. typed “英雄联盟现在是哪个版本？” → quick things she does herself (web.search is in her instant set), no
   `agent.start`;
1. typed a multi-step task (write and save an automation, don't run it) → the main conversation calls `agent.start`,
   which returns `running` within the ~4 s window, and her turn ends with a short line;
2. the background task runs on (card status in `agent_conversation().subagents`), finishes, and its result comes
   back as an `agent-report` message that wakes the main conversation to write one line;
3. the background record is hidden from the conversation list; `agent_subagent_messages` returns its steps;
4. the background model / search charges are billed to the main conversation's dispatching turn (usage.jsonl).

Running commands and files in the background (ADR 030), same conversation:
E1. “在你的文件夹里写一个 notes.txt，内容是今天的日期” → done without asking (writing in Kotone's own files);
E2. “搜一下 PowerShell 怎么列出文件夹大小，然后照着在你的文件夹里跑一下” → after reading the web, the command
    waits for the player (card `waiting` + question); answered “允许” through `agent_subagent_answer`; done;
E3. “把你文件夹里的 notes.txt 删掉” → deleting asks; answered “拒绝”; the file is still there and she says so.
`--exec-only` skips 0–4.
usage: python -X utf8 -I scripts/manual-e2e/background_task_real.py [--exec-only]
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
WORK = os.path.join(REPO, "target", "background-real")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
PORT = 9336
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}
RESULTS = []
QUICK = "英雄联盟现在是哪个版本？一句话"
REQUEST = "帮我写一个自动化：每隔 5 秒按一下 Q 键，按 F8 停下。写好直接保存，名字叫“自动按Q”，先不要运行。"


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


def conversation(cid):
    return invoke("agent_conversation", {"id": cid}).get("ok") or {}


def wait_until(predicate, timeout, step=1.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return None


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
        # 0. 快的事她自己做（搜一下就答），不交给后台
        quick_at = time.time()
        r = invoke("agent_send", {"text": QUICK})
        check("quick question accepted", "ok" in r, json.dumps(r, ensure_ascii=False)[:200])
        wait_until(lambda: not state().get("running"), 60, 0.5)
        quick_s = time.time() - quick_at
        qmsgs = conversation(r["ok"]["conversationId"]).get("messages", [])
        qtools = [c["name"] for m in qmsgs if m["role"] == "assistant" for c in m.get("toolCalls", [])]
        qreply = next((m.get("text", "") for m in reversed(qmsgs) if m["role"] == "assistant" and m.get("text")), "")
        check("a quick question is answered directly (no agent.start)", "agent.start" not in qtools and qreply,
              f"{quick_s:.1f} s, tools {qtools}: {qreply[:80]}")
        if "--exec-only" in sys.argv:
            exec_cases()
            return finish()
        sent_at = time.time()
        r = invoke("agent_send", {"text": REQUEST})
        check("typed request accepted", "ok" in r, json.dumps(r, ensure_ascii=False)[:200])
        main_id = r["ok"]["conversationId"]

        # 1. 主会话派活，4 秒内返回 running，这一轮很快结束
        idle = wait_until(lambda: not state().get("running"), 60, 0.5)
        main_done = time.time() - sent_at
        conv = conversation(main_id)
        msgs = conv.get("messages", [])
        starts = [(i, c) for i, m in enumerate(msgs) if m["role"] == "assistant" for c in m.get("toolCalls", [])
                  if c["name"] == "agent.start"]
        check("main conversation handed the work to the background (agent.start)", starts,
              ",".join(c["name"] for m in msgs if m["role"] == "assistant" for c in m.get("toolCalls", [])))
        if starts:
            i, call_ = starts[0]
            result = next((m for m in msgs[i + 1:] if m["role"] == "tool" and m["callId"] == call_["id"]), {})
            waited = (result.get("atMs", 0) - msgs[i]["atMs"]) / 1000
            check("agent.start came back within the ~4 s window", 0 <= waited < 6, f"{waited:.1f} s")
            check("agent.start said it is still running", '"running"' in result.get("content", ""),
                  result.get("content", "")[:200])
            print("   task:", json.loads(call_["arguments"]).get("label"), "/", json.loads(call_["arguments"]).get("name"))
        reply = next((m.get("text", "") for m in reversed(msgs) if m["role"] == "assistant" and m.get("text")), "")
        check("her turn ended quickly with a short line", idle and main_done < 30 and 0 < len(reply) < 120,
              f"{main_done:.1f} s: {reply[:80]}")
        cards = conv.get("subagents", [])
        check("the card is registered on the main conversation", cards, json.dumps(cards, ensure_ascii=False)[:300])
        listed = [c["id"] for c in state().get("conversations", [])]
        check("the background record is not in the conversation list",
              cards and cards[0]["conversationId"] not in listed and main_id in listed)

        # 2. 后台做完，结果交回主会话并唤醒她写一句
        def reported():
            c = conversation(main_id)
            ms = c.get("messages", [])
            idx = [i for i, m in enumerate(ms) if m["role"] == "user" and m.get("source") == "agent-report"]
            if not idx or state().get("running"):
                return None
            after = [m for m in ms[idx[-1] + 1:] if m["role"] == "assistant" and m.get("text", "").strip()]
            return (c, ms[idx[-1]], after) if after else None
        got = wait_until(reported, 300, 2)
        total = time.time() - sent_at
        check("the result came back by itself and she wrote a line", got, f"{total:.0f} s after sending")
        if got:
            c, report, after = got
            card = next((a for a in c.get("subagents", []) if a["name"] in report["text"]), {})
            check("the card says done, steps counted, result handed back",
                  card.get("status") == "done" and card.get("steps", 0) > 0 and card.get("reported"),
                  json.dumps({k: card.get(k) for k in ("status", "steps", "reported")}))
            print("   summary:", (card.get("summary") or "")[:300].replace("\n", " "))
            print("   she said:", after[-1]["text"][:200].replace("\n", " "))
            steps = invoke("agent_subagent_messages", {"conversationId": main_id, "name": card.get("name", "")})
            tools = [t["name"] for m in steps.get("ok", {}).get("messages", []) if m["role"] == "assistant"
                     for t in m.get("toolCalls", [])]
            check("the card can show the background steps", tools, ",".join(tools))
            player_turn = next(m["atMs"] for m in c["messages"] if m["role"] == "user" and m.get("text") == REQUEST)
            rows = [r for r in ledger() if r.get("conversation") == main_id and r.get("turn") == player_turn]
            kinds = sorted({r.get("kind") for r in rows})
            check("background charges are billed to the dispatching turn", any(r.get("kind") == "chat" for r in rows),
                  f"{len(rows)} rows: {kinds}")
            child = card.get("conversationId")
            stray = [r for r in ledger() if r.get("conversation") == child]
            check("nothing is billed to the hidden record itself", not stray, f"{len(stray)} rows")
        exec_cases()
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
    return finish()


def finish():
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    if TABLE:
        print("\n| 用例 | 结果 | 问了几次 | 后台用的工具 | 用时 | 费用 | 她说的 |")
        print("| --- | --- | --- | --- | --- | --- | --- |")
        for row in TABLE:
            print("| " + " | ".join(str(x) for x in row) + " |")
    return not failed


# ---------- 后台执行命令、读写文件（ADR 030） ----------

TABLE = []
NOTES = os.path.join(HOME, "agent", "files", "notes.txt")


def run_case(name, text, answer=None, timeout=300):
    """Type `text` and follow it to the end: answer the background's confirmation(s) with `answer`
    ("allow-once" / "deny"; None = must not ask). Returns a summary dict."""
    guard()
    started = time.time()
    r = invoke("agent_send", {"text": text})
    if not check(f"{name}: typed request accepted", "ok" in r, json.dumps(r, ensure_ascii=False)[:200]):
        return {}
    main = r["ok"]["conversationId"]
    before = conversation(main)
    reports_before = sum(1 for m in before.get("messages", []) if m.get("source") == "agent-report")
    player_at = None
    questions, answered, touched = [], set(), set()
    deadline = time.time() + timeout
    finished = False
    while time.time() < deadline:
        conv = conversation(main)
        msgs = conv.get("messages", [])
        if player_at is None:
            player_at = next((m["atMs"] for m in reversed(msgs) if m["role"] == "user" and m.get("text") == text), None)
        new = [m for m in msgs if player_at and m.get("atMs", 0) >= player_at]
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
                    if answer:
                        invoke("agent_subagent_answer", {"conversationId": main, "name": c["name"], "answer": answer})
                    else:
                        invoke("agent_subagent_answer", {"conversationId": main, "name": c["name"], "answer": "deny"})
        delegated = any(t["name"] in ("agent.start", "agent.followup") for m in new if m["role"] == "assistant"
                        for t in m.get("toolCalls", []))
        reports = sum(1 for m in msgs if m.get("source") == "agent-report")
        # 4 秒内做完的：agent.start / followup 直接返回结果，不另外交回
        synced = any(m["role"] == "tool" and m.get("content", "").find('"status":"done"') >= 0
                     or m["role"] == "tool" and m.get("content", "").find('"status":"failed"') >= 0
                     for m in new)
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
    tools = []
    for task in sorted(touched):
        steps = invoke("agent_subagent_messages", {"conversationId": main, "name": task}).get("ok", {}).get("messages", [])
        tools += [t["name"] for m in steps if m["role"] == "assistant" and m.get("atMs", 0) >= (player_at or 0)
                  for t in m.get("toolCalls", [])]
    rows = [x for x in ledger() if x.get("conversation") == main and x.get("turn") == player_at]
    cost = sum(x.get("cost") or 0 for x in rows)
    cards = {c["name"]: c for c in conv.get("subagents", [])}
    via = "当场" if any(m["role"] == "tool" and '"status":"done"' in m.get("content", "") for m in msgs
                       if player_at and m.get("atMs", 0) >= player_at) else "交回"
    out = {"main": main, "finished": finished, "via": via, "questions": questions, "tools": tools, "said": said,
           "seconds": seconds, "cost": cost, "cards": [cards[t] for t in touched if t in cards]}
    TABLE.append((name, ("完成·" + via) if finished else "超时", len(questions), ",".join(dict.fromkeys(tools)) or "-",
                  f"{seconds:.0f} s", f"¥{cost:.4f}", said[:60].replace("\n", " ").replace("|", "/")))
    print(f"   [{name}] {seconds:.0f} s, asked {len(questions)}, tools {tools}, ¥{cost:.4f}\n   she said: {said[:160]}")
    return out


def exec_cases():
    print("\n== ADR 030: commands and files in the background ==")
    today = time.strftime("%Y-%m-%d")
    e1 = run_case("E1 写 notes.txt", "在你的文件夹里写一个 notes.txt，内容是今天的日期")
    check("E1: finished and reported back", e1.get("finished"))
    check("E1: no confirmation needed (Kotone's own files)", e1 and not e1["questions"], e1.get("questions"))
    check("E1: used the file / shell tools in the background",
          any(t in ("file.write", "shell.run") for t in e1.get("tools", [])), e1.get("tools"))
    content = open(NOTES, encoding="utf-8", errors="replace").read() if os.path.exists(NOTES) else ""
    month_day = f"{int(today[5:7])}月{int(today[8:10])}日"
    check("E1: notes.txt holds today's date", today in content or month_day in content or today.replace("-", "/") in content,
          repr(content[:60]))

    e2 = run_case("E2 搜完照着跑命令", "搜一下 PowerShell 怎么列出文件夹大小，然后照着在你的文件夹里跑一下",
                  answer="allow-once")
    check("E2: finished and reported back", e2.get("finished"))
    check("E2: asked before running the command (it read the web first)", e2.get("questions"), e2.get("questions"))
    check("E2: ran the command after “允许”", "shell.run" in e2.get("tools", []), e2.get("tools"))
    card = next(iter(e2.get("cards", [])), {})
    check("E2: the card remembers it read outside things", card.get("taint"), card.get("taint"))

    e3 = run_case("E3 删除被拒", "把你文件夹里的 notes.txt 删掉", answer="deny")
    check("E3: asked before deleting", any("删除" in q or "删" in q for q in e3.get("questions", [])), e3.get("questions"))
    check("E3: the file is still there after “拒绝”", os.path.exists(NOTES))
    check("E3: she says it was not deleted", any(w in e3.get("said", "") for w in ("没删", "没有删", "未删", "保留", "还在", "取消", "不删")),
          e3.get("said", "")[:120])


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
