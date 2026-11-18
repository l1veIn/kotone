"""Real-machine test of sub-sessions + the ear (ADR 021 / 022) on an isolated Kotone instance.

琴音 (mock AI, scripted by mock_sessions.py) spawns an interpreter sub-session, which starts listening to
python.exe; this script then plays an English sentence from a separate python process; the sub-session must
receive the recognised sentence, show a (mock) translation and send it to the main session; "停止同传" must
stop the sub-session and its listener.

Needs: target/debug/kotone.exe (embedded frontend build). Uses KOTONE_HOME=target/sess-e2e/home, never ~/.kotone.
The SenseVoice model (~240 MB) is downloaded into the isolated home on first run.
usage: python -X utf8 -I scripts/manual-e2e/sessions_e2e.py
"""
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "sess-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
PORT = 18766
SENSEVOICE = "sense-voice-zh-en-ja-ko-yue-2024-07-17"


def call(tool, args=None, timeout_ms=30_000):
    path = os.path.join(WORK, "args.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(args if args is not None else {}, f, ensure_ascii=False)
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", path, "--timeout-ms", str(timeout_ms)],
                         capture_output=True, text=True, encoding="utf-8", env=ENV)
    try:
        envelope = json.loads(out.stdout)
    except json.JSONDecodeError:
        raise RuntimeError(f"{tool}: {out.stdout} {out.stderr}")
    if not envelope.get("ok"):
        raise RuntimeError(f"{tool}: {json.dumps(envelope.get('error'), ensure_ascii=False)}")
    return envelope["result"]


def wait_until(check, seconds, what):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = check()
        if value:
            return value
        time.sleep(1)
    raise AssertionError(f"timed out waiting for {what}")


def conversations():
    out = []
    agent = os.path.join(HOME, "agent", "conversations")
    for name in os.listdir(agent) if os.path.isdir(agent) else []:
        if name.startswith("c-") and name.endswith(".json"):
            with open(os.path.join(agent, name), encoding="utf-8") as f:
                out.append(json.load(f))
    return out


def child():
    return next((c for c in conversations() if c.get("label") == "同传"), None)


def log_text():
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return f.read()
    except FileNotFoundError:
        return ""


def make_speech(path):
    """English sentence via Windows SAPI (no network)."""
    script = (
        "Add-Type -AssemblyName System.Speech;"
        "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer;"
        "$v = $s.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -like 'en-*' } | Select-Object -First 1;"
        "if ($v) { $s.SelectVoice($v.VoiceInfo.Name) };"
        "$s.Rate = -1;"
        f"$s.SetOutputToWaveFile('{path}');"
        "$s.Speak('Enemy behind the red house. There are two of them.');"
        "$s.Dispose()"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


PLAYER = r"""
import sys, time, wave, numpy as np, sounddevice as sd
w = wave.open(sys.argv[1], 'rb')
data = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768
rate = w.getframerate()
time.sleep(float(sys.argv[2]))
for _ in range(2):
    sd.play(data, rate); sd.wait(); time.sleep(1.5)
"""


def main():
    import shutil
    shutil.rmtree(os.path.join(HOME, "agent"), ignore_errors=True)
    os.makedirs(HOME, exist_ok=True)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"toolService": {"enabled": True}, "ui": {"firstRunCompleted": True}}, f)
    open(os.path.join(WORK, "requests.jsonl"), "w").close()
    mock = subprocess.Popen([sys.executable, "-X", "utf8", "-I", os.path.join(HERE, "mock_sessions.py"), str(PORT), WORK])
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    passed = True
    try:
        wait_until(lambda: os.path.exists(os.path.join(HOME, "kotone.log")) and "tool service" in log_text().lower() or True, 5, "start")
        time.sleep(6)
        def sensevoice_ready():
            m = next(m for m in call("models.list", {"category": "asr"})["models"] if m["id"] == SENSEVOICE)
            return m.get("state") == "ready" or m.get("installed") is True or m.get("downloaded") is True
        if not sensevoice_ready():
            print("downloading SenseVoice into the isolated home ...")
            call("models.download", {"id": SENSEVOICE, "category": "asr"})
            wait_until(sensevoice_ready, 900, "SenseVoice download")
        call("connections.upsert", {"connection": {"id": "mock", "displayName": "mock", "kind": "remote", "provider": "custom",
                                                    "baseUrl": f"http://127.0.0.1:{PORT}/v1", "model": "mock"}, "apiKey": "sk-local-fixture"})
        call("settings.update", {"patch": {"assistant": {"connectionId": "mock", "defaultDelivery": ["danmaku"]},
                                           "agent": {"progressSpeech": False}}})
        speech = os.path.join(WORK, "speech.wav")
        make_speech(speech)
        player = subprocess.Popen([sys.executable, "-c", PLAYER, speech, "12"])

        t0 = time.time()
        call("assistant.submit", {"text": "帮我盯着队友"})
        c = wait_until(child, 60, "the interpreter sub-session")
        print(f"sub-session {c['id']} created after {time.time() - t0:.1f}s")
        passed &= bool(wait_until(lambda: "listens to python.exe" in log_text(), 60, "listen start"))
        print("listening to python.exe")

        def heard():
            c = child()
            for m in (c or {}).get("messages", []):
                if m.get("role") == "user" and m.get("source") == "heard":
                    data = " ".join(a.get("text", "") for a in m.get("attachments", []))
                    if data:
                        return data
            return None
        sentence = wait_until(heard, 90, "a heard sentence in the sub-session")
        print("sub-session heard:", sentence)
        ok = "enemy" in sentence.lower() or "house" in sentence.lower()
        print(("PASS" if ok else "FAIL") + " recognised the English sentence")
        passed &= ok

        def forwarded():
            for c in conversations():
                if c.get("label"):
                    continue
                for m in c.get("messages", []):
                    if m.get("role") == "user" and m.get("source") == "session":
                        return True
            return False
        ok = bool(wait_until(forwarded, 60, "the sub-session's message in the main session"))
        print(("PASS" if ok else "FAIL") + " sub-session -> main session.send")
        passed &= ok

        call("assistant.submit", {"text": "停止同传"})
        ok = bool(wait_until(lambda: "stopped (父会话结束了它)" in log_text() and " stopped" in log_text().split("listen:")[-1], 60, "stop"))
        print(("PASS" if ok else "FAIL") + " session.stop stopped the sub-session and its listener")
        passed &= ok
        player.wait(timeout=60)
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
