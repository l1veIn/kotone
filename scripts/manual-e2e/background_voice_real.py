"""ADR 029 phase 2 + ADR 030 by voice: background work started by voice, its result spoken by itself, and a
background confirmation asked out loud and answered by voice.

Same setup as voice_turn_real.py (whose helpers it reuses): real desktop app with `--background`, isolated
KOTONE_HOME=target/background-voice/home, the player's real DeepSeek key in memory only (chat, speech recognition and
speech synthesis all DeepSeek), the chat page's call button clicked through WebView2's DevTools port, the player's
words are SAPI sentences played into "CABLE Input" (the instance listens on "CABLE Output"). No OS input. Her voice is
audible but quiet (volume 8). Stops before starting — and before every sentence — if a game is in front.

V1. call → “帮我写一个每隔十分钟提醒我喝水的自动化，先别运行”: she hands it to the background (`agent.start`) and says
    one short line; the background finishes; the result comes back and she *says* it by herself (no click).
V2. call → “把你文件夹里的 notes.txt 删掉”: deleting asks; she asks out loud; the script answers “可以” by voice; the
    background goes on, the file is gone, she says so.
Prints what she said (characters per spoken answer: the voice rule is “at most two sentences, about 40 characters”),
the voice-timing lines, waiting phrases, and the charges.
usage: python -X utf8 -I scripts/manual-e2e/background_voice_real.py
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
import real_deepseek  # noqa: E402
import voice_turn_real as v  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
v.WORK = os.path.join(REPO, "target", "background-voice")
v.HOME = os.path.join(v.WORK, "home")
v.PORT = 9337
v.ENV = dict(os.environ, KOTONE_HOME=v.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(v.WORK, "webview"),
             WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={v.PORT}")
v.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
NOTES = os.path.join(v.HOME, "agent", "files", "notes.txt")
SPOKEN = []


def registry(main):
    path = os.path.join(v.HOME, "agent", "subagents", f"{main}.json")
    if not os.path.exists(path):
        return []
    try:
        return json.load(open(path, encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return []


def after(messages, at_ms):
    return [m for m in messages if m.get("atMs", 0) >= at_ms]


def spoken_answers(messages, since_ms):
    """Her answers in voice turns (the last text of each turn after a player's spoken message or a report)."""
    out = []
    current = None
    for m in messages:
        if m.get("atMs", 0) < since_ms:
            continue
        if m["role"] == "user":
            if current and current["text"]:
                out.append(current)
            current = {"after": m.get("source"), "text": ""}
        elif m["role"] == "assistant" and current is not None and m.get("text", "").strip():
            current["text"] = m["text"].strip()
    if current and current["text"]:
        out.append(current)
    return out


def wait_report(main, since_ms, timeout):
    """Wait until a background result came back after `since_ms` and she answered it."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        msgs = v.main_conversation()["messages"]
        idx = [i for i, m in enumerate(msgs) if m["role"] == "user" and m.get("source") == "agent-report"
               and m["atMs"] >= since_ms]
        if idx:
            rest = [m for m in msgs[idx[-1] + 1:] if m["role"] == "assistant" and m.get("text", "").strip()]
            if rest and not any(a.get("status") in ("running", "waiting") for a in registry(main)):
                return msgs[idx[-1]], rest[-1]
        time.sleep(1)
    return None


STREAM = re.compile(r"\[(\d+\.\d+)\] speech stream \(deepseek\): \d+ sentences, first audio Some\((\d+)\) ms after start, "
                    r"played \d+ ms, total (\d+) ms")


def report_spoken(mark, timeout):
    """Seconds from “result handed back to the main conversation” to her first sound for that turn. The stream line
    is written when the stream ends: start = line time − total; first sound = start + first audio."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        handed = v.line_time(mark, r"background result\(s\) handed back")
        if handed:
            for line in v.log_since(mark).splitlines():
                m = STREAM.match(line)
                if m and float(m.group(1)) > handed:
                    first_sound = float(m.group(1)) - int(m.group(3)) / 1000 + int(m.group(2)) / 1000
                    return first_sound - handed
        time.sleep(0.5)
    return None


def main():
    v.guard()
    shutil.rmtree(v.WORK, ignore_errors=True)
    os.makedirs(v.HOME)
    with open(os.path.join(v.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 8,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": v.NO_INPUT}}}, f)
    os.makedirs(os.path.dirname(NOTES))
    with open(NOTES, "w", encoding="utf-8") as f:
        f.write("2026-11-20\n")
    app = v.start_app()
    started_ms = int(time.time() * 1000)
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

        # V1. 用嘴派活，后台做完她自己开口说结果
        said_at = time.time()
        mark, timing = v.voice_turn("v1", "帮我写一个每隔十分钟提醒我喝水的自动化，先别运行", click=True, timeout=120)
        print("   timing:", timing.group(0) if timing else "-")
        conv = v.main_conversation()
        main = conv.get("id")
        player = next((m for m in reversed(conv["messages"]) if m["role"] == "user" and m.get("source") != "agent-report"), {})
        calls = [t["name"] for m in after(conv["messages"], player.get("atMs", 0)) if m["role"] == "assistant"
                 for t in m.get("toolCalls", [])]
        v.check("V1: handed to the background by voice (agent.start)", "agent.start" in calls, ",".join(calls))
        got = wait_report(main, player.get("atMs", 0), 300)
        v.check("V1: the result came back by itself", got)
        if got:
            report, answer = got
            spoke = report_spoken(mark, 30)
            back = (report["atMs"] / 1000) - said_at
            v.check("V1: she said the result out loud (no click)", spoke and answer["text"],
                    f"report {back:.0f} s after she was asked; she said “{answer['text'][:60]}”")
            if spoke is not None:
                print(f"   result handed back → she starts saying it {spoke:.1f} s later")
            fillers = re.findall(r"filler=(\S+)", v.log_since(mark))
            print("   waiting phrases in V1:", fillers or "none")
        time.sleep(9)  # 接着听的 8 秒

        # V2. 删除要先问：她开口问，用嘴答“可以”
        v.guard()
        mark, timing = v.voice_turn("v2", "把你文件夹里的 notes.txt 删掉", click=True, wait_follow_up=False, timeout=120)
        conv = v.main_conversation()
        player2 = next((m for m in reversed(conv["messages"]) if m["role"] == "user" and m.get("source") != "agent-report"), {})
        question = None
        deadline = time.time() + 180
        while time.time() < deadline and not question:
            question = next((a.get("question") for a in registry(main) if a.get("status") == "waiting"), None)
            time.sleep(0.5)
        v.check("V2: the background waits for the player before deleting", question, question or "")
        asked = v.wait_log(mark, r"voice ask: listening for \d+s", 90)
        v.check("V2: she asked out loud", asked)
        if asked:
            v.guard()
            wav = os.path.join(v.WORK, "yes.wav")
            v.speak_to_wav("可以", wav)
            v.play_into_cable(wav, 0.5)
        got = wait_report(main, player2.get("atMs", 0), 240)
        v.check("V2: went on after “可以” and reported back", got)
        v.check("V2: notes.txt is gone", not os.path.exists(NOTES))
        if got:
            print(f"   she said: {got[1]['text'][:120]}")

        # 她在语音里说的每一句（规则：最多两句、约 40 字）
        conv = v.main_conversation()
        answers = spoken_answers(conv["messages"], started_ms)
        print("\nspoken answers:")
        for a in answers:
            n = len(a["text"])
            sentences = len([s for s in re.split(r"[。！？!?]", a["text"]) if s.strip()])
            print(f"  [{a['after']}] {n} chars, {sentences} sentences: {a['text'][:100]}")
            SPOKEN.append((n, sentences))
        long = [n for n, s in SPOKEN if n > 60 or s > 3]
        v.check("voice answers stay short (≤ ~60 chars / 3 sentences)", not long, f"{len(long)} long of {len(SPOKEN)}")
        rows = [r for r in v.ledger() if r.get("conversation") == main]
        by = {}
        for r in rows:
            key = (r.get("kind"), bool(r.get("background")))
            by[key] = by.get(key, 0) + (r.get("cost") or 0)
        print("\ncharges:", ", ".join(f"{k}{'(后台)' if b else ''} ¥{c:.4f}" for (k, b), c in sorted(by.items())))
    finally:
        real_deepseek.delete_connection(v.EXE, v.ENV)
        v.stop_app(app)
        listing = subprocess.run(["cmdkey", "/list"], capture_output=True, text=True, encoding="mbcs",
                                 errors="replace").stdout
        for line in listing.splitlines():
            if "target=" in line and v.WORK in line:
                subprocess.run(["cmdkey", f"/delete:{line.split('target=', 1)[1].strip()}"], capture_output=True)
                print("removed leftover test credential")
    failed = [name for name, ok in v.RESULTS if not ok]
    print(f"\n{len(v.RESULTS) - len(failed)}/{len(v.RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
