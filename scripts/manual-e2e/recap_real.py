"""Match recap against the real DeepSeek service: a game process exits → recap in the chat + the game's memory.

Isolated Kotone (KOTONE_HOME=target/recap-e2e/home, hotkeys off, speech volume 0, KOTONE_RECAP_MIN_SECONDS=20) with the
real DeepSeek key (real_deepseek.py, memory only, deleted afterwards). The "game" is a copy of Windows' PING.EXE
renamed kotone-fake-game.exe and run as `-n 600 127.0.0.1` (a unique process name, so none of the player's own programs is watched or touched),
registered as the game profile "测试游戏". While it runs, three things are said to 琴音 through the in-game entry
(assistant.submit); then the process is killed and the script waits for the recap.
usage: python -X utf8 -I scripts/manual-e2e/recap_real.py
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

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "recap-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
FAKE = os.path.join(WORK, "kotone-fake-game.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           KOTONE_RECAP_MIN_SECONDS="20")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
SAID = [
    "我在玩测试游戏，记住：这个游戏里按 F 是开地图，按 Tab 看比分。",
    "这把我玩的是法师，提醒我 30 秒后补血。",
    "这游戏的 Boss 叫“石像鬼”，打它要先拆两边的柱子，记一下。",
]


# 真机测试不让琴音按键：会往前台窗口打字、点鼠标的工具一律禁止（2026-11-18 voice_chat_real 里她选了打字，
# 字和回车进了桌面）。
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny"}


def call(tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, ENV, tool, args or {}, timeout_ms)


def log_has(text):
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return [line for line in f if text in line]
    except FileNotFoundError:
        return []


def wait(check, seconds, what):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = check()
        if value:
            return value
        time.sleep(1)
    raise AssertionError(f"timed out waiting for {what}")


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    shutil.copy(r"C:\Windows\System32\PING.EXE", FAKE)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False}}, f)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    game = None
    try:
        time.sleep(8)
        cid = real_deepseek.add_connection(EXE, ENV)
        call("profiles.save", {"profile": {
            "id": "test-game", "displayName": "测试游戏", "processNames": ["kotone-fake-game.exe"],
            "windowTitlePatterns": [], "openChatKey": "Enter", "sendKey": "Enter", "preOpenDelayMs": 0,
            "prePasteDelayMs": 0, "preSendDelayMs": 100, "preferClipboardPaste": True, "hotwords": []}})
        call("settings.update", {"patch": {"assistant": {"connectionId": cid}, "activeProfileId": "test-game"}})
        game = subprocess.Popen([FAKE, "-n", "600", "127.0.0.1"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                creationflags=subprocess.CREATE_NO_WINDOW)
        started = time.time()
        time.sleep(6)
        convs = os.path.join(HOME, "agent", "conversations")
        for text in SAID:
            call("assistant.submit", {"text": text})
            # 等这一轮答完（对话文件里出现新的回答）
            before = sum(1 for _ in glob.glob(os.path.join(convs, "*.json")))
            time.sleep(12)
            print("said:", text, "| conversations:", before)
        wait(lambda: time.time() - started > 25, 60, "the match to last 20 s")
        game.kill()
        game = None
        print("game process killed; waiting for the recap …")
        line = wait(lambda: log_has("recap: test-game written") or log_has("recap: test-game failed")
                    or log_has("recap: test-game 这一局"), 180, "the recap")
        print("log:", line[-1].strip())
        memory_path = os.path.join(HOME, "agent", "workspace", "memory", "games", "test-game.md")
        memory = open(memory_path, encoding="utf-8").read() if os.path.exists(memory_path) else ""
        print("\n== game memory ==\n" + memory)
        notes = []
        for path in glob.glob(os.path.join(convs, "*.json")):
            conv = json.load(open(path, encoding="utf-8"))
            notes += [m for m in conv["messages"] if m.get("role") == "notice" and m.get("recap")]
        print("== recap in the chat ==")
        for n in notes:
            print(n["text"])
            print(n["recap"])
        ok = bool(notes) and "F" in memory and ("石像鬼" in memory or "柱子" in memory)
        print("\nusage today:", json.dumps(call("usage.summary")["byKind"], ensure_ascii=False)[:600])
        print("done:", "PASS" if ok else "CHECK BY HAND")
    finally:
        if game:
            game.kill()
        real_deepseek.delete_connection(EXE, ENV)
        try:
            call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()


if __name__ == "__main__":
    main()
