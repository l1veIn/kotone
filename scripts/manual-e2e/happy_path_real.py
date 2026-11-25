"""Real-DeepSeek happy path: with only a DeepSeek key, 琴音 configures Kotone; then voice, TTS and OCR all go to DeepSeek.

Against the real DeepSeek service (real_deepseek.py: the player's key, memory only, deleted afterwards), so it judges
outcomes. (The mock twin happy_path_e2e.py + mock_deepseek.py was deleted 2026-11-25: same story, weaker evidence.)
The connection is added through the tool service, not the onboarding (which would switch everything itself, ADR 028;
that path is copilot_flow_real.py), so her settings start local and it is her job to switch them:
1. Chat "帮我把 Kotone 配置好，用 DeepSeek 就行" (one follow-up if she only asks): she reads the bundled kotone-config
   skill (skill.view) and switches speech recognition to deepseek-asr and OCR to deepseek-ocr.
2. The chat page's call button + a SAPI sentence into VB-CABLE: the words are recognized by DeepSeek ASR (ledger: asr),
   reach her, and her answer is synthesized with a DeepSeek voice (log: "speech: synthesized … with deepseek/").
3. OCR: the test window (target_window.ps1, shows 找到对局) is brought to the front — only if no game is in front —
   captured with screen.capture and recognized with ocr.recognize {backend: deepseek-ocr} (ledger: ocr).
Isolated KOTONE_HOME=target/happy-real/home, hotkeys off, speech volume 0, input tools denied.
usage: python -X utf8 -I scripts/manual-e2e/happy_path_real.py
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
from computer_use_e2e import focus  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "happy-real")
HOME = os.path.join(WORK, "home")
TARGET = os.path.join(WORK, "target")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}
ASR_TEXT = "帮我看看队友说了什么"


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    return bool(ok)


def make_speech(path):
    script = (
        "Add-Type -AssemblyName System.Speech;"
        "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer;"
        "$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono);"
        f"$s.SetOutputToWaveFile('{path}', $f);"
        f"$s.Speak('{ASR_TEXT}');"
        "$s.Dispose()"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


def play_into_cable(path, lead_s):
    import numpy as np
    import sounddevice as sd
    import wave
    w = wave.open(path)
    data = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768
    data = np.concatenate([np.zeros(int(16000 * lead_s), dtype=np.float32), data, np.zeros(16000 * 2, dtype=np.float32)])
    dev = next(i for i, d in enumerate(sd.query_devices())
               if d["name"].startswith("CABLE Input") and d["max_output_channels"] > 0
               and sd.query_hostapis(d["hostapi"])["name"] == "MME")
    sd.play(data, 16000, device=dev, blocking=True)


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def ui(script):
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:9333");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
%s
} finally { await browser.close(); }
""" % script
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    if out.returncode != 0:
        raise RuntimeError(out.stdout + out.stderr)
    return out.stdout.strip()


def send_chat(text):
    # The app opens on the chat (ADR 028); the composer is `assistant-request`. Only if some other page is showing,
    # open a new chat from the sidebar (the follow-up stays in the same conversation).
    ui("""
const box = page.getByTestId("assistant-request");
if (!(await box.count())) await page.getByTestId("sidebar-new-chat").click();
await box.fill(%s);
await box.press("Enter");
""" % json.dumps(text, ensure_ascii=False))
    time.sleep(2)
    deadline = time.time() + 180
    while time.time() < deadline:
        state = json.loads(ui('console.log(JSON.stringify(await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("agent_state"))));'))
        if not state.get("running"):
            return
        time.sleep(2)
    raise AssertionError("琴音 did not finish")


def find_key(value, key):
    """The first value of `key` anywhere in a JSON result."""
    if isinstance(value, dict):
        if key in value:
            return value[key]
        value = list(value.values())
    if isinstance(value, list):
        for item in value:
            found = find_key(item, key)
            if found is not None:
                return found
    return None


def main_conversation():
    files = [f for f in glob.glob(os.path.join(HOME, "agent", "conversations", "*.json"))
             if "parent" not in json.load(open(f, encoding="utf-8"))]
    return json.load(open(max(files, key=os.path.getmtime), encoding="utf-8"))


def ledger():
    path = os.path.join(HOME, "usage.jsonl")
    return [json.loads(line) for line in open(path, encoding="utf-8")] if os.path.exists(path) else []


def log_text():
    with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
        return f.read()


