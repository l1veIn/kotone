"""ADR 029 phase 3 real-machine timing: voice turns whose answer is spoken while it is being written.

Real desktop app (debug build with the web UI embedded, `--background`: no window, no focus change), isolated
KOTONE_HOME=target/voice-real/home, the player's real DeepSeek key in memory only (real_deepseek.py) — chat, speech
recognition and speech synthesis are all DeepSeek. No OS input at all: the chat page's call button is clicked through
WebView2's DevTools port (Playwright connectOverCDP), and the player's words are SAPI sentences played into
"CABLE Input" (the isolated instance listens on "CABLE Output"). Wake word off, hotkeys off, input tools denied.
Her voice is audible but quiet (speech volume 8). Stops before starting — and before every sentence — if a game is
in front.

Turns:
1. call → “你好，你是谁”: a plain answer.
2. no click: speak inside the 8-second follow-up window → “那你都能帮我做什么”.
3. call → “帮我搜一下英雄联盟现在是哪个版本”: the line before web.search, then the answer (two streams).
4. call → a long answer; ~1.5 s after she starts, click call again: she stops at once, the conversation keeps only
   what she said plus “（被打断）”, and she does not listen on afterwards.
5. restart: the waiting phrases come back from <KOTONE_HOME>/cache/tts without new speech charges.
Prints the voice-timing lines as a table and the speech charges per turn from usage.jsonl.
Environment (optional): KOTONE_TEST_SPEECH_VOLUME — her speech volume, default 8 (0 = silent: the stream still plays
at zero gain, the pre-made phrases are skipped; use it when someone is asleep); KOTONE_TEST_EXE — the kotone.exe to run
(a copy outside target/debug, so other builds are not blocked while it runs).
Call mode (ADR 033, 2026-11-25 update of this script): a click starts a call that keeps listening; clicking again
hangs up. So a “call →” turn first hangs up a call that is still open, and “she keeps listening” means the call is
still connected (there is no “follow-up: listening” log line any more). Since the 2026-11-26 revision of ADR 033 the
default after a call is 说一句就走 (one sentence, quiet after answering), so this script sets assistant.callMode = call.
usage: python -X utf8 -I scripts/manual-e2e/voice_turn_real.py
"""
import ctypes
import ctypes.wintypes as wt
import glob
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

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "voice-real")
HOME = os.path.join(WORK, "home")
EXE = os.environ.get("KOTONE_TEST_EXE") or os.path.join(REPO, "target", "debug", "kotone.exe")
VOLUME = int(os.environ.get("KOTONE_TEST_SPEECH_VOLUME", "8"))
PORT = 9335
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}
# 前台是这些程序就不测（会出声；玩家可能正在对局）。
GAMES = ("league of legends", "leagueclient", "riotclient", "valorant", "genshinimpact", "yuanshen", "starrail",
         "zenlesszonezero", "dota2", "cs2", "wow", "overwatch", "pubg", "naraka", "eldenring", "steam")
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def foreground_process():
    user32 = ctypes.WinDLL("user32")
    kernel32 = ctypes.WinDLL("kernel32")
    hwnd = user32.GetForegroundWindow()
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    handle = kernel32.OpenProcess(0x1000, False, pid.value)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
        return buf.value
    finally:
        kernel32.CloseHandle(handle)


