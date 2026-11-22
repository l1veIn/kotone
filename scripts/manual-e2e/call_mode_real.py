"""ADR 033 real-machine check: calling her opens a call — always listening, barge-in, "退下", idle hang-up,
and confirmations answered by voice any time.

Same rig as voice_turn_real.py (reuses its helpers): the debug desktop app in `--background`, isolated
KOTONE_HOME=target/call-real/home, the player's real DeepSeek key in memory only, no OS input — the chat page's
call button is clicked through WebView2's DevTools port, the player's words are SAPI sentences played into
"CABLE Input" (the instance listens on "CABLE Output"). Her voice goes to the default speakers, quietly.
Echo cancellation is on: its reference is the default speakers' loopback; the CABLE microphone never hears her,
so this checks the pipeline (frames flow, recognition works through it), not how much echo it removes — that
needs a real microphone and speakers (docs/playtest-adr-033.md).

Steps:
1. click call → “你好，你是谁”; after her answer, without clicking → “那你都能帮我做什么”: heard (always listening).
2. a long answer; while she talks → “嗯”: she gets quieter, then back to full volume and goes on.
3. a long answer; while she talks → “等一下，现在几点了”: she stops (speak result “玩家打断了”), answers that.
4. web.search set to ask: she asks; stay silent past the window; then → “允许”: the search runs.
5. → “退下吧”: the call ends at once.
6. click call → “你好”; stay silent: the call ends after callIdleSeconds (20 s here).
7. click call → “你好”; click again: the call ends (呼叫键).
usage: python -X utf8 -I scripts/manual-e2e/call_mode_real.py
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
import voice_turn_real as vt  # noqa: E402

vt.WORK = os.path.join(vt.REPO, "target", "call-real")
vt.HOME = os.path.join(vt.WORK, "home")
vt.PORT = 9336
vt.ENV = dict(os.environ, KOTONE_HOME=vt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(vt.WORK, "webview"),
              WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={vt.PORT}")
vt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
IDLE = 20
check, wait_log, log_since, log_text = vt.check, vt.wait_log, vt.log_since, vt.log_text


def say(name, text, lead_s=0.3):
    """Play the player's sentence into the CABLE microphone (no click)."""
    vt.guard()
    wav = os.path.join(vt.WORK, f"{name}.wav")
    vt.speak_to_wav(text, wav)
    vt.play_into_cable(wav, lead_s)


def wait_quiet(mark, timeout=90):
    """Until her answer has been spoken (the call shows its countdown again)."""
    return wait_log(mark, r"speech stream \(deepseek\): .*Ok\(\(\)\)|speech stream \(deepseek\): .*Err", timeout)


