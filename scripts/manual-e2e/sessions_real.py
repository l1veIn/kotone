"""Interpreter sub-session (ADR 021 / 022) with the real DeepSeek model.

Like sessions_e2e.py, but no scripted mock: the real model has to decide to spawn a sub-session, the sub-session has to
start listening to python.exe (a separate python process plays an English sentence twice on the default speakers),
translate what it hears, and stop with its listener on "停止同传". Uses the player's real DeepSeek key
(real_deepseek.py, memory only, deleted afterwards). Isolated KOTONE_HOME=target/sess-real/home; the SenseVoice model is
read from target/sess-e2e/home/models (downloaded by sessions_e2e.py). Input tools denied.
usage: python -X utf8 -I scripts/manual-e2e/sessions_real.py
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
from sessions_e2e import PLAYER, make_speech  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "sess-real")
HOME = os.path.join(WORK, "home")
MODELS = os.path.join(REPO, "target", "sess-e2e", "home", "models")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def conversations():
    out = []
    agent = os.path.join(HOME, "agent", "conversations")
    for name in os.listdir(agent) if os.path.isdir(agent) else []:
        if name.endswith(".json"):
            with open(os.path.join(agent, name), encoding="utf-8") as f:
                out.append(json.load(f))
    return out


def child():
    return next((c for c in conversations() if c.get("parent")), None)


def log_text():
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return f.read()
    except FileNotFoundError:
        return ""


def wait_until(check, seconds, what):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = check()
        if value:
            return value
        time.sleep(1)
    print(f"     (timed out waiting for {what})")
    return None


def say(msg):
    return re.search(r"[一-鿿]", msg or "") is not None


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "models": {"dir": MODELS},
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "defaultDelivery": ["danmaku"],
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    passed = True
    player = None
    try:
        time.sleep(8)
        cid = real_deepseek.add_connection(EXE, ENV)
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        speech = os.path.join(WORK, "speech.wav")
        make_speech(speech)
        player = subprocess.Popen([sys.executable, "-c", PLAYER, speech, "25"])
        t0 = time.time()
        call("assistant.submit", {"text": "帮我盯着队友在说什么：他们的声音从 python.exe 这个程序出来，听到英语就翻译成中文用弹幕告诉我。"})
        c = wait_until(child, 90, "a sub-session")
        passed &= bool(c)
        print(("PASS" if c else "FAIL") + f" the main session spawned a sub-session {(c or {}).get('label')!r} after {time.time() - t0:.0f} s")
        listening = wait_until(lambda: re.search(r"listen: \S+ listens to python\.exe", log_text()), 90, "the listener")
        passed &= bool(listening)
        print(("PASS" if listening else "FAIL") + " the sub-session listens to python.exe")

        def heard():
            for m in (child() or {}).get("messages", []):
                if m.get("role") == "user" and m.get("source") == "heard":
                    return " ".join(a.get("text", "") for a in m.get("attachments", [])) or m.get("text")
            return None
        sentence = wait_until(heard, 120, "a heard sentence")
        ok = bool(sentence) and ("enemy" in sentence.lower() or "house" in sentence.lower())
        passed &= ok
        print(("PASS" if ok else "FAIL") + f" heard the English sentence: {sentence!r}")

        def translated():
            messages = (child() or {}).get("messages", [])
            first_heard = next((i for i, m in enumerate(messages) if m.get("source") == "heard"), None)
            if first_heard is None:
                return None
            # 只看听到之后的：之前那句“我先开始监听”不算翻译
            for m in messages[first_heard + 1:]:
                if m.get("role") == "assistant":
                    for call_ in m.get("toolCalls", []):
                        if call_["name"] == "reply.say" and say(call_["arguments"]):
                            return call_["arguments"]
                    if say(m.get("text")):
                        return m.get("text")
            return None
        translation = wait_until(translated, 90, "a translation")
        passed &= bool(translation)
        print(("PASS" if translation else "FAIL") + f" translated into Chinese: {translation}")
        forwarded = any(m.get("role") == "user" and m.get("source") == "session"
                        for c in conversations() if not c.get("parent") for m in c.get("messages", []))
        print(("INFO" if forwarded else "INFO") + f" told the main session (only for urgent things): {forwarded}")

        call("assistant.submit", {"text": "停止同传"})
        stopped = wait_until(lambda: re.search(r"listen: \S+ stopped", log_text()), 90, "the listener to stop")
        passed &= bool(stopped)
        print(("PASS" if stopped else "FAIL") + " 停止同传 stopped the sub-session and its listener")
        today = call("usage.summary")["today"]
        print("usage today:", json.dumps(today, ensure_ascii=False))
    finally:
        if player:
            player.kill()
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
