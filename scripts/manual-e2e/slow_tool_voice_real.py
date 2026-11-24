"""MCP plan phase 1.5 / ADR 029 §1 by voice: a tool that has not come back after 3 seconds moves to the background.

Same setup as background_voice_real.py (whose helpers it reuses): real desktop app with `--background`, isolated
KOTONE_HOME=target/slow-tool-voice/home, the player's real DeepSeek key in memory only (chat, speech recognition and
speech synthesis all DeepSeek), the chat page's call button clicked through WebView2's DevTools port, the player's
words are SAPI sentences played into "CABLE Input" (the instance listens on "CABLE Output"). No OS input. Her speech
volume comes from KOTONE_TEST_SPEECH_VOLUME (default 8; 0 = silent). The app runs with
KOTONE_TEST_SLOW_TOOL=web.search:5000 (a debug-build test hook): web.search waits 5 s before searching.

S1. call → “帮我搜一下英雄联盟现在是哪个版本”: web.search is still running after 3 s, so it moves to the background — the
    stored tool message carries the `background: true` mark (Kotone's flag, not the result text), the registry
    has a task for it (tool = web.search), Kotone says the pre-made
    “交给我，好了告诉你”, the turn ends and she listens on. About 2 s later the search finishes, the result is handed
    back once, and she says it by herself (speak, no click).
Prints: utterance end → first sound, tool start → moved, moved → handed back, handed back → she speaks.
Stops before starting — and before every sentence — if a game is in front.
usage: python -X utf8 -I scripts/manual-e2e/slow_tool_voice_real.py
"""
import json
import os
import re
import shutil
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402
import voice_turn_real as v  # noqa: E402
import background_voice_real as bv  # noqa: E402  (its import points v at its own folder; set ours below)

REPO = os.path.dirname(os.path.dirname(HERE))
v.WORK = os.path.join(REPO, "target", "slow-tool-voice")
v.HOME = os.path.join(v.WORK, "home")
v.PORT = 9339
v.ENV = dict(os.environ, KOTONE_HOME=v.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(v.WORK, "webview"),
             WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={v.PORT}",
             KOTONE_TEST_SLOW_TOOL="web.search:5000")
v.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)


