"""ADR 043 round B item 3 real-machine check without opening the game: the “今天的状态” push in the lobby, the match
summary push with every player's augments, and the today card updated in place after the game.

The fake client from lol_tools_real.py (recording target/lol-probe/20261123-210745: a real ARAM Mayhem game, queue
2400, whose match detail has every player's `playerAugment1..6`) plays a timeline:
  Lobby (until the test starts the clock) → 0 s Lobby → 6 s ChampSelect → 12 s InProgress (in-game data from 16 s)
  → 30 s EndOfGame → 40 s Lobby.
My own match history is the recorded one with the dates moved: the 5 newest games are today (local time; one of them
is a custom game, which does not count), the rest yesterday or earlier; the two losses of today are both 亚索 (157).
The game that is played on the timeline (the session's gameId) shows up at the top of my history only 18 s after
EndOfGame, i.e. after the player is back in the lobby: the plugin has to wait for it before it updates the card.
Setup as game_events_real.py: debug build with the web UI embedded (target/debug/kotone.exe), `--background`,
isolated KOTONE_HOME=target/lol-postgame-real/home and WebView2 folder, KOTONE_LOL_LCU / KOTONE_LOL_LIVE point at the
fake client. No DeepSeek key (pushes never call the model), no OS input at all, the floating window's popups off.

Checks:
P1. lobby → one “今天的状态” push in the chat (trigger today, key today:<local date>, page ui://kotone/lol/today):
    4 games today, 2W 2L, a 2-game win streak, 亚索 lost twice; no note for her (no hidden message about it);
P2. EndOfGame → the “这局总结” push (page ui://kotone/lol/game, justEnded) carries every player's 4 augments with
    names from the client's game data;
P3. back in the lobby → the same today card is updated in place once the new game is in my history: still one today
    push, same key, `updatedAtMs`, 5 games; the log shows it was posted then updated.
usage: python -X utf8 -I scripts/manual-e2e/lol_postgame_real.py [--probe DIR] [--gamedata DIR]
"""
import datetime
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lol_tools_real as lt  # noqa: E402

