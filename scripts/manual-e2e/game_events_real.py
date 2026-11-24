"""ADR 036 phase 1 real-machine check without opening the game: the game hub, LoL adapter, event-triggered automations
("ask her" with the real DeepSeek model, and a speak automation with event data as args) and the match recap.

The fake client from lol_tools_real.py (same recording, HTTPS, random token) plays a timeline instead of a frozen game:
  0 s  Lobby → 8 s ChampSelect → 16 s InProgress (in-game API answers 404: loading) → 22 s in-game data (game clock
  runs) → 40 s the enemy takes a fire dragon → 60 s EndOfGame (in-game API gone) → 70 s Lobby.
Setup as lol_tools_real.py (debug build with the web UI embedded, `--background`, isolated
KOTONE_HOME=target/game-events-real/home, real DeepSeek key in memory only, input tools and real-window tools denied,
speech volume 0, KOTONE_RECAP_MIN_SECONDS=20). No OS input at all, so it may run while the player is in a match.
Before the timeline starts: the built-in template “英雄联盟进加载界面播报突破口” is switched on (the player's flow) and a
speak automation “对面拿了{dragon_name}” is saved with on {lol.dragon_kill, team enemy}.

Checks:
G1. the hub shows lol in menu (claimed by the LoL adapter), then pre_match; D7 switched activeProfileId to lol;
G2. loading → the template ran: a system message in the conversation, she called lol.scout and said something;
G3. the dragon → the speak automation “对面拿了{dragon_name}” ran as “对面拿了火龙” (event data as args);
G4. game.state lists the recent events (phase changes, lol.dragon_kill);
G5. EndOfGame → game.match_ended → the recap ran for lol (she was asked something during the match).
G6. (ADR 038 step 3) champion select → the LoL plugin asked the overlay layer for the champ-select panel (log only:
    the overlay shows only when the real client window is in front, so nothing is drawn on the player's screen).
usage: python -X utf8 -I scripts/manual-e2e/game_events_real.py [--probe DIR] [--gamedata DIR]
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lol_tools_real as lt  # noqa: E402
import real_deepseek  # noqa: E402

bt = lt.bt
bt.WORK = os.path.join(lt.REPO, "target", "game-events-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9341
bt.guard = lambda: None  # 不发任何系统按键（玩家同意对局中也跑，2026-11-23）
check = bt.check

TIMELINE = [(0, "Lobby"), (8, "ChampSelect"), (16, "InProgress"), (60, "EndOfGame"), (70, "Lobby")]
LIVE_FROM, LIVE_UNTIL, DRAGON_AT = 22, 60, 40


class Timeline(lt.Recording):
    """The recording, played as a match over about a minute."""

    def __init__(self, probe, gamedata):
        super().__init__(probe, gamedata)
        self.t0 = None  # 时间线开始（配置好自动化之后）
        game = self.live
        mine = next((p["team"] for p in game["allPlayers"]
                     if p.get("riotId") == game["activePlayer"].get("riotId")), "ORDER")
        enemy = next(p for p in game["allPlayers"] if p["team"] != mine)
        self.enemy_name = enemy.get("riotIdGameName") or enemy.get("summonerName", "")

    def elapsed(self):
        return -1 if self.t0 is None else time.time() - self.t0

    def phase(self):
        t = self.elapsed()
        if t < 0:
            return "Lobby"
        return [name for at, name in TIMELINE if t >= at][-1]

    def lcu(self, path):
        if path.partition("?")[0] == "/lol-gameflow/v1/gameflow-phase":
            return self.phase()
        return super().lcu(path)

    def live_data(self, path):
        t = self.elapsed()
        if path.partition("?")[0] != "/liveclientdata/allgamedata" or not (LIVE_FROM <= t < LIVE_UNTIL):
            return None  # 加载中 / 不在对局里：404
        data = json.loads(json.dumps(self.live))
        data["gameData"]["gameTime"] = 1.0 + t - LIVE_FROM
        events = data.setdefault("events", {}).setdefault("Events", [])
        if t >= DRAGON_AT:
            events.append({"EventID": 1000, "EventName": "DragonKill", "EventTime": DRAGON_AT - LIVE_FROM,
                           "DragonType": "Fire", "Stolen": "False", "KillerName": self.enemy_name})
        return data


def log_text():
    try:
        return open(os.path.join(bt.HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return ""


def game_state():
    return bt.call("game.state", {"game": "lol"})


def lol_phase():
    playing = (game_state().get("playing") or [])
    return playing[0]["phase"] if playing else None


def main():
    shutil.rmtree(bt.WORK, ignore_errors=True)
    os.makedirs(bt.HOME)
    lt.REC = Timeline(lt.PROBE, lt.GAMEDATA)
    server, port = lt.serve()
    bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
                  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}",
                  KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}",
                  KOTONE_RECAP_MIN_SECONDS="20")
    bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": lt.DENIED}}}, f)
    app = subprocess.Popen([bt.EXE, "--background"], env=bt.ENV)
    try:
        bt.wait_ui()
        cid = real_deepseek.add_connection(bt.EXE, bt.ENV, model="deepseek-flash")
        bt.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        bt.invoke("agent_new_conversation", {})

        # 玩家的做法：打开内置模板；再存一个“对面拿龙就播报”
        template = bt.call("automation.get", {"id": "builtin-lol-scout-on-loading"})
        on = dict(template["on"], enabled=True)
        bt.call("automation.save", {"id": template["id"], "name": template["name"], "description": template["description"],
                                    "action": template["action"], "on": on})
        bt.call("automation.save", {"name": "对面拿龙就说", "params": [{"name": "dragon_name", "type": "text"}],
                                    "action": {"kind": "speak", "text": "对面拿了{dragon_name}", "delivery": ["danmaku"]},
                                    "on": {"event": ["lol.dragon_kill"], "game": "lol", "match": {"team": "enemy"},
                                           "enabled": True}})

        menu = bt.wait_until(lambda: lol_phase() == "menu", 30, 1)
        check("G1: the hub shows lol in the lobby (LoL adapter claimed it)", menu, json.dumps(game_state())[:200])
        lt.REC.t0 = time.time()
        pre = bt.wait_until(lambda: lol_phase() == "pre_match", 20, 1)
        check("G1: champion select → pre_match", pre, str(lol_phase()))
        settings = bt.call("settings.get", {})
        active = (settings.get("settings") or settings).get("activeProfileId")
        check("G1: D7 switched the current game profile to lol", active == "lol" or "自动切到 lol" in log_text(), str(active))

        panel = bt.wait_until(lambda: "ui://kotone/lol/champ-select" in log_text(), 10, 1)
        check("G6: champion select → the LoL plugin asked for the champ-select overlay", panel,
              [l for l in log_text().splitlines() if "覆盖层" in l][-3:])

        loading = bt.wait_until(lambda: lol_phase() == "loading", 20, 1)
        check("G2: InProgress without in-game data → loading", loading, str(lol_phase()))
        main = bt.state().get("current") or ""

        def asked():
            convs = [c["id"] for c in bt.state().get("conversations", [])]
            for cid_ in convs:
                msgs = bt.conversation(cid_).get("messages", [])
                if any(m["role"] == "user" and m.get("source") == "automation" and "game.phase_changed" in m.get("text", "")
                       for m in msgs) and not bt.state().get("running"):
                    return cid_, msgs
            return None
        got = bt.wait_until(asked, 90, 2)
        check("G2: the template handed her the request (system message)", got, "")
        if got:
            _, msgs = got
            tools = [c["name"] for m in msgs if m["role"] == "assistant" for c in m.get("toolCalls", [])]
            # 游戏里的一轮用 speak 开口（ADR 029 §八），正文可能是空的
            spoken = [json.loads(c["arguments"]).get("text", "") for m in msgs if m["role"] == "assistant"
                      for c in m.get("toolCalls", []) if c["name"] == "speak"]
            said = " / ".join(spoken) or next((m.get("text", "") for m in reversed(msgs)
                                               if m["role"] == "assistant" and m.get("text")), "")
            check("G2: she used lol.scout and said something", "lol.scout" in tools and said, f"{tools}: {said[:160]}")
            print(f"   she said: {said[:200]}")

        in_match = bt.wait_until(lambda: lol_phase() == "in_match", 30, 1)
        check("G3: in-game data → in_match", in_match, str(lol_phase()))

        def dragon_run():
            runs = bt.invoke("automation_runs", {}).get("ok", {}).get("runs", [])
            return next((r for r in runs if r["action"].get("kind") == "speak" and "对面拿了" in r["action"].get("text", "")), None)
        run = bt.wait_until(dragon_run, 40, 1)
        # 事件的 dragonName 填进参数 dragon_name（camelCase 字段 = snake_case 参数）
        check("G3: the dragon ran the speak automation with dragon_name = 火龙", run and "火龙" in run["action"]["text"],
              json.dumps(run, ensure_ascii=False)[:200] if run else "no run")
        recent = [e["type"] for e in ((game_state().get("playing") or [{}])[0].get("recent") or [])]
        check("G4: game.state lists recent events", "lol.dragon_kill" in recent and "game.phase_changed" in recent, str(recent))

        ended = bt.wait_until(lambda: "game.match_ended" in log_text() or "recap: lol" in log_text(), 40, 1)
        check("G5: EndOfGame → game.match_ended", ended, "")
        recap = bt.wait_until(lambda: "recap: lol ended" in log_text(), 20, 1)
        check("G5: the recap ran for lol", recap, "")
        outcome = bt.wait_until(lambda: next((line for line in log_text().splitlines()
                                              if "recap: lol" in line and "ended" not in line), None), 90, 2)
        print(f"   recap: {outcome}")
        lines = [line for line in log_text().splitlines() if "game: lol" in line or "automation" in line and "触发" in line]
        print("   hub log:\n     " + "\n     ".join(lines[:20]))
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