def guard():
    path = foreground_process()
    name = os.path.basename(path).lower()
    if any(g in name for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def ui(script):
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
const out = {};
try {
%s
} finally { console.log(JSON.stringify(out)); await browser.close(); }
""" % (PORT, script)
    proc = subprocess.run(["node", "--input-type=module", "-e", code], capture_output=True, text=True,
                          encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    if proc.returncode != 0:
        raise RuntimeError((proc.stdout + proc.stderr)[-3000:])
    return json.loads(proc.stdout.strip().splitlines()[-1])


def wait_ui(timeout=60):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            ui('await page.waitForSelector("[data-testid=app-frame]", { state: "attached", timeout: 2000 });')
            return
        except Exception:  # noqa: BLE001
            time.sleep(1)
    raise RuntimeError("settings window did not come up")


def click_call():
    """The chat page's call button (the big one on an empty chat, the small one under a conversation).

    Clicked in the page (the window is hidden in --background mode, so no actionability checks)."""
    r = ui("""
const button = page.locator('[data-testid=chat-call], [data-testid=call-kotone]').first();
await button.waitFor({ state: "attached", timeout: 10000 });
out.at = Date.now() / 1000;
await button.evaluate((el) => el.click());
""")
    return r["at"]


def speak_to_wav(text, path):
    script = (
        "Add-Type -AssemblyName System.Speech;"
        "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer;"
        "$v = $s.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -eq 'zh-CN' } | Select-Object -First 1;"
        "if ($v) { $s.SelectVoice($v.VoiceInfo.Name) };"
        "$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono);"
        f"$s.SetOutputToWaveFile('{path}', $f);"
        f"$s.Speak('{text}');"
        "$s.Dispose()"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


def play_into_cable(path, lead_s):
    import numpy as np
    import sounddevice as sd
    import wave
    w = wave.open(path)
    data = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768
    data = np.concatenate([np.zeros(int(16000 * lead_s), dtype=np.float32), data, np.zeros(16000, dtype=np.float32)])
    dev = next(i for i, d in enumerate(sd.query_devices())
               if d["name"].startswith("CABLE Input") and d["max_output_channels"] > 0
               and sd.query_hostapis(d["hostapi"])["name"] == "MME")
    sd.play(data, 16000, device=dev, blocking=True)


def log_text():
    path = os.path.join(HOME, "kotone.log")
    if not os.path.exists(path):
        return ""
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def log_since(mark):
    return log_text()[mark:]


def wait_log(mark, pattern, timeout):
    deadline = time.time() + timeout
    while time.time() < deadline:
        m = re.search(pattern, log_since(mark))
        if m:
            return m
        time.sleep(0.3)
    return None


def line_time(mark, pattern):
    """Timestamp (epoch seconds) of the first log line after `mark` matching `pattern`."""
    for line in log_since(mark).splitlines():
        if re.search(pattern, line):
            m = re.match(r"\[(\d+\.\d+)\]", line)
            return float(m.group(1)) if m else None
    return None


def ledger():
    path = os.path.join(HOME, "usage.jsonl")
    return [json.loads(line) for line in open(path, encoding="utf-8")] if os.path.exists(path) else []


def main_conversation():
    files = [f for f in glob.glob(os.path.join(HOME, "agent", "conversations", "*.json"))
             if "parent" not in json.load(open(f, encoding="utf-8"))]
    return json.load(open(max(files, key=os.path.getmtime), encoding="utf-8")) if files else {"messages": []}


def spoken(message):
    """What she said aloud in one assistant step: the text of its `speak` calls (ADR 029 §8: only speak is spoken)."""
    out = []
    for c in message.get("toolCalls", []) or []:
        if c["name"] == "speak":
            try:
                out.append(json.loads(c["arguments"]).get("text", ""))
            except Exception:  # noqa: BLE001
                pass
    return " ".join(out)


def turn_speech(messages):
    """For the latest player turn: (spoken text, text shown only in the chat, speak tool results, what the player said).
    Shown-only text is the reply body plus speak's `written`."""
    start = max(i for i, m in enumerate(messages) if m["role"] == "user" and m.get("source") != "agent-report")
    steps = [m for m in messages[start:] if m["role"] == "assistant"]
    said = " ".join(s for s in (spoken(m) for m in steps) if s)
    shown = [m.get("text", "").strip() for m in steps]
    for m in steps:
        for c in m.get("toolCalls", []) or []:
            if c["name"] == "speak":
                try:
                    shown.append(json.loads(c["arguments"]).get("written", "").strip())
                except Exception:  # noqa: BLE001
                    pass
    body = " ".join(s for s in shown if s)
    # 交给后台、她自己没说话：Kotone 替她说了“交给我，好了告诉你”（ADR 029 第八节），不算漏调 speak；
    # 慢的工具 3 秒没回来转到了后台（ADR 029 §一，结果里 background: true）也一样
    moved = any(m["role"] == "tool" and '"background":true' in (m.get("content") or "").replace(" ", "")
                for m in messages[start:])
    if not said and (moved or any(c["name"] in ("agent.start", "agent.followup")
                                  for m in steps for c in m.get("toolCalls", []) or [])):
        said = "（交给后台）交给我，好了告诉你"
    results = [m.get("content", "") for m in messages[start:] if m["role"] == "tool" and m.get("name") == "speak"]
    return said, body, results, messages[start].get("text", "")


VOICE_TURNS = []  # (name, spoken, body)
PATHY = re.compile(r"[A-Za-z]:\\|\\|/[\w.-]+/|https?://|www\.|\.json|\.exe")


TIMING = re.compile(r"voice-timing: vad→text=(\S+) text→token=(\S+) token→sound=(\S+) vad→sound=(\S+) first=(\S+) "
                    r"filler=(\S+)(?: tts=(\S+))?")


def in_call():
    """A call is open (ADR 033): the last “call: connected” is after the last “call: ended”."""
    text = log_text()
    return text.rfind("call: connected") > text.rfind("call: ended")


def hang_up():
    """Hang up a call that is still open, so the next click starts a new call (ADR 033 D1: a click in a call hangs up)."""
    if in_call():
        mark = len(log_text())
        click_call()
        wait_log(mark, r"call: ended", 10)


# 这一轮说完了：她念完 speak（流式语音那一路结束）或 Kotone 替她说了“交给我”（ADR 033 以后没有“接着听”的日志了）
TURN_DONE = r"speech stream \(deepseek\): \d+ sentences, first audio Some|交给后台，先说"


def voice_turn(name, text, click, wait_follow_up=True, timeout=90):
    """Say `text` into CABLE (clicking call first unless she is already listening). Returns (mark, timing match)."""
    guard()
    wav = os.path.join(WORK, f"{name}.wav")
    speak_to_wav(text, wav)
    if click:
        hang_up()
    mark = len(log_text())
    if click:
        click_call()
    play_into_cable(wav, 0.8)
    timing = wait_log(mark, TIMING.pattern, timeout)
    if wait_follow_up:
        wait_log(mark, TURN_DONE, timeout)
        time.sleep(1)
    return mark, timing


def start_app():
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    wait_ui()
    return app


def stop_app(app):
    try:
        call("app.quit")
    except Exception as e:  # noqa: BLE001
        print("quit:", e)
    time.sleep(3)
    if app.poll() is None:
        app.terminate()
        app.wait(10)


def main():
    guard()
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   # 呼叫以后选通话（ADR 033 2026-11-26 修订起缺省是说一句就走）：第 2 轮不点按钮直接说
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": VOLUME,
                                 "wake": {"enabled": False}, "callMode": "call"},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}}}, f)
    app = start_app()
    rows = []
    try:
        cid = real_deepseek.add_connection(EXE, ENV, model="deepseek-flash")
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        call("models.activate", {"category": "asr", "engineId": "deepseek-asr", "modelId": "deepseek-asr"}, timeout_ms=30_000)
        cable = next(d["id"] for d in call("audio.devices")["devices"] if d["id"].startswith("CABLE Output"))
        call("settings.update", {"patch": {"audioDeviceId": cable}})
        call("runtime.start")
        ui('await page.reload(); await page.waitForTimeout(2000);'
           'await page.getByTestId("sidebar-new-chat").evaluate((el) => el.click()); await page.waitForTimeout(1000);')
        # 等待短句合成好（第一次要请求在线语音）
        ready = wait_log(0, r"fillers: \d+/\d+ ready in \d+ ms", 60)
        print(ready.group(0) if ready else "fillers: not ready")
        tts_files = glob.glob(os.path.join(HOME, "cache", "tts", "*.wav"))
        check("waiting phrases were written to cache/tts", len(tts_files) >= 8, f"{len(tts_files)} files")

        # 1. 呼叫 → 简单回答
        mark, t = voice_turn("t1", "你好，你是谁", click=True)
        rows.append(("1 你好，你是谁", t))
        check("turn 1: answer spoken while written (stream log)", "speech stream (deepseek)" in log_since(mark))
        said, body, _, _ = turn_speech(main_conversation()["messages"])
        VOICE_TURNS.append(("1", said, body))
        check("turn 1: she answered with speak", said, said)
        check("turn 1: listening on after she finished (the call is still open, ADR 033)", in_call())

        # 2. 不按键，在接着听的 8 秒里直接说
        mark, t = voice_turn("t2", "那你都能帮我做什么", click=False)
        rows.append(("2 接着说（不按键）", t))
        users = [m for m in main_conversation()["messages"] if m["role"] == "user"]
        check("turn 2: heard without pressing anything", len(users) >= 2 and users[-1].get("source") != "chat",
              users[-1].get("text") if users else "-")
        said, body, _, _ = turn_speech(main_conversation()["messages"])
        VOICE_TURNS.append(("2", said, body))
        time.sleep(9)  # 接着听的 8 秒过去

        # 3. 先查再说
        mark, t = voice_turn("t3", "帮我搜一下英雄联盟现在是哪个版本", click=True, timeout=120)
        rows.append(("3 搜版本（先查再说）", t))
        streams = re.findall(r"speech stream \(deepseek\): (\d+) sentences, first audio (\S+) ms", log_since(mark))
        messages = main_conversation()["messages"]
        calls = [c["name"] for m in messages if m["role"] == "assistant" for c in m.get("toolCalls", [])]
        check("turn 3: searched", "web.search" in calls, ",".join(calls[-3:]))
        # 只念 speak 的字：一次 speak 一路（正文不念）
        start = max(i for i, m in enumerate(messages) if m["role"] == "user")
        speaks = [spoken(m) for m in messages[start:] if m["role"] == "assistant" and spoken(m)]
        check("turn 3: one stream per speak call, the body is not spoken", len(streams) == len(speaks),
              f"speak: {[s[:12] for s in speaks]}; streams {streams}")
        said, body, _, _ = turn_speech(messages)
        VOICE_TURNS.append(("3", said, body))
        time.sleep(9)

        # 4. 长回答，开口 1.5 秒后按呼叫打断
        mark, t = voice_turn("t4", "详细给我讲讲英雄联盟打野前期怎么开野，分五点说，每点都说清楚", click=True,
                             wait_follow_up=False, timeout=60)
        rows.append(("4 长回答（随后打断）", t))
        time.sleep(1.5)
        clicked = click_call()
        stopped = wait_log(mark, r"voice reply: interrupted", 10)
        stopped_at = line_time(mark, r"voice reply: interrupted")
        check("turn 4: the call button stopped her", stopped,
              f"{(stopped_at - clicked) * 1000:.0f} ms after the click" if stopped_at else "")
        # 被打断的这一轮不接着听：通话里按呼叫键就是挂断（ADR 033 D1）
        time.sleep(10)
        check("turn 4: no listening on after being interrupted (the click hung up)",
              re.search(r"call: ended", log_since(mark)) and not in_call())
        said, body, results, _ = turn_speech(main_conversation()["messages"])
        VOICE_TURNS.append(("4", said, body))
        check("turn 4: the five points were written in the chat (not spoken)", len(body) > 80, body[:80])
        check("turn 4: the speak result says what the player heard", any("玩家打断了" in r for r in results),
              results[-1][:120] if results else "no speak result")

        # 5. 容易带出路径、网址的问题：只有 speak 的字念出来，路径和网址留在正文里
        for i, (question, keyword) in enumerate([("琴音的设置文件放在哪里", "设置文件"), ("DeepSeek 的官网是什么", "官网")]):
            time.sleep(9)
            guard()
            mark, t = voice_turn(f"t5{i}", question, click=True, timeout=120)
            rows.append((f"5 {question}", t))
            said, body, _, heard = turn_speech(main_conversation()["messages"])
            if not check(f"turn 5{i}: heard the question", keyword in heard, heard):
                continue  # 识别出错时这一轮没进对话（不算她的回答）
            VOICE_TURNS.append((f"5{i}", said, body))
            check(f"turn 5{i}: nothing path- or URL-like was spoken", said and not PATHY.search(said),
                  f"spoken “{said}” | body “{body[:80]}”")

        # 费用：每轮的语音合成都记到那一轮
        conv = main_conversation()
        turns = {m["atMs"]: m.get("text", "") for m in conv["messages"] if m["role"] == "user"}
        tts = [r for r in ledger() if r.get("kind") == "tts"]
        mine = [r for r in tts if r.get("conversation") == conv.get("id")]
        print("\nspeech charges by turn:")
        for at, said in turns.items():
            rs = [r for r in mine if r.get("turn") == at]
            print(f"  {said[:16]:<16} {len(rs)} rows, {sum(r.get('characters', 0) for r in rs)} chars, "
                  f"¥{sum(r.get('cost', 0) for r in rs):.4f}")
        stray = [r for r in mine if r.get("turn") not in turns]
        check("every streamed sentence is billed to a turn of this conversation", mine and not stray,
              f"{len(mine)} rows, {len(stray)} without a turn")
        fillers_before = len([r for r in tts if not r.get("conversation")])
        print(f"  waiting phrases (no conversation): {fillers_before} rows")
    finally:
        real_deepseek.delete_connection(EXE, ENV)
        stop_app(app)

    # 5. 重启：等待短句从文件读回来，不再花钱
    guard()
    before = len([r for r in ledger() if r.get("kind") == "tts"])
    mark = len(log_text())
    app = start_app()
    try:
        cid = real_deepseek.add_connection(EXE, ENV, model="deepseek-flash")
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        ready = wait_log(mark, r"fillers: (\d+)/(\d+) ready in (\d+) ms", 60)
        print("after restart:", ready.group(0) if ready else "not ready")
        time.sleep(2)
        after = len([r for r in ledger() if r.get("kind") == "tts"])
        check("after a restart the waiting phrases load from disk", ready and ready.group(1) == ready.group(2),
              ready.group(0) if ready else "")
        check("…without new speech charges", after == before, f"{after - before} new rows")
    finally:
        real_deepseek.delete_connection(EXE, ENV)
        stop_app(app)

    print("\n| 回合 | 说完→识别 | 识别→首字 | 首字→出声 | 说完→出声 | 先出声的 | 填充话 | 开始合成→出声 |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for name, t in rows:
        if t:
            print("| " + " | ".join([name, *[(g or "-") for g in t.groups()]]) + " |")
        else:
            print(f"| {name} | (no voice-timing line) |")
    print("\n| 回合 | 念出来的（speak） | 字数 | 只在聊天里的正文 |")
    print("| --- | --- | --- | --- |")
    for name, said, body in VOICE_TURNS:
        print(f"| {name} | {said or '（没有 speak）'} | {len(said)} | {body[:60]} |")
    missing = [n for n, said, _ in VOICE_TURNS if not said]
    print(f"speak missing: {len(missing)}/{len(VOICE_TURNS)} voice turns {missing}")
    print("\ndone:", "ALL PASS" if all(ok for _, ok in RESULTS) else "SOME FAILED")
    return all(ok for _, ok in RESULTS)


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