bt = lt.bt
bt.WORK = os.path.join(lt.REPO, "target", "lol-postgame-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9347
check = bt.check

TIMELINE = [(0, "Lobby"), (6, "ChampSelect"), (12, "InProgress"), (30, "EndOfGame"), (40, "Lobby")]
LIVE_FROM, LIVE_UNTIL = 16, 30
# 这一局在“我的战绩”里出现的时刻（结算后 18 秒，已经回到房间了）
LISTED_AT = 48
YASUO = 157


def local_midnight_ms():
    now = datetime.datetime.now()
    return int(datetime.datetime(now.year, now.month, now.day).timestamp() * 1000)


class Timeline(lt.Recording):
    """The recording, played as a lobby → match → lobby, with my history moved to today."""

    def __init__(self, probe, gamedata):
        super().__init__(probe, gamedata)
        self.t0 = None
        self.game_id = self.session["gameData"]["gameId"]
        own = lt.read(os.path.join(probe, "lcu-match-history-self.json"))
        games = own["games"]["games"]
        midnight = local_midnight_ms()
        # 今天 5 局（新的在前，第 2 局是自定义，不算）：赢、自定义、赢、输、输（两局输的都换成亚索）；其余是昨天以前的
        for i, g in enumerate(games):
            if i < 5:
                g["gameCreation"] = midnight + (5 - i) * 1000
            else:
                g["gameCreation"] = midnight - i * 3_600_000
        wins = [g["participants"][0]["stats"]["win"] for g in games[:5]]
        assert wins[0] and wins[2] and not wins[3] and not wins[4], wins
        for g in games[3:5]:
            g["participants"][0]["championId"] = YASUO
        self.history = own
        detail = self.game_detail
        me = self.me["puuid"]
        pid = next(p["participantId"] for p in detail["participantIdentities"] if p["player"]["puuid"] == me)
        mine = next(p for p in detail["participants"] if p["participantId"] == pid)
        # 打完的这一局（session 的 gameId）：赢没赢、英雄照详情里我的那一行
        self.new_game = json.loads(json.dumps(games[0]))
        self.new_game.update({"gameId": self.game_id, "gameCreation": midnight + 10_000, "gameType": "MATCHED_GAME",
                              "gameDuration": detail["gameDuration"]})
        self.new_game["participants"][0]["championId"] = mine["championId"]
        self.new_game["participants"][0]["stats"]["win"] = mine["stats"]["win"]
        self.new_win = mine["stats"]["win"]

    def elapsed(self):
        return -1 if self.t0 is None else time.time() - self.t0

    def phase(self):
        t = self.elapsed()
        if t < 0:
            return "Lobby"
        return [name for at, name in TIMELINE if t >= at][-1]

    def my_history(self):
        h = json.loads(json.dumps(self.history))
        if self.elapsed() >= LISTED_AT:
            h["games"]["games"].insert(0, self.new_game)
        return h

    def lcu(self, path):
        bare = path.partition("?")[0]
        if bare == "/lol-gameflow/v1/gameflow-phase":
            return self.phase()
        if bare == f"/lol-match-history/v1/products/lol/{self.me['puuid']}/matches":
            return self.my_history()
        if bare == "/lol-champ-select/v1/session" and self.phase() != "ChampSelect":
            return None
        return super().lcu(path)

    def live_data(self, path):
        t = self.elapsed()
        if path.partition("?")[0] != "/liveclientdata/allgamedata" or not (LIVE_FROM <= t < LIVE_UNTIL):
            return None
        data = json.loads(json.dumps(self.live))
        data["gameData"]["gameTime"] = 1.0 + t - LIVE_FROM
        return data


def log_text():
    try:
        return open(os.path.join(bt.HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return ""


def messages():
    cid = bt.state().get("current") or ""
    return bt.conversation(cid).get("messages", []) if cid else []


def pushes(trigger):
    return [m["push"] for m in messages() if m.get("role") == "notice" and (m.get("push") or {}).get("trigger") == trigger]


def data(push):
    return ((push or {}).get("card") or {}).get("structuredContent") or {}


def main():
    lt.no_game_running()
    shutil.rmtree(bt.WORK, ignore_errors=True)
    os.makedirs(bt.HOME)
    # exe 先复制一份（别的测试重编 target/debug 时不受影响）
    os.makedirs(os.path.join(bt.WORK, "bin"))
    bt.EXE = shutil.copy2(bt.EXE, os.path.join(bt.WORK, "bin", "kotone.exe"))
    lt.REC = Timeline(lt.PROBE, lt.GAMEDATA)
    server, port = lt.serve()
    bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
                  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}",
                  KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}")
    bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "autoAdminPromptDismissed": True,
                   "runAsAdminOnStart": False, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}, "bubble": {"enabled": False}},
                   # 推送不弹悬浮窗的小窗：这台电脑上可能有人在用
                   "agent": {"autoCards": {"popup": False},
                             "permissions": {"mode": "allow-all", "rules": lt.DENIED}}}, f)
    app = subprocess.Popen([bt.EXE, "--background"], env=bt.ENV)
    today = datetime.date.today().isoformat()
    try:
        bt.wait_ui()
        # P1：在房间里，今天的状态
        first = bt.wait_until(lambda: (pushes("today") or [None])[0], 40, 1)
        d = data(first)
        lost = d.get("lostOften") or []
        check("P1: lobby → the today card is in the chat (key today:<local date>, page ui://kotone/lol/today)",
              first and first["key"] == f"today:{today}" and first["card"]["ui"]["uri"] == "ui://kotone/lol/today",
              json.dumps(first, ensure_ascii=False)[:300] if first else [l for l in log_text().splitlines() if "lol:" in l][-5:])
        check("P1: 4 games today (the custom game doesn't count), 2W 2L, a 2-game win streak, 亚索 lost twice",
              (d.get("games"), d.get("wins"), d.get("losses"), d.get("streak")) == (4, 2, 2, 2)
              and len(lost) == 1 and lost[0]["champion"]["id"] == YASUO and lost[0]["losses"] == 2,
              json.dumps({k: d.get(k) for k in ("games", "wins", "losses", "streak", "lostOften", "modes")}, ensure_ascii=False)[:400])
        notes = [m for m in messages() if m.get("role") == "user" and m.get("source") == "auto-card" and "今天的状态" in m.get("text", "")]
        check("P1: no note for her about the today card", not notes, json.dumps(notes, ensure_ascii=False)[:200])
        print(f"   today card: {json.dumps({k: d.get(k) for k in ('games', 'wins', 'losses', 'streak', 'modes')}, ensure_ascii=False)}")

        # P2：一局打完，这局总结带每个人的海克斯
        lt.REC.t0 = time.time()
        summary = bt.wait_until(lambda: (pushes("summary") or [None])[0], 90, 1)
        sd = data(summary)
        players = [p for t in sd.get("teams") or [] for p in t.get("players") or []]
        enemies = [p for t in sd.get("teams") or [] if not t.get("ally") for p in t.get("players") or []]
        named = all(len(p.get("augments") or []) == 4 and all(a.get("name") for a in p["augments"]) for p in players)
        check("P2: EndOfGame → the summary card (page ui://kotone/lol/game, justEnded) has 4 named augments per player",
              summary and summary["card"]["ui"]["uri"] == "ui://kotone/lol/game" and sd.get("justEnded")
              and len(players) == 10 and len(enemies) == 5 and named,
              json.dumps([[a.get("name") for a in p.get("augments") or []] for p in enemies], ensure_ascii=False)[:400]
              if summary else [l for l in log_text().splitlines() if "lol:" in l][-5:])
        if enemies:
            print("   enemy augments: " + " | ".join("、".join(a["name"] for a in p["augments"]) for p in enemies))

        # P3：回到房间，等这一局出现在战绩里，同一张卡片原地更新
        updated = bt.wait_until(lambda: next((p for p in pushes("today") if data(p).get("games") == 5), None), 90, 1)
        cards = pushes("today")
        ud = data(updated)
        streak = ud.get("streak")
        check("P3: back in the lobby → the same today card updated in place (one card, same key, updatedAtMs, 5 games)",
              updated and len(cards) == 1 and updated["key"] == first["key"] and updated.get("updatedAtMs")
              and streak == (3 if lt.REC.new_win else -1),
              json.dumps([{k: data(p).get(k) for k in ("games", "streak")} | {"key": p["key"], "updatedAtMs": p.get("updatedAtMs")}
                          for p in cards], ensure_ascii=False)[:300])
        waited = time.time() - lt.REC.t0
        check("P3: the update waited for the new game to show up in my history", waited >= LISTED_AT - 1, f"{waited:.0f} s")
        lines = [line for line in log_text().splitlines() if "lol:" in line and ("推送" in line or "今天" in line or "总结" in line)]
        lines += [line for line in log_text().splitlines() if "push:" in line and ("today" in line or "summary" in line)]
        print("   log:\n     " + "\n     ".join(lines[-14:]))
    finally:
        try:
            bt.call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
        server.shutdown()
        if lt.MISSES:
            print("not served:", dict(lt.MISSES))
    return bt.finish()


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
