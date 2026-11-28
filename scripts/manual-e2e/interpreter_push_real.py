"""Real-model check of the interpreter's push (同传, ADR 043 D11) on an isolated Kotone, with the real DeepSeek key.

The interpreter plugin's page says the start sentence for the player (`interpreter.start`: “帮我盯着队友说的话，当同传：听
kotone-speaker.exe，每句外语翻成中文，用气泡显示。”); the real DeepSeek 琴音 must hand it to a background task, which listens
to kotone-speaker.exe (a renamed powershell.exe playing an English SAPI sentence twice, volume 40) and, for each sentence,
calls the plugin's `interpreter.show` (the plugin rule says so) instead of only `danmaku`. Expected: the main conversation
gets ONE “这局的同传” push card holding the translated lines (Chinese), the heard English and the program; the push's line
(the floating window's bubble) is the latest translation; no note for her. Then the page's “停止” stops it.

Isolated: KOTONE_HOME=target/interp-e2e/home (shared with interpreter_e2e.py; SenseVoice is already there), its own
WebView2 data; the key stays in memory (real_deepseek.py) and the test connection is deleted afterwards. NO keyboard or
mouse input; typing tools denied. Plays sound on the default speakers: stops when a game is in front.
usage: python -X utf8 -I scripts/manual-e2e/interpreter_push_real.py
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import interpreter_e2e as base  # noqa: E402
import real_deepseek  # noqa: E402

NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny"}


def main():
    base.guard()
    shutil.rmtree(os.path.join(base.HOME, "agent"), ignore_errors=True)
    shutil.rmtree(os.path.join(base.WORK, "webview"), ignore_errors=True)
    os.makedirs(base.HOME, exist_ok=True)
    with open(os.path.join(base.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"toolService": {"enabled": True}, "ui": {"firstRunCompleted": True}, "adminPromptDismissed": True,
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False},
                   "plugins": {"interpreter": {"settings": {"app": base.SPEAKER, "target": "中文"}}}}, f)
    app = subprocess.Popen([base.EXE, "--background"], env=base.ENV)
    player = None
    try:
        base.wait_until(lambda: "ok" in base.invoke("agent_state", {}), 90, "the settings page")
        cid = real_deepseek.add_connection(base.EXE, base.ENV)
        base.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        speech = os.path.join(base.WORK, "speech.wav")
        base.make_speech(speech)

        t0 = time.time()
        r = base.invoke("plugin_call", {"plugin": "interpreter", "tool": "interpreter.start", "arguments": {}})
        said = ((r.get("ok") or {}).get("content") or {}).get("said", "")
        base.check("page: “开始” said the sentence for the player (气泡 by default)", "用气泡显示" in said, said)
        main_id = base.wait_until(lambda: (base.invoke("agent_state", {}).get("ok") or {}).get("current"), 10, "conversation")

        def task():
            conv = base.invoke("agent_conversation", {"id": main_id}).get("ok") or {}
            return next(iter(conv.get("subagents", [])), None)
        t = base.wait_until(task, 90, "the background task")
        base.check("she handed it to a background task", t, f"{t.get('name')} / {t.get('label')} after {time.time() - t0:.1f} s")
        base.check(f"listening to {base.SPEAKER}", base.wait_until(lambda: f"listens to {base.SPEAKER}" in base.log_text(), 90))
        player = base.start_speaker(speech, 2)

        def shown():
            msgs = (base.invoke("agent_subagent_messages", {"conversationId": main_id, "name": t["name"]}).get("ok")
                    or {}).get("messages", [])
            return [c for m in msgs if m["role"] == "assistant" for c in m.get("toolCalls", [])
                    if c["name"] in ("interpreter.show", "danmaku")]
        calls = base.wait_until(lambda: [c for c in (shown() or []) if c["name"] == "interpreter.show"], 120) or shown() or []
        names = [c["name"] for c in calls]
        base.check("the background used interpreter.show for the translation", "interpreter.show" in names, str(names))
        if calls:
            print("INFO first call:", calls[0]["name"], calls[0]["arguments"][:200])

        def cards():
            conv = base.invoke("agent_conversation", {"id": main_id}).get("ok") or {}
            pushes = [m.get("push") for m in conv.get("messages", []) if m.get("role") == "notice" and m.get("push")]
            return [p for p in pushes if p.get("plugin") == "interpreter" and p.get("trigger") == "lines"]
        found = base.wait_until(cards, 30) or []
        data = ((found[0].get("card") or {}).get("structuredContent") or {}) if found else {}
        lines = data.get("lines") or []
        base.check("one “这局的同传” card in the main conversation", len(found) == 1, f"{len(found)} card(s), {len(lines)} line(s)")
        has_cjk = lambda s: any("一" <= ch <= "鿿" for ch in s)  # noqa: E731
        base.check("its lines are Chinese translations with the English heard",
                   lines and all(has_cjk(line.get("text", "")) for line in lines) and any(line.get("heard") for line in lines),
                   json.dumps(lines, ensure_ascii=False)[:300])
        cell = (((found[0].get("line") or {}).get("cells") or [{}])[0].get("text", "")) if found else ""
        last = lines[-1].get("text", "").strip() if lines else ""
        clipped = last if len(last) <= 40 else last[:39] + "…"  # 宿主截一整句的一行：最多 40 个字
        base.check("the push's line (the bubble) is the latest translation", last and cell == clipped, cell)
        conv = base.invoke("agent_conversation", {"id": main_id}).get("ok") or {}
        notes = [m for m in conv.get("messages", []) if m.get("role") == "user" and m.get("source") == "auto-card"]
        base.check("no note for her", not notes, f"{len(notes)} note(s)")

        base.invoke("plugin_call", {"plugin": "interpreter", "tool": "interpreter.stop", "arguments": {}})
        base.check("page: “停止” stopped the background task",
                   base.wait_until(lambda: (task() or {}).get("status") == "stopped", 90), (task() or {}).get("status"))
        print(f"INFO took {time.time() - t0:.1f} s")
    finally:
        if player:
            try:
                player.wait(timeout=60)
            except Exception:  # noqa: BLE001
                player.kill()
        try:
            real_deepseek.delete_connection(base.EXE, base.ENV)
        except Exception as e:  # noqa: BLE001
            print("delete connection:", e)
        try:
            base.call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
    failed = [name for name, ok in base.RESULTS if not ok]
    print(f"\n{len(base.RESULTS) - len(failed)}/{len(base.RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
