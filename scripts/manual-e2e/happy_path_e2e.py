"""Real-machine happy path: with only a DeepSeek key, 琴音 configures Kotone, then the player calls her by voice.

Isolated Kotone (KOTONE_HOME=target/happy-e2e/home, hotkeys off, permission mode allow-all) + mock_deepseek.py.
1. The player types "帮我把 Kotone 配置好，用 DeepSeek 就行" in the chat (WebView2 over CDP, DOM input only).
   Expect: the system prompt lists the bundled kotone-config skill, 琴音 reads it with skill.view, switches speech
   recognition to deepseek-asr and OCR to deepseek-ocr; the tool cards link to the changed settings.
2. The player presses 「呼叫琴音」 on the home page and speaks (a SAPI sentence played into the VB-CABLE
   "CABLE Input", which the isolated Kotone uses as its microphone).
   Expect: the audio goes to DeepSeek ASR (/v1/tools/asr), the text reaches 琴音, and her answer is spoken with
   DeepSeek TTS (/v1/tools/tts) and shown in the chat.
Needs: target/debug/kotone.exe with the embedded frontend, VB-CABLE, Node + Playwright (apps/desktop).
usage: python -X utf8 -I scripts/manual-e2e/happy_path_e2e.py
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "happy-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
PORT = 18767
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ASR_TEXT = "帮我看看队友说了什么"


def call(tool, args=None, timeout_ms=30_000):
    path = os.path.join(WORK, "args.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(args if args is not None else {}, f, ensure_ascii=False)
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", path, "--timeout-ms", str(timeout_ms)],
                         capture_output=True, text=True, encoding="utf-8", env=ENV)
    envelope = json.loads(out.stdout)
    if not envelope.get("ok"):
        raise RuntimeError(f"{tool}: {json.dumps(envelope.get('error'), ensure_ascii=False)}")
    return envelope["result"]


def wait_until(check, seconds, what):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.5)
    raise AssertionError(f"timed out waiting for {what}")


def requests():
    try:
        with open(os.path.join(WORK, "requests.jsonl"), encoding="utf-8") as f:
            return [json.loads(line) for line in f if line.strip()]
    except FileNotFoundError:
        return []


def chats():
    return [r for r in requests() if r["path"].endswith("/chat/completions")]


def ui(script):
    """Run a Playwright snippet against the isolated app's settings window (CDP)."""
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


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    return bool(ok)


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": {}}, "progressSpeech": False}}, f)
    mock = subprocess.Popen([sys.executable, "-X", "utf8", "-I", os.path.join(HERE, "mock_deepseek.py"), str(PORT), WORK],
                            env=dict(os.environ, MOCK_ASR_TEXT=ASR_TEXT))
    app = subprocess.Popen([EXE], env=ENV)
    passed = True
    try:
        wait_until(lambda: os.path.exists(os.path.join(HOME, "tool-service")) or os.path.exists(os.path.join(HOME, "kotone.log")), 30, "start")
        time.sleep(6)
        call("connections.upsert", {"connection": {"id": "deepseek", "displayName": "DeepSeek", "kind": "remote", "provider": "deepseek",
                                                    "baseUrl": f"http://127.0.0.1:{PORT}/v1", "model": "deepseek-chat"}, "apiKey": "sk-local-fixture"})
        call("settings.update", {"patch": {"assistant": {"connectionId": "deepseek"}}})

        # 1. 只给了 DeepSeek Key：让琴音把 Kotone 配好
        print(ui("""
await page.reload(); await page.waitForTimeout(1500);
await page.getByTestId("settings-nav-assistant").click();
const box = page.getByTestId("assistant-request");
await box.fill("帮我把 Kotone 配置好，用 DeepSeek 就行");
await box.press("Enter");
await page.getByText("配好了：语音识别和文字识别都用 DeepSeek。").first().waitFor({ timeout: 60000 });
const links = await page.getByTestId("agent-tool-changed-link").allTextContents();
console.log("links:", JSON.stringify(links));
await page.screenshot({ path: "%s" });
""" % os.path.join(WORK, "configured.png").replace("\\", "/")))
        first = chats()[0]["body"]
        system = first["messages"][0]["content"] if isinstance(first["messages"][0]["content"], str) else json.dumps(first["messages"][0]["content"], ensure_ascii=False)
        tools = [t["function"]["name"] for t in first.get("tools", [])]
        passed &= check("the system prompt lists the bundled kotone-config skill", "kotone-config" in system)
        passed &= check("skill.view and models.activate are offered without searching", "skill_view" in tools and "models_activate" in tools, f"{len(tools)} tools")
        second = chats()[1]["body"]["messages"]
        skill_result = next((m for m in second if m.get("role") == "tool"), {})
        passed &= check("skill.view returned the skill text", "推荐方案 A：只用 DeepSeek" in json.dumps(skill_result, ensure_ascii=False))
        settings = call("settings.get", {})
        settings = settings.get("settings", settings)
        passed &= check("speech recognition is DeepSeek", settings.get("sttEngine") == "deepseek-asr", settings.get("sttEngine"))
        passed &= check("OCR is DeepSeek", (settings.get("ocr") or {}).get("activeModelId") == "deepseek-ocr", (settings.get("ocr") or {}).get("activeModelId"))

        # 2. 呼叫琴音，用嘴说
        devices = call("audio.devices")["devices"]
        cable = next(d["id"] for d in devices if d["id"].startswith("CABLE Output"))
        call("settings.update", {"patch": {"audioDeviceId": cable}})
        call("runtime.start", timeout_ms=30_000)
        speech = os.path.join(WORK, "speech.wav")
        make_speech(speech)
        before = len(chats())
        ui("""
await page.getByTestId("settings-nav-general").click();
await page.getByTestId("home-call").waitFor({ timeout: 15000 });
await page.getByTestId("home-call").click();
""")
        play_into_cable(speech, 0.6)
        asr = wait_until(lambda: [r for r in requests() if r["path"].endswith("/tools/asr")], 30, "DeepSeek ASR request")
        passed &= check("the call's audio went to DeepSeek ASR as a WAV file", asr[0]["bytes"] > 16000 and asr[0]["multipartFileField"], f"{asr[0]['bytes']} bytes")
        reply = wait_until(lambda: [c for c in chats()[before:] if ASR_TEXT in json.dumps(c["body"]["messages"][-1], ensure_ascii=False)], 30, "the heard text in a chat request")
        passed &= check("琴音received the recognized words", bool(reply))
        tts = wait_until(lambda: [r for r in requests() if r["path"].endswith("/tools/tts") and "收到" in json.dumps(r["body"], ensure_ascii=False)], 30, "her spoken answer")
        passed &= check("her answer was spoken with DeepSeek TTS", bool(tts), tts[0]["body"].get("text", ""))
    finally:
        try:
            call("app.quit")
        except Exception as e:
            print("quit:", e)
        time.sleep(3)
        mock.terminate()
    print("done:", "ALL PASS" if passed else "SOME FAILED")
    return passed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
