"""琴音 speaks to teammates (ADR 020) with the real DeepSeek service.

Isolated Kotone (KOTONE_HOME=target/voicechat-e2e/home, hotkeys off) with the real DeepSeek key (real_deepseek.py,
memory only, deleted afterwards); game voice on, the player's own microphone NOT mixed in (includeMic=false), so only
琴音's voice goes into the virtual cable. The player says (in-game entry) "跟队友说我去拿龙了"; expected: she first
answers the player with reply.say, then voice_chat.say speaks to the teammates — heard on "CABLE Output" (what the game
would hear as the microphone) and on the player's own speakers (WASAPI loopback, needs `soundcard`).
usage: python -X utf8 -I scripts/manual-e2e/voice_chat_real.py
"""
import glob
import json
import os
import shutil
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "voicechat-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
LISTEN_S = 40


# 真机测试不让琴音按键：会往前台窗口打字、点鼠标的工具一律禁止（2026-11-18 voice_chat_real 里她选了打字，
# 字和回车进了桌面）。
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def log_text():
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return f.read()
    except FileNotFoundError:
        return ""


def record_cable(result):
    """Peak per 0.5 s on CABLE Output (the virtual microphone the game would use)."""
    import numpy as np
    import sounddevice as sd
    dev = next(i for i, d in enumerate(sd.query_devices())
               if d["name"].startswith("CABLE Output") and d["max_input_channels"] > 0
               and sd.query_hostapis(d["hostapi"])["name"] == "MME")
    data = sd.rec(int(16000 * LISTEN_S), samplerate=16000, channels=1, device=dev, dtype="float32", blocking=True)
    result["cable"] = [float(np.abs(c).max()) for c in np.array_split(data[:, 0], LISTEN_S * 2)]


def record_speaker(result):
    try:
        import numpy as np
        import soundcard as sc
        mic = sc.get_microphone(sc.default_speaker().name, include_loopback=True)
        data = mic.record(samplerate=16000, numframes=16000 * LISTEN_S)
        result["speaker"] = [float(np.abs(c).max()) for c in np.array_split(data.mean(axis=1), LISTEN_S * 2)]
    except Exception as e:  # noqa: BLE001
        result["speaker_error"] = str(e)


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    return bool(ok)


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 70, "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False},
                   "voiceChat": {"enabled": True, "includeMic": False}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    passed = True
    try:
        time.sleep(8)
        cid = real_deepseek.add_connection(EXE, ENV)
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        deadline = time.time() + 20
        while "voice chat: routing" not in log_text() and time.time() < deadline:
            time.sleep(0.5)
        passed &= check("game voice is routing into the virtual cable", "voice chat: routing" in log_text())
        result = {}
        threads = [threading.Thread(target=record_cable, args=(result,)), threading.Thread(target=record_speaker, args=(result,))]
        for t in threads:
            t.start()
        time.sleep(0.5)
        t0 = time.time()
        call("assistant.submit", {"text": "跟队友说我去拿龙了"})
        for t in threads:
            t.join()
        said = "voice chat: said" in log_text()
        passed &= check("voice_chat.say spoke to teammates", said)
        conv = max(glob.glob(os.path.join(HOME, "agent", "conversations", "*.json")), key=os.path.getmtime)
        messages = json.load(open(conv, encoding="utf-8"))["messages"]
        calls = [(c["name"], c["arguments"]) for m in messages if m["role"] == "assistant" for c in m.get("toolCalls", [])]
        print("tool calls:", calls)
        names = [n for n, _ in calls]
        first_ack = "reply.say" in names and "voice_chat.say" in names and names.index("reply.say") < names.index("voice_chat.say")
        passed &= check("she answered the player before speaking to teammates", first_ack)
        cable = result.get("cable", [])
        loud = [i / 2 for i, p in enumerate(cable) if p > 0.05]
        passed &= check("the teammates' side (CABLE Output) carried her voice", loud,
                        f"peak {max(cable or [0]):.2f}, loud from {loud[:1]} s to {loud[-1:]} s")
        speaker = result.get("speaker")
        if speaker is None:
            print("SKIP player's speakers:", result.get("speaker_error"))
        else:
            passed &= check("the player heard it too (default speaker)", max(speaker) > 0.05, f"peak {max(speaker):.2f}")
        print(f"took {time.time() - t0:.1f} s")
    finally:
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
