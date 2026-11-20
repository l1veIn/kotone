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
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
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


TIMING = re.compile(r"voice-timing: vad→text=(\S+) text→token=(\S+) token→sound=(\S+) vad→sound=(\S+) first=(\S+) "
                    r"filler=(\S+)(?: tts=(\S+))?")


def voice_turn(name, text, click, wait_follow_up=True, timeout=90):
    """Say `text` into CABLE (clicking call first unless she is already listening). Returns (mark, timing match)."""
    guard()
    wav = os.path.join(WORK, f"{name}.wav")
    speak_to_wav(text, wav)
    mark = len(log_text())
    if click:
        click_call()
    play_into_cable(wav, 0.8)
    timing = wait_log(mark, TIMING.pattern, timeout)
    if wait_follow_up:
        wait_log(mark, r"follow-up: (listening|not listening)", timeout)
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
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 8,
                                 "wake": {"enabled": False}},
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
        follow = wait_log(mark, r"follow-up: listening for 8s", 1)
        check("turn 1: listening on after she finished", follow)

        # 2. 不按键，在接着听的 8 秒里直接说
        mark, t = voice_turn("t2", "那你都能帮我做什么", click=False)
        rows.append(("2 接着说（不按键）", t))
        users = [m for m in main_conversation()["messages"] if m["role"] == "user"]
        check("turn 2: heard without pressing anything", len(users) >= 2 and users[-1].get("source") != "chat",
              users[-1].get("text") if users else "-")
        time.sleep(9)  # 接着听的 8 秒过去

        # 3. 工具前的一句 + 回答（两路）
        mark, t = voice_turn("t3", "帮我搜一下英雄联盟现在是哪个版本", click=True, timeout=120)
        rows.append(("3 搜版本（工具前一句）", t))
        streams = re.findall(r"speech stream \(deepseek\): (\d+) sentences, first audio (\S+) ms", log_since(mark))
        messages = main_conversation()["messages"]
        calls = [c["name"] for m in messages if m["role"] == "assistant" for c in m.get("toolCalls", [])]
        check("turn 3: searched", "web.search" in calls, ",".join(calls[-3:]))
        # 工具前说不说那一句由模型决定：每一步写了字就是单独的一路（边写边念）。
        start = max(i for i, m in enumerate(messages) if m["role"] == "user")
        worded = [m.get("text", "").strip() for m in messages[start:] if m["role"] == "assistant"
                  and m.get("text", "").strip()]
        check("turn 3: one stream per step that had words", len(streams) == len(worded),
              f"steps with words: {[w[:12] for w in worded]}; streams {streams}")
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
        # 被打断的这一轮不接着听（打断那一下本身开始的收音不算：它在“follow-up”之外）
        time.sleep(10)
        check("turn 4: no listening on after being interrupted",
              not re.search(r"follow-up: listening for", log_since(mark)))
        last = next((m for m in reversed(main_conversation()["messages"]) if m["role"] == "assistant"), {})
        check("turn 4: the conversation keeps only what she said", last.get("text", "").endswith("（被打断）"),
              last.get("text", "")[:120])

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
    print("\ndone:", "ALL PASS" if all(ok for _, ok in RESULTS) else "SOME FAILED")
    return all(ok for _, ok in RESULTS)


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
