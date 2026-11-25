"""Switching roles in a real isolated instance: name, voice and wake word follow the role.

Isolated Kotone (KOTONE_HOME=target/roles-e2e/home, hotkeys off) with the real DeepSeek key (real_deepseek.py, memory
only, deleted afterwards): DeepSeek speech recognition, the downloaded wake model copied from target/agent-smoke,
VB-CABLE "CABLE Output" as the microphone, wake answer = voice (synthesized with the role's voice, played at volume 0).
For 琴音 → 大肥鱼 → Claude大小姐 → 琴音: roles.switch, check agent.identity, check the wake detector restarted with the role's
wake word, play the wake word (SAPI) into "CABLE Input", check it was detected and that the "嗯？" answer was synthesized
with the role's DeepSeek voice.
usage: python -X utf8 -I scripts/manual-e2e/roles_real.py
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
from wake_eval import read_wav, sapi, pad, write_wav  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "roles-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
WAKE_MODEL = os.path.join(REPO, "target", "agent-smoke", "home", "models", "wake")


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def log_lines():
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return f.read().splitlines()
    except FileNotFoundError:
        return []


def wait_log(pattern, since, seconds=20):
    deadline = time.time() + seconds
    while time.time() < deadline:
        for line in log_lines()[since:]:
            if re.search(pattern, line):
                return line
        time.sleep(0.5)
    return None


def play(path):
    import numpy as np
    import sounddevice as sd
    data = read_wav(path)
    dev = next(i for i, d in enumerate(sd.query_devices())
               if d["name"].startswith("CABLE Input") and d["max_output_channels"] > 0
               and sd.query_hostapis(d["hostapi"])["name"] == "MME")
    sd.play(np.asarray(data, dtype="float32"), 16000, device=dev, blocking=True)


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail else ""))
    return bool(ok)


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    shutil.copytree(WAKE_MODEL, os.path.join(HOME, "models", "wake"))
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   # callIdleSeconds 0 = 不通话（ADR 033）：喊醒以后没人说话就结束，下一个角色的唤醒词才会被当成唤醒词
                   # 听（通话开着时开口就是下一句，不再认唤醒词；缺省 30 秒，比下面等的 7 秒长）
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "callIdleSeconds": 0,
                                 "wake": {"enabled": True, "ack": "voice"}}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    passed = True
    try:
        time.sleep(8)
        cid = real_deepseek.add_connection(EXE, ENV)
        cable = next(d["id"] for d in call("audio.devices")["devices"] if d["id"].startswith("CABLE Output"))
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}, "sttEngine": "deepseek-asr", "audioDeviceId": cable}})
        since = len(log_lines())
        call("runtime.start")
        # wake.rs 从 ce4e66ca（ADR 033）起打的是 "wake: listening (wake=true, phrase=…"
        line = wait_log(r"wake: listening \(wake=true, phrase=琴音琴音", since, 30)
        passed &= check("琴音: wake detector listens for 琴音琴音", line, line)
        for role, word, voice in [("dafeiyu", "你好大肥鱼", "deepseek/stella"), ("claude-ojou", "你好大小姐", "deepseek/stella"), ("kotone", "琴音琴音", None)]:
            since = len(log_lines())
            switched = since
            result = call("roles.switch", {"id": role})
            settings = call("settings.get")
            settings = settings.get("settings", settings)
            identity = settings["agent"]["identity"]
            passed &= check(f"{role}: switched, identity follows", result["current"] == role and identity["name"] == result["name"],
                            f"{identity['name']}")
            line = wait_log(rf"wake: listening \(wake=true, phrase={word}", since, 30)
            passed &= check(f"{role}: wake detector restarted with {word}", line)
            wav = os.path.join(WORK, f"{word}.wav")
            sapi(word, wav + ".src.wav")
            write_wav(wav, pad(read_wav(wav + ".src.wav"), 0.5, 1.5))
            os.remove(wav + ".src.wav")
            time.sleep(1.5)
            since = len(log_lines())
            play(wav)
            line = wait_log(rf"wake: detected {word}", since, 10)
            if not line:
                # 喊一次没中（真机也会有），再喊一次，记下来
                print(f"     {word}: first try missed, trying again")
                time.sleep(2)
                since = len(log_lines())
                play(wav)
                line = wait_log(rf"wake: detected {word}", since, 10)
            passed &= check(f"{role}: saying {word} wakes her", line, line)
            if voice:
                # 应答“嗯？/我在”在开始监听时就用角色的声音先合成好了（预取）。
                synth = wait_log(r"speech: synthesized .* with deepseek/", switched, 15)
                passed &= check(f"{role}: her answer uses the role's voice {voice}", synth and voice in synth, synth)
            time.sleep(7)  # 唤醒会话没人说话，5 秒后结束
    finally:
        try:
            call("roles.switch", {"id": "kotone"})
        except Exception:  # noqa: BLE001
            pass
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