def main():
    v.guard()
    shutil.rmtree(v.WORK, ignore_errors=True)
    os.makedirs(v.HOME)
    with open(os.path.join(v.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": v.VOLUME,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": v.NO_INPUT}}}, f)
    app = v.start_app()
    try:
        cid = real_deepseek.add_connection(v.EXE, v.ENV, model="deepseek-flash")
        v.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        v.call("models.activate", {"category": "asr", "engineId": "deepseek-asr", "modelId": "deepseek-asr"})
        cable = next(d["id"] for d in v.call("audio.devices")["devices"] if d["id"].startswith("CABLE Output"))
        v.call("settings.update", {"patch": {"audioDeviceId": cable}})
        v.call("runtime.start")
        v.ui('await page.reload(); await page.waitForTimeout(2000);'
             'await page.getByTestId("sidebar-new-chat").evaluate((el) => el.click()); await page.waitForTimeout(1000);')
        ready = v.wait_log(0, r"fillers: \d+/\d+ ready in \d+ ms", 60)
        print(ready.group(0) if ready else "fillers: not ready")

        # S1. 搜索故意慢 5 秒：3 秒时转到后台、Kotone 说“交给我”、这一轮结束；做完自己开口说结果
        mark, timing = v.voice_turn("s1", "帮我搜一下英雄联盟现在是哪个版本", click=True, wait_follow_up=False,
                                    timeout=60)
        print("   timing:", timing.group(0) if timing else "-")
        conv = v.main_conversation()
        main = conv.get("id")
        player = next((m for m in reversed(conv["messages"]) if m["role"] == "user"
                       and m.get("source") not in ("agent-report", "auto-card")), {})
        turn = bv.after(conv["messages"], player.get("atMs", 0))
        searches = [m for m in turn if m["role"] == "tool" and m.get("name") == "web.search"]
        # 看存下来的标记（Message::Tool::background），不看结果内容（审查 2026-11-25）
        moved = [m for m in searches if m.get("background") is True]
        v.check("S1: web.search moved to the background after 3 s (tool message marked background)", moved,
                (searches[0].get("content") or "")[:160] if searches else "no web.search in this turn")
        hook = v.line_time(mark, r"test hook: web\.search waits")
        moved_at = v.line_time(mark, r"web\.search took over \d+ s, moved to the background")
        if hook and moved_at:
            v.check("S1: moved about 3 s after the tool started", 2.8 <= moved_at - hook <= 3.6,
                    f"{moved_at - hook:.2f} s")
        said_handoff = v.wait_log(mark, r"交给后台，先说", 5)
        v.check("S1: Kotone said “交给我，好了告诉你” (she did not speak in that step)",
                said_handoff or any(v.spoken(m) for m in turn if m["role"] == "assistant"),
                "handoff line" if said_handoff else "she spoke herself")
        got = bv.wait_report(main, player.get("atMs", 0), 120)
        # 这一轮在转走那一步就结束了（没再调用模型），通话一直开着（ADR 033：接着听）
        msgs = v.main_conversation()["messages"]
        report_at = next((i for i, m in enumerate(msgs) if m["role"] == "user" and m.get("source") == "agent-report"
                          and m["atMs"] >= player.get("atMs", 0)), len(msgs))
        before_report = [m for m in msgs[:report_at] if m.get("atMs", 0) >= player.get("atMs", 0)]
        v.check("S1: the turn ended at the move (no further model call)",
                before_report and before_report[-1]["role"] == "tool"
                and before_report[-1].get("background") is True,
                before_report[-1]["role"] if before_report else "-")
        handed_at = v.line_time(mark, r"background result\(s\) handed back")
        ended = v.line_time(mark, r"call: ended")
        v.check("S1: the call stayed open meanwhile (she kept listening)",
                ended is None or (handed_at is not None and ended > handed_at),
                "call still open at the hand-back" if not ended or (handed_at and ended > handed_at) else
                f"call ended {ended - (moved_at or ended):.1f} s after the move")
        v.check("S1: the search result came back by itself", got)
        if got:
            report, answer = got
            v.check("S1: she said the result out loud with speak (no click)", answer["spoken"],
                    f"she said “{answer['text'][:80]}”")
            spoke = bv.report_spoken(mark, 30)
            handed = v.line_time(mark, r"background result\(s\) handed back")
            finished = v.line_time(mark, r"background tool web-search \(web\.search\) finished")
            if moved_at and finished:
                print(f"   moved → search finished {finished - moved_at:.1f} s")
            if finished and handed:
                print(f"   finished → handed back {handed - finished:.1f} s")
            if spoke is not None:
                print(f"   handed back → she starts saying it {spoke:.1f} s later")
        # 聊天里那一行工具变成后台任务卡片（和 agent.start 派出去的同一种），标题照工具卡片的叫法
        try:
            card = v.ui("""
const card = page.locator('[data-testid=task-card][data-task-name="web-search"]').last();
await card.waitFor({ state: "attached", timeout: 10000 });
out.status = await card.getAttribute("data-task-status");
out.label = (await card.locator('[data-testid=task-card-label]').textContent() || "").trim();
""")
        except Exception as e:  # noqa: BLE001
            card = {"error": str(e)[:200]}
        v.check("S1: the chat shows it as a background task card, done", card.get("status") == "done",
                json.dumps(card, ensure_ascii=False))
        tasks = [a for a in bv.registry(main) if a.get("tool") == "web.search"]
        v.check("S1: one background task card for web.search, done and reported",
                len(tasks) == 1 and tasks[0].get("status") == "done" and tasks[0].get("reported"),
                json.dumps(tasks, ensure_ascii=False)[:200])
        time.sleep(5)
        reports = [m for m in bv.after(v.main_conversation()["messages"], player.get("atMs", 0))
                   if m["role"] == "user" and m.get("source") == "agent-report"]
        v.check("S1: handed back exactly once", len(reports) == 1, f"{len(reports)} report message(s)")
        if timing:
            print("\n| 回合 | 说完→识别 | 识别→首字 | 首字→出声 | 说完→出声 | 先出声的 | 填充话 | 开始合成→出声 |")
            print("| --- | --- | --- | --- | --- | --- | --- | --- |")
            print("| S1 搜版本（搜索慢 5 秒） | " + " | ".join((g or "-") for g in timing.groups()) + " |")
        fillers = re.findall(r"filler=(\S+)", v.log_since(mark))
        print("   waiting phrases:", fillers or "none")
    finally:
        real_deepseek.delete_connection(v.EXE, v.ENV)
        v.stop_app(app)
    failed = [name for name, ok in v.RESULTS if not ok]
    print(f"\n{len(v.RESULTS) - len(failed)}/{len(v.RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
