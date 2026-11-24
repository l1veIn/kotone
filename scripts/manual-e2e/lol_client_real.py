"""ADR 038 step 3.5 check without opening the game: the LoL plugin's “操作客户端” (like LeagueAkari: auto-accept,
auto pick / ban in the player's order, and `lol.client`), against a fake client that records every write.

The fake client is lol_tools_real.py's HTTPS server (same recording, random token, KOTONE_LOL_LCU) plus a small
champion-select flow: ReadyCheck (POST accept / decline answers it) → ChampSelect with my ban turn, then my pick
turn (PATCH /lol-champ-select/v1/session/actions/{id} completes them) → Lobby. No DeepSeek, no OS input, no real
client: the isolated Kotone (KOTONE_HOME=target/lol-client-real/home, `--background`) only talks to 127.0.0.1.

Three passes on one instance:
A. “操作客户端” OFF (the default) with every preference on: not a single write; `lol.client` is not offered.
B. ON with auto-accept (no delay), ban order [亚索], pick order [an unpickable id, 艾瑞莉娅] with lock-in: exactly one
   accept, one ban PATCH {157, completed} on my ban action, one pick PATCH {39, completed} on my pick action.
C. ON with the automatic preferences off: nothing happens by itself; `lol.client` through the tool service accepts
   the match, hovers 亚索 (no lock), and bans 劫 when it is my ban turn.
usage: python -X utf8 -I scripts/manual-e2e/lol_client_real.py
"""
import json
import os
import shutil
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lol_tools_real as lt  # noqa: E402

