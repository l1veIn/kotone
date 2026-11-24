"""ADR 039 D3 / D4 real-machine check without opening the game: auto cards and the message pop-up.

The fake LoL client from game_events_real.py (same recording, HTTPS, random token) plays the same timeline:
  0 s Lobby → 8 s ChampSelect → 16 s InProgress (in-game API 404: loading) → 22 s in-game data → 60 s EndOfGame → 70 s Lobby.
Setup as game_events_real.py (debug build with the web UI embedded, `--background`, isolated
KOTONE_HOME=target/auto-card-real/home, the real DeepSeek key in memory only, input tools and real-window tools denied,
speech volume 0) but no automations are switched on and the match recap is off, so the only thing the timeline can
do on its own is post auto cards. KOTONE_POPUP_CAPTURABLE=1 lets this script screenshot the pop-up (in normal use it
is excluded from screen capture). No OS input at all, no clicks: clicking the pop-up is covered by Playwright
(e2e/auto-card.spec.ts) and the Rust unit tests.

Checks:
C0. typed “你好，一句话就行” (agent_send = typed in the chat page) → one real model turn: the conversation the cards go to;
C1. champion select → no auto card;
C2. loading → one “开局战绩” card (ui://kotone/lol/match, key opening:<gameId>) at the end of that conversation, followed by
    the note for her (source auto-card, untrusted data attachment); the messages before it are byte-identical (prefix);
C3. the message pop-up window shows (visible, topmost, WS_EX_NOACTIVATE) and the foreground window did not change;
    a screenshot of just the pop-up goes to target/plugin-shots/round7/message-popup-real.png. If Windows reports a
    fullscreen app in front (the pop-up is then skipped by design) this is reported, not failed;
C4. EndOfGame → one “这局总结” card (ui://kotone/lol/game, justEnded) within 60 s;
C5. back in the lobby: still exactly two cards (no repeats);
C6. usage.jsonl: posting the cards added no entries at all (no model call, nothing spent);
C7. typed “刚才英雄联盟发来的卡片说了什么？一句话” → her next turn (one more model call) answers from the notes.
usage: python -X utf8 -I scripts/manual-e2e/auto_card_real.py [--probe DIR] [--gamedata DIR]
"""
import ctypes
import ctypes.wintypes as wt
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import game_events_real as ge  # noqa: E402
import real_deepseek  # noqa: E402