def settings():
    s = call("settings.get")
    return s.get("settings", s)


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    os.makedirs(TARGET)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False}}, f)
    app = subprocess.Popen([EXE], env=ENV)
    target = None
    passed = True
    try:
        time.sleep(10)
        cid = real_deepseek.add_connection(EXE, ENV)
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        ui("await page.reload(); await page.waitForTimeout(1500);")

        # 1. 只给了 DeepSeek Key：让琴音把 Kotone 配好
        send_chat("帮我把 Kotone 配置好，用 DeepSeek 就行")
        s = settings()
        if s.get("sttEngine") != "deepseek-asr" or (s.get("ocr") or {}).get("activeModelId") != "deepseek-ocr":
            print("   (she asked first; answering)", main_conversation()["messages"][-1].get("text", "")[:200].replace("\n", " "))
            send_chat("就按你推荐的来：语音识别和文字识别都用 DeepSeek，不用再问我了。")
            s = settings()
        calls = [(c["name"], c["arguments"]) for m in main_conversation()["messages"] if m["role"] == "assistant"
                 for c in m.get("toolCalls", [])]
        print("tool calls:", [n for n, _ in calls])
        passed &= check("she read the bundled kotone-config skill", any(n == "skill.view" and "kotone-config" in a for n, a in calls))
        passed &= check("speech recognition is DeepSeek", s.get("sttEngine") == "deepseek-asr", s.get("sttEngine"))
        passed &= check("OCR is DeepSeek", (s.get("ocr") or {}).get("activeModelId") == "deepseek-ocr", (s.get("ocr") or {}).get("activeModelId"))
        print("her answer:", main_conversation()["messages"][-1].get("text", "")[:300].replace("\n", " "))

        # 2. 呼叫琴音，用嘴说
        cable = next(d["id"] for d in call("audio.devices")["devices"] if d["id"].startswith("CABLE Output"))
        call("settings.update", {"patch": {"audioDeviceId": cable}})
        call("runtime.start")
        speech = os.path.join(WORK, "speech.wav")
        make_speech(speech)
        asr_before = sum(1 for r in ledger() if r.get("kind") == "asr")
        log_before = len(log_text())
        # The chat page's call button (the big one on an empty chat, the small one under a conversation), as in
        # voice_turn_real.py; the home page and its `home-call` went away with ADR 028 (254b2748).
        ui("""
const button = page.locator('[data-testid=chat-call], [data-testid=call-kotone]').first();
await button.waitFor({ state: "attached", timeout: 15000 });
await button.evaluate((el) => el.click());
""")
        play_into_cable(speech, 0.6)
        deadline = time.time() + 60
        heard = None
        while time.time() < deadline and not heard:
            time.sleep(2)
            users = [m for m in main_conversation()["messages"] if m["role"] == "user" and m.get("source") not in ("chat",)]
            heard = users[-1]["text"] if users else None
        asr = [r for r in ledger() if r.get("kind") == "asr"][asr_before:]
        passed &= check("the call's audio was recognized by DeepSeek ASR", asr and asr[-1].get("provider") == "deepseek",
                        f"{asr[-1].get('audioMs') if asr else '-'} ms audio")
        passed &= check("琴音 received the recognized words", heard, heard)
        deadline = time.time() + 60
        spoken = None
        while time.time() < deadline and not spoken:
            time.sleep(2)
            tail = log_text()[log_before:]
            spoken = next((line for line in tail.splitlines() if "speech: synthesized" in line and "deepseek/" in line), None)
        passed &= check("her answer was spoken with DeepSeek TTS", spoken, spoken)

        # 3. OCR：测试窗口（只在前台不是游戏时才切过去）
        target = subprocess.Popen(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                                   os.path.join(HERE, "target_window.ps1"), "-Dir", TARGET])
        deadline = time.time() + 20
        while not os.path.exists(os.path.join(TARGET, "ready.txt")) and time.time() < deadline:
            time.sleep(0.5)
        focus()
        windows = find_key(call("window.list"), "windows") or []
        window = next(w for w in windows if w.get("title") == "KotoneScriptTarget")
        artifact = find_key(call("screen.capture", {"windowId": window["windowId"]}), "artifactId")
        ocr = call("ocr.recognize", {"artifactId": artifact, "backend": "deepseek-ocr"}, timeout_ms=30_000)
        text = ocr.get("text") or " ".join(o.get("text", "") for o in ocr.get("observations", []))
        passed &= check("DeepSeek OCR read the test window", "找到对局" in text, text[:80].replace("\n", " "))
        passed &= check("the OCR call is in the ledger", any(r.get("kind") == "ocr" for r in ledger()))
        print("usage today:", json.dumps(call("usage.summary")["today"], ensure_ascii=False))
    finally:
        if target:
            with open(os.path.join(TARGET, "control.txt"), "w") as f:
                f.write("close")
        real_deepseek.delete_connection(EXE, ENV)
        try:
            call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
    print("done:", "ALL PASS" if passed else "SOME FAILED")
    return passed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