bt = lt.bt
bt.WORK = os.path.join(lt.REPO, "target", "lol-client-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9342
bt.guard = lambda: None  # 不发任何系统按键
check = bt.check

ME, ALLY = 2, 1
BAN_ID, PICK_ID = 11, 21
PICKABLE = [39, 103, 157, 238, 64]


class ClientFlow(lt.Recording):
    """ReadyCheck → ChampSelect (my ban, then my pick) → Lobby, replayed by `start()`."""

    def __init__(self, probe, gamedata):
        super().__init__(probe, gamedata)
        self.lock = threading.Lock()
        self.writes = []
        self.reset()

    def reset(self):
        with self.lock:
            self.t0 = None
            self.response = "None"
            self.actions = {BAN_ID: {"type": "ban", "done": False, "champion": 0},
                            PICK_ID: {"type": "pick", "done": False, "champion": 0}}
            self.bans = []

    def start(self):
        self.reset()
        with self.lock:
            self.t0 = time.time()

    def phase(self):
        if self.t0 is None:
            return "Lobby"
        t = time.time() - self.t0
        if t < 10 and self.response == "None":
            return "ReadyCheck"
        if t < 40:
            return "ChampSelect"
        return "Lobby"

    def session_json(self):
        ban, pick = self.actions[BAN_ID], self.actions[PICK_ID]

        def action(aid, a, cell, in_progress):
            return {"id": aid, "actorCellId": cell, "type": a["type"], "completed": a["done"],
                    "isInProgress": in_progress and not a["done"], "championId": a["champion"]}
        return {
            "localPlayerCellId": ME,
            "actions": [
                [action(BAN_ID, ban, ME, True),
                 {"id": 10, "actorCellId": ALLY, "type": "ban", "completed": True, "championId": 64}],
                [action(PICK_ID, pick, ME, ban["done"])],
            ],
            "bans": {"myTeamBans": [64] + self.bans, "theirTeamBans": []},
            "myTeam": [{"cellId": ALLY, "championId": 0, "championPickIntent": 103},
                       {"cellId": ME, "championId": pick["champion"] if pick["done"] else 0, "championPickIntent": 0}],
            "theirTeam": [],
            "benchChampions": [],
        }

    def lcu(self, path):
        p = path.partition("?")[0]
        with self.lock:
            if p == "/lol-gameflow/v1/gameflow-phase":
                return self.phase()
            if p == "/lol-matchmaking/v1/ready-check":
                if self.phase() != "ReadyCheck":
                    return None  # 真客户端过了准备确认也不再给这个
                return {"state": "InProgress", "playerResponse": self.response}
            if p == "/lol-champ-select/v1/session":
                return self.session_json() if self.phase() == "ChampSelect" else None
            if p in ("/lol-champ-select/v1/pickable-champion-ids", "/lol-champ-select/v1/bannable-champion-ids"):
                return PICKABLE
        return super().lcu(path)

    def write(self, method, path, body):
        with self.lock:
            self.writes.append((method, path, body))
            if path == "/lol-matchmaking/v1/ready-check/accept":
                self.response = "Accepted"
            elif path == "/lol-matchmaking/v1/ready-check/decline":
                self.response = "Declined"
            elif path.startswith("/lol-champ-select/v1/session/actions/"):
                aid = int(path.rsplit("/", 1)[1])
                a = self.actions.get(aid)
                if a is None or a["done"]:
                    return 400
                a["champion"] = body.get("championId", a["champion"])
                if body.get("completed"):
                    a["done"] = True
                    if a["type"] == "ban":
                        self.bans.append(a["champion"])
            else:
                return 404
        return 204


def handle_write(self, method):
    path = self.path.partition("?")[0]
    if self.headers.get("Authorization") != lt.AUTH:
        return self.reply(401, {"message": "bad auth"})
    length = int(self.headers.get("Content-Length") or 0)
    raw = self.rfile.read(length) if length else b""
    body = json.loads(raw) if raw else {}
    code = lt.REC.write(method, path, body)
    if code == 204:
        self.send_response(204)
        self.end_headers()
    else:
        self.reply(code, {"message": "rejected"})


lt.Handler.do_POST = lambda self: handle_write(self, "POST")  # noqa: N815
lt.Handler.do_PATCH = lambda self: handle_write(self, "PATCH")  # noqa: N815


def writes():
    with lt.REC.lock:
        return list(lt.REC.writes)


def wait_phase(name, timeout=20):
    return bt.wait_until(lambda: lt.REC.phase() == name, timeout, 0.2)


def set_lol(sources=None, settings=None):
    lol = {}
    if sources is not None:
        lol["sources"] = sources
    if settings is not None:
        lol["settings"] = settings
    bt.call("settings.update", {"patch": {"plugins": {"lol": lol}}})


AUTO = {"autoAccept": True, "acceptDelaySeconds": 0,
        "pick": {"enabled": True, "champions": [9999, 39], "lockIn": True, "delaySeconds": 0},
        "ban": {"enabled": True, "champions": [157], "delaySeconds": 0}}
MANUAL = {"autoAccept": False, "pick": {"enabled": False, "champions": [39]},
          "ban": {"enabled": False, "champions": [157]}}


def main():
    shutil.rmtree(bt.WORK, ignore_errors=True)
    os.makedirs(bt.HOME)
    lt.REC = ClientFlow(lt.PROBE, lt.GAMEDATA)
    server, port = lt.serve()
    bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
                  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}",
                  KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}")
    bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False}},
                   "plugins": {"lol": {"settings": AUTO}}}, f)
    app = subprocess.Popen([bt.EXE, "--background"], env=bt.ENV)
    try:
        bt.wait_ui()
        tools = [t["name"] for t in bt.call("system.capabilities", {}).get("tools", [])]
        check("A: “操作客户端” is off by default, so lol.client is not offered", "lol.client" not in tools,
              [t for t in tools if t.startswith("lol.")])

        # A：开关关着，偏好全开：一个写请求都不能有
        lt.REC.start()
        wait_phase("ChampSelect")
        time.sleep(8)
        check("A: switch off → no write at all (ready check and both champ-select turns passed by)",
              not writes(), writes())

        # B：开关开着、自动接受 / 禁 / 选
        set_lol(sources={"control": True}, settings=AUTO)
        tools = [t["name"] for t in bt.call("system.capabilities", {}).get("tools", [])]
        check("B: with the switch on, lol.client is offered", "lol.client" in tools, "")
        lt.REC.start()
        ok = bt.wait_until(lambda: lt.REC.actions[PICK_ID]["done"], 30, 0.5)
        w = writes()
        accepts = [x for x in w if x[1].endswith("/ready-check/accept")]
        bans = [x for x in w if x[1].endswith(f"/actions/{BAN_ID}")]
        picks = [x for x in w if x[1].endswith(f"/actions/{PICK_ID}")]
        check("B: auto-accept sent exactly one POST accept", len(accepts) == 1 and accepts[0][0] == "POST", accepts)
        check("B: auto-ban sent one PATCH {157, completed} on my ban action",
              [b[2] for b in bans] == [{"championId": 157, "completed": True}], bans)
        check("B: auto-pick skipped the unpickable id and locked 39 once",
              ok and [p[2] for p in picks] == [{"championId": 39, "completed": True}], picks)
        check("B: nothing else was written", len(w) == 3, w)
        log = open(os.path.join(bt.HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
        check("B: the log says what it did", "自动接受了对局" in log and "自动禁了英雄 157" in log
              and "自动选并锁定了英雄 39" in log, [l for l in log.splitlines() if "lol: 自动" in l][-3:])

        # C：开关开着、自动的偏好关着：自己不动；lol.client 经工具服务替玩家做
        lt.REC.writes.clear()
        set_lol(settings=MANUAL)
        lt.REC.start()
        time.sleep(4)
        check("C: with the automatic preferences off nothing happens by itself", not writes(), writes())
        r = bt.call("lol.client", {"action": "accept"})
        check("C: lol.client accept", (r.get("content") or {}).get("status") == "ok"
              and any(x[1].endswith("/ready-check/accept") for x in writes()), r)
        wait_phase("ChampSelect")
        time.sleep(2.5)
        r = bt.call("lol.client", {"action": "hover", "champion": "亚索"})
        hover = [x for x in writes() if x[0] == "PATCH"]
        check("C: lol.client hover 亚索 → PATCH {157} without completing",
              (r.get("content") or {}).get("status") == "ok" and hover and hover[-1][2] == {"championId": 157}, (r, hover))
        r = bt.call("lol.client", {"action": "ban", "champion": "劫"})
        ban = [x for x in writes() if x[1].endswith(f"/actions/{BAN_ID}")]
        check("C: lol.client ban 劫 → PATCH {238, completed} on my ban action",
              ban and ban[-1][2] == {"championId": 238, "completed": True}, (r, ban))
        r = bt.call("lol.client", {"action": "accept"})
        check("C: accept outside a ready check says so and writes nothing",
              (r.get("content") or {}).get("status") == "not_now", r)
    finally:
        try:
            bt.call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.kill()
        server.shutdown()
    return bt.finish()


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