lt, bt = ge.lt, ge.bt
bt.WORK = os.path.join(lt.REPO, "target", "auto-card-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9343
bt.guard = lambda: None  # 不发任何系统按键、不点
check = bt.check
SHOTS = os.path.join(lt.REPO, "target", "plugin-shots", "round7")
user32 = ctypes.WinDLL("user32", use_last_error=True)
user32.FindWindowW.restype = wt.HWND
user32.GetForegroundWindow.restype = wt.HWND
user32.GetWindowLongPtrW.restype = ctypes.c_ssize_t
user32.GetWindowLongPtrW.argtypes = [wt.HWND, ctypes.c_int]
GWL_EXSTYLE = -20
WS_EX_TOPMOST, WS_EX_NOACTIVATE = 0x00000008, 0x08000000


def log_text():
    try:
        return open(os.path.join(bt.HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return ""


def ledger_count():
    return len(bt.ledger())


def popup_window():
    """(hwnd, visible, rect, exstyle) of the pop-up window (title from tauri.conf.json), or None."""
    hwnd = user32.FindWindowW(None, "Kotone 消息提示")
    if not hwnd:
        return None
    rect = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    return hwnd, bool(user32.IsWindowVisible(hwnd)), (rect.left, rect.top, rect.right, rect.bottom), \
        user32.GetWindowLongPtrW(hwnd, GWL_EXSTYLE)


def describe(hwnd):
    """前台窗口是谁（标题、进程号、看不看得见），排查“Kotone 的窗口在前台”用。"""
    title = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(hwnd, title, 256)
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return f"{title.value[:40]!r} pid {pid.value} visible {bool(user32.IsWindowVisible(hwnd))}"


def cards_in(cid):
    msgs = bt.conversation(cid).get("messages", [])
    return msgs, [(i, m) for i, m in enumerate(msgs) if m.get("role") == "notice" and m.get("card")]


def main():
    shutil.rmtree(bt.WORK, ignore_errors=True)
    os.makedirs(bt.HOME)
    os.makedirs(SHOTS, exist_ok=True)
    # exe 先复制一份（别的测试重编 target/debug 时不受影响）
    os.makedirs(os.path.join(bt.WORK, "bin"))
    bt.EXE = shutil.copy2(bt.EXE, os.path.join(bt.WORK, "bin", "kotone.exe"))
    lt.REC = ge.Timeline(lt.PROBE, lt.GAMEDATA)
    server, port = lt.serve()
    bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
                  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}",
                  KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}",
                  KOTONE_POPUP_CAPTURABLE="1")
    bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": lt.DENIED}, "matchRecap": False}}, f)
    print(f"   foreground before Kotone starts: {describe(user32.GetForegroundWindow())}")
    app = subprocess.Popen([bt.EXE, "--background"], env=bt.ENV)
    try:
        bt.wait_ui()
        cid = real_deepseek.add_connection(bt.EXE, bt.ENV, model="deepseek-flash")
        bt.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        bt.invoke("agent_new_conversation", {})

        # C0：先和她说一句，卡片发到这段对话（也用来看前缀）
        hello = "你好，一句话就行"
        bt.invoke("agent_send", {"text": hello})
        main_id = bt.wait_until(lambda: (bt.state().get("current") if not bt.state().get("running") and any(
            m["role"] == "assistant" for m in bt.conversation(bt.state().get("current") or "").get("messages", []))
            else None), 90, 1)
        check("C0: a typed turn gave the conversation the cards go to", main_id, str(bt.state())[:200])
        before = bt.conversation(main_id).get("messages", []) if main_id else []
        # 开局后 5 秒 Kotone 会先合成几句“我看看”之类的等待语（fillers，DeepSeek 语音，每句记一笔）：等账本安静下来再数
        last, quiet_since, started = -1, time.time(), time.time()
        while time.time() - quiet_since < 6 and time.time() - started < 90:
            count = ledger_count()
            if count != last:
                last, quiet_since = count, time.time()
            time.sleep(1)
        spent = ledger_count()
        print(f"   ledger entries before the match: {spent}")

        menu = bt.wait_until(lambda: ge.lol_phase() == "menu", 30, 1)
        check("setup: the fake client is connected (lol in menu)", menu, json.dumps(ge.game_state())[:200])
        foreground = user32.GetForegroundWindow()
        print(f"   foreground before the match: {describe(foreground)} (Kotone pid {app.pid})")
        lt.REC.t0 = time.time()
        bt.wait_until(lambda: ge.lol_phase() == "pre_match", 20, 1)
        time.sleep(4)
        _, picked = cards_in(main_id)
        check("C1: champion select → no auto card", not picked and "auto card:" not in log_text(), str(picked)[:200])

        def opening():
            msgs, cards = cards_in(main_id)
            return (msgs, cards) if cards else None
        got = bt.wait_until(opening, 40, 1)
        check("C2: loading → an auto card in the conversation", got, log_text()[-600:])
        if got:
            msgs, cards = got
            index, card = cards[0]
            info = card["card"]
            check("C2: it is 开局战绩 on ui://kotone/lol/match from lol",
                  info.get("plugin") == "lol" and info.get("title") == "开局战绩"
                  and info.get("ui", {}).get("uri") == "ui://kotone/lol/match" and info.get("key", "").startswith("opening:")
                  and info.get("structuredContent"), json.dumps(info, ensure_ascii=False)[:200])
            note = msgs[index + 1] if index + 1 < len(msgs) else {}
            check("C2: followed by the note for her (auto-card, data attachment)",
                  note.get("role") == "user" and note.get("source") == "auto-card" and note.get("attachments"),
                  json.dumps(note, ensure_ascii=False)[:300])
            print(f"   note: {json.dumps(note.get('attachments'), ensure_ascii=False)[:300]}")
            check("C2: the messages before it are unchanged (prefix)",
                  json.dumps(msgs[:len(before)], ensure_ascii=False) == json.dumps(before, ensure_ascii=False)
                  and index == len(before), f"{index} vs {len(before)}")

        # C3：消息提示
        shown = bt.wait_until(lambda: (lambda w: w if w and w[1] else None)(popup_window()), 10, 0.5)
        skipped = next((line for line in log_text().splitlines() if "消息提示：没弹" in line), None)
        if shown:
            hwnd, _, rect, exstyle = shown
            time.sleep(0.6)  # 滑入动画 180 ms
            check("C3: the pop-up is topmost and never activates (WS_EX_TOPMOST | WS_EX_NOACTIVATE)",
                  exstyle & WS_EX_TOPMOST and exstyle & WS_EX_NOACTIVATE, hex(exstyle))
            check("C3: the foreground window did not change", user32.GetForegroundWindow() == foreground,
                  f"{foreground} → {user32.GetForegroundWindow()}")
            from PIL import ImageGrab
            path = os.path.join(SHOTS, "message-popup-real.png")
            ImageGrab.grab(bbox=rect, all_screens=True).save(path)
            print(f"   pop-up at {rect}, screenshot {path}")
        elif skipped:
            print(f"   pop-up skipped by design: {skipped}")
            check("C3: the foreground window did not change", user32.GetForegroundWindow() == foreground, "")
        else:
            check("C3: the message pop-up showed", False, log_text()[-800:])

        def summary():
            _, cards = cards_in(main_id)
            return [c for _, c in cards if c["card"].get("title") == "这局总结"] or None
        done = bt.wait_until(summary, 120, 2)
        check("C4: EndOfGame → one 这局总结 card on ui://kotone/lol/game with justEnded",
              done and len(done) == 1 and done[0]["card"]["ui"]["uri"] == "ui://kotone/lol/game"
              and done[0]["card"]["structuredContent"].get("justEnded") is True,
              json.dumps(done, ensure_ascii=False)[:300] if done else log_text()[-800:])
        if done:
            print(f"   summary note: {json.dumps(bt.conversation(main_id)['messages'][-1].get('attachments'), ensure_ascii=False)[:300]}")
        bt.wait_until(lambda: ge.lol_phase() == "menu", 30, 1)
        time.sleep(4)
        _, cards = cards_in(main_id)
        check("C5: exactly two cards in the match", len(cards) == 2, [c["card"]["key"] for _, c in cards])
        first_card = min((c["atMs"] for _, c in cards), default=0)
        late = [e for e in bt.ledger() if first_card and e.get("atMs", 0) >= first_card]
        check("C6: posting the cards made no model call or anything else billed (usage.jsonl unchanged)",
              ledger_count() == spent and not late, f"{spent} → {ledger_count()}; after the first card: {late}")

        # C7：她下一轮看得到
        ask = "刚才英雄联盟发来的卡片说了什么？一句话"
        bt.invoke("agent_send", {"text": ask})
        bt.wait_until(lambda: not bt.state().get("running") and bt.conversation(main_id)["messages"][-1]["role"] == "assistant"
                      and any(m.get("text") == ask for m in bt.conversation(main_id)["messages"]), 120, 1)
        msgs = bt.conversation(main_id)["messages"]
        reply = next((m.get("text", "") for m in reversed(msgs) if m["role"] == "assistant"), "")
        print(f"   she said: {reply[:200]}")
        check("C7: her next turn answers from the cards (one more model call)",
              reply and ledger_count() > spent and any(w in reply for w in ("开局", "总结", "突破口", "赢", "输", "战绩")),
              reply[:200])
        lines = [line for line in log_text().splitlines() if "auto card" in line or "消息提示" in line or "自动卡片" in line]
        print("   log:\n     " + "\n     ".join(lines[:12]))
    finally:
        real_deepseek.delete_connection(bt.EXE, bt.ENV)
        try:
            bt.call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
        bt.sweep_credentials()
        server.shutdown()
        if lt.MISSES:
            print("not served:", dict(lt.MISSES))
    return bt.finish()


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