def main():
    vt.guard()
    shutil.rmtree(vt.WORK, ignore_errors=True)
    os.makedirs(vt.HOME)
    rules = dict(vt.NO_INPUT)
    with open(os.path.join(vt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 8,
                                 "wake": {"enabled": False}, "callIdleSeconds": IDLE, "echoCancellation": True},
                   "agent": {"permissions": {"mode": "allow-all", "rules": rules}}}, f)
    app = vt.start_app()
    try:
        cid = real_deepseek.add_connection(vt.EXE, vt.ENV, model="deepseek-flash")
        vt.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        vt.call("models.activate", {"category": "asr", "engineId": "deepseek-asr", "modelId": "deepseek-asr"},
                timeout_ms=30_000)
        cable = next(d["id"] for d in vt.call("audio.devices")["devices"] if d["id"].startswith("CABLE Output"))
        vt.call("settings.update", {"patch": {"audioDeviceId": cable}})
        vt.call("runtime.start")
        vt.ui('await page.reload(); await page.waitForTimeout(2000);'
              'await page.getByTestId("sidebar-new-chat").evaluate((el) => el.click()); await page.waitForTimeout(1000);')
        wait_log(0, r"fillers: \d+/\d+ ready", 60)

        # 1. 接通；答完不按键接着说
        mark = len(log_text())
        vt.click_call()
        say("t1", "你好，你是谁", lead_s=0.8)
        check("1: the call connected", wait_log(mark, r"call: connected", 10))
        check("1: microphone opened with echo cancellation",
              wait_log(0, r"wake: listening \(wake=false.*echo-cancel=true\)", 10),
              (re.search(r"wake: listening.*", log_text()) or [""])[0])
        wait_quiet(mark)
        time.sleep(1.5)
        mark = len(log_text())
        say("t1b", "那你都能帮我做什么")
        check("1: heard the next sentence without pressing anything",
              wait_log(mark, r"call: player started speaking", 10))
        wait_quiet(mark)
        users = [m for m in vt.main_conversation()["messages"] if m["role"] == "user"]
        check("1: it became her next turn", users and "做什么" in users[-1].get("text", ""),
              users[-1].get("text") if users else "-")
        time.sleep(1.5)

        # 2. 她说话时“嗯”一声：压低再恢复，接着说
        mark = len(log_text())
        say("t2", "用两三句话详细说说英雄联盟里打野前期该怎么刷野")
        opened = wait_log(mark, r"speech stream: opening", 60)
        time.sleep(1.0)
        say("t2b", "嗯", lead_s=0.0)
        check("2: she got quieter when the player made a sound", wait_log(mark, r"voice reply: ducked", 10))
        restored = wait_log(mark, r"voice reply: back to full volume \((只是应了一声|没识别出字|只有一个字|没识别出话|是她自己的回声)\)", 15)
        check("2: “嗯” was not an interruption (back to full volume)", restored, restored.group(0) if restored else "")
        check("2: she was not stopped", not re.search(r"玩家开口打断了她", log_since(mark)))
        wait_quiet(mark)
        time.sleep(1.5)

        # 3. 她说话时开口：停下，回答新的
        mark = len(log_text())
        say("t3", "再用两三句话详细说说中路对线该注意什么")
        opened = wait_log(mark, r"speech stream: opening", 60)
        check("3: she started talking", opened)
        time.sleep(1.0)
        say("t3b", "等一下，现在几点了", lead_s=0.0)
        check("3: ducked when the player spoke", wait_log(mark, r"voice reply: ducked", 10))
        check("3: she stopped (barge-in)", wait_log(mark, r"玩家开口打断了她", 20))
        wait_log(mark, r"voice-timing: ", 60)
        wait_quiet(len(log_text()) - 2000 if len(log_text()) > 2000 else 0)
        time.sleep(2)
        messages = vt.main_conversation()["messages"]
        interrupted = [m for m in messages if m["role"] == "tool" and m.get("name") == "speak"
                       and "玩家打断了" in m.get("content", "")]
        check("3: the conversation records what she got to say", interrupted,
              interrupted[-1]["content"][:60] if interrupted else "-")
        users = [m for m in messages if m["role"] == "user"]
        check("3: the new sentence went to her", any("几点" in m.get("text", "") for m in users[-3:]),
              users[-1].get("text") if users else "-")
        time.sleep(1.5)

        # 4. 确认：问完那几秒不说话，之后说“允许”也算
        # 权限只能由玩家在设置页改（没有工具）：像设置页那样调命令
        mark = len(log_text())
        vt.ui("""
await page.evaluate(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const state = await invoke("agent_state");
  const permissions = { ...state.permissions, rules: { ...state.permissions.rules, "web.search": "ask" } };
  await invoke("agent_set_permissions", { permissions });
});
""")
        say("t4", "帮我搜一下今天英雄联盟有什么新闻")
        asked = wait_log(mark, r"voice ask: listening for \d+s", 60)
        check("4: she asked by voice", asked)
        no_answer = wait_log(mark, r"voice ask: no answer", 20)
        check("4: nobody answered in the window", no_answer)
        time.sleep(4)  # “没听清。说‘可以’或‘不要’就行。”
        say("t4b", "允许")
        answered = wait_log(mark, r"玩家用嘴回答了还在等的确认（允许）", 20)
        check("4: “允许” later still answered the confirmation", answered)
        wait_log(mark, r"voice-timing: ", 60)
        time.sleep(6)
        calls = [c["name"] for m in vt.main_conversation()["messages"] if m["role"] == "assistant"
                 for c in m.get("toolCalls", []) or []]
        check("4: the search ran", "web.search" in calls[-6:], ",".join(calls[-6:]))
        wait_quiet(mark)
        time.sleep(1.5)

        # 5. 退下：马上挂断
        mark = len(log_text())
        say("t5", "退下吧")
        check("5: “退下吧” ended the call", wait_log(mark, r"call: ended \(玩家说了退下\)", 20))
        time.sleep(2)
        mark = len(log_text())
        say("t5b", "你还在听吗")
        time.sleep(4)
        check("5: after hanging up she no longer listens", not re.search(r"call: player started speaking", log_since(mark)))

        # 6. 一直没人说话：自动挂断
        mark = len(log_text())
        vt.click_call()
        say("t6", "你好", lead_s=0.8)
        check("6: connected again", wait_log(mark, r"call: connected", 10))
        ended = wait_log(mark, r"call: ended \(没人说话\)", IDLE + 60)
        check(f"6: ended after {IDLE}s of silence", ended)

        # 7. 通话中再点一下：挂断
        mark = len(log_text())
        vt.click_call()
        say("t7", "你好", lead_s=0.8)
        check("7: connected", wait_log(mark, r"call: connected", 10))
        wait_quiet(mark)
        time.sleep(1)
        vt.click_call()
        check("7: clicking call again hung up", wait_log(mark, r"call: ended \(呼叫键\)", 10))
    finally:
        try:
            real_deepseek.delete_connection(vt.EXE, vt.ENV)
        except Exception as e:  # noqa: BLE001
            print("delete connection:", e)
        vt.stop_app(app)
    stats = re.findall(r"echo cancel: .*", log_text())
    for line in stats[-3:]:
        print(line[:300])
    print("\ndone:", "ALL PASS" if all(ok for _, ok in vt.RESULTS) else "SOME FAILED")
    return all(ok for _, ok in vt.RESULTS)


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
