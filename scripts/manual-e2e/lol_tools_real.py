"""ADR 035 real-model check without opening the game: the three LoL tools on a recorded game, the real DeepSeek model.

A small HTTPS server on 127.0.0.1 replays one `lol_probe.py` recording (default target/lol-probe/20261123-210745, a real
ARAM Mayhem game, queue 2400; game data from target/lol-probe/gamedata) as both the client API (LCU: Basic auth with a
random token, phase InProgress, the session, every player's summoner and last 20 games) and the in-game API (Live
Client Data: `allgamedata`, the game clock runs on from the recorded time). The app is pointed at it with
KOTONE_LOL_LCU=<port>:<token> and KOTONE_LOL_LIVE=https://127.0.0.1:<port>, so it never looks for the player's real
client. Otherwise the same setup as music_real.py: debug build with the web UI embedded, `--background`, isolated
KOTONE_HOME=target/lol-tools-real/home, the player's real DeepSeek key in memory only, input tools denied
(game_chat.send included) and so are the tools that touch real game windows (game.focus / launch / close: on
2026-11-23 she called game.focus before game_chat.send and brought the player's running match to the front),
speech volume 0, typed through WebView2's DevTools port (no OS input).

Before typing: the app connected to the fake client and fetched all 10 players' histories in the background.
Typed, in one conversation:
T1. “查一下这把的战绩，对面谁是突破口” → `lol.scout`; the result says the heroes are random (no “没玩过”);
    she names an enemy champion and uses no labels like 牛马 / 下等马.
T2. “对面丽桑卓闪了” → `lol.spell` with champion 丽桑卓 and status ok; she says when it's back.
T3. “谁没闪” → she names 丽桑卓 (from `lol.match`, or from what she just recorded).
T4. “发出去” → she tries `game_chat.send` (denied in this test) with a line naming 丽桑卓 and the flash.
The recording has real player names: it stays under target/ and this printout is for this machine only.
Stops before starting if a game is running or in front (the player plays on this machine). `--while-playing` skips
that check (the player OK'd keyless tests during a match, 2026-11-23): this test sends no OS input at all, only types
through the debugging port with input tools denied; the test instance's hotkey hooks run on their own thread.

C. Send to game in the client (ADR 041 phase 2): the fake client also answers `/lol-chat/v1/conversations`,
`/lol-lobby/v2/lobby` and records every `POST /lol-chat/v1/conversations/<id>/messages`; its phase can be switched.
The isolated config selects the League of Legends game profile, so a send that went the keyboard way for her / an
external Agent would be refused (the foreground is not the game) - nothing below can type into the player's windows.
C0 (always, no OS input, the foreground is not touched): champion select in the fake client, the window in front is
   NOT the client -> `game_chat.send` over the tool service is refused and the fake client gets no message
   (the foreground decides, not the client's phase). Only runs while a terminal / Explorer / VS Code is in front
   (with a game in front the keyboard way would type into it), never with --while-playing.
C1-C7 (only with --chat-front and never with --while-playing: they need the fake client window IN FRONT, so the
   player has to leave the machine):
   a copy of powershell.exe named LeagueClientUx.exe runs target_window.ps1 -Chat (title KotoneScriptTarget; it logs
   every key and Kotone's tag 4B544E49) and is brought to the front with fg.ps1 (that presses Alt once). If the
   window in front is not LeagueClientUx.exe the part is skipped and says so. Before every call the script checks
   the test window is still in front.
   C1 champion select, "第一行\n第二行": route client, plugin lol, channel team, 2/2 lines, ONE POST to the champion
      select conversation with the lines joined by a newline, no Kotone key in the window;
   C2 channel all in champion select: refused ("只有队友看得到"), no POST, no key;
   C3 lobby: one POST to the lobby conversation ("房间的聊天");
   C4 client home (phase None): refused ("进房间或选英雄"), no POST;
   C5 the client never answers the POST (6 s), timeoutMs 3000: back within the deadline, "可能已经发出去了", ONE
      POST (not retried), no key;
   C6 the plugin's "在客户端里发聊天" switch off: refused with the switch named, no POST, no key (she does not type
      into the client);
   C7 send:false: refused ("没法只打进聊天框"), no POST, no key;
   C8 (with --chat-front and without --chat-only, real model): game_chat.send allowed for this turn only, "把对面谁是
      突破口发到聊天里" -> she sends, ONE POST, no key.
S. One-key shout (ADR 041 phase 3: `lol.shout` + the four presets in the plugin's [[commands]]), always, no OS input,
   nothing reaches the player's windows (the parts that would type need a game in front and are not run here):
   S1 `hotkeys.status`: the four presets are rows of the hotkey table (plugin lol, action tool, no key);
   S2 `lol.shout {what: flash, preview: true}`: nothing recorded -> not sent, says how to record (or only "没带闪");
      after `lol.spell` 丽桑卓 the first line is exactly lol.spell's `chat` line;
   S3 records (enemy) and premade previews: one line per enemy starting "对面 ", no player name from the recording;
   S4 (only with a terminal / Explorer / VS Code in front, never --while-playing) a real send (no preview): refused by
      the entry ("请先切到游戏"), the fake client got no message;
   S5 binding F19 to 喊对面闪现 over settings.update: lol.shout is denied in this test's permissions -> the key is not
      registered and the reason names the permission; the binding is removed again.
`--chat-only` skips T1-T6 / P2 / P3 (no DeepSeek key needed) and runs S and C only.
Not covered here: the player's own voice input in the client (needs a microphone or VB-CABLE), a real client.
usage: python -X utf8 -I scripts/manual-e2e/lol_tools_real.py [--probe DIR] [--gamedata DIR] [--while-playing]
       [--chat-only] [--chat-front]
"""
import base64
import collections
import ctypes
import glob
import http.server
import json
import os
import re
import secrets
import shutil
import ssl
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import background_task_real as bt  # noqa: E402
import real_deepseek  # noqa: E402
from voice_turn_real import GAMES  # noqa: E402

REPO = bt.REPO
bt.WORK = os.path.join(REPO, "target", "lol-tools-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9340
check = bt.check


def arg(name, default):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else default


PROBE = arg("--probe", os.path.join(REPO, "target", "lol-probe", "20261123-210745"))
GAMEDATA = arg("--gamedata", os.path.join(REPO, "target", "lol-probe", "gamedata"))
WHILE_PLAYING = "--while-playing" in sys.argv
CHAT_ONLY = "--chat-only" in sys.argv
CHAT_FRONT = "--chat-front" in sys.argv
# 会按键的，加上会动玩家真实游戏窗口的（切到前台、打开、关闭）
DENIED = dict(bt.NO_INPUT, **{"game.focus": "deny", "game.launch": "deny", "game.close": "deny", "lol.shout": "deny"})
TOKEN = secrets.token_urlsafe(18)
AUTH = "Basic " + base64.b64encode(f"riot:{TOKEN}".encode()).decode()
HITS = collections.Counter()
MISSES = collections.Counter()
# 发到游戏（C 部分）：假客户端收到的每个 POST（路径, 请求体）；POST 怎么回：ok / 500 / hang（6 秒以后才回）
POSTS = []
CHAT_POST = {"mode": "ok"}
CHAMP_SELECT_CHAT = "c0ffee00@lol-champ-select.tj100.pvp.net"
LOBBY_CHAT = "10bb7000@lol-lobby.tj100.pvp.net"


def read(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


class Recording:
    """One probe directory served back as the LCU and the Live Client Data API."""

    def __init__(self, probe, gamedata):
        self.session = read(os.path.join(probe, "lcu-gameflow-session.json"))
        self.me = read(os.path.join(probe, "lcu-current-summoner.json"))
        self.live = read(os.path.join(probe, "live-allgamedata.json"))
        self.gamedata = gamedata
        self.players = {}
        for path in glob.glob(os.path.join(probe, "scout-*.json")):
            v = read(path)
            self.players[v["summoner"]["puuid"]] = v
        detail = os.path.join(probe, "lcu-game-detail.json")
        cs = os.path.join(probe, "lcu_lol-champ-select_v1_session.json")
        # 选英雄（ADR 038 第 3 步的选英雄覆盖层）：录到的那一份
        self.champ_select = read(cs) if os.path.exists(cs) else None
        if not self.champ_select or self.champ_select.get("status") == 404:
            # 录的时候不在选英雄（录到的是 404）：用这局的人合成一份（我方 5 个人的英雄 + 两个备选席英雄）
            self.champ_select = self.synth_champ_select()
        # 单局详情（ADR 038 第 2 步的单局页面）：录到的只有一局，任何 gameId 都给这一局
        self.game_detail = read(detail) if os.path.exists(detail) else None
        own = os.path.join(probe, "lcu-match-history-self.json")
        if self.me.get("puuid") not in self.players and os.path.exists(own):
            self.players[self.me["puuid"]] = {"summoner": self.me, "history": read(own)}
        self.recorded_time = self.live["gameData"]["gameTime"]
        self.started = time.time()
        # 对局阶段：T 部分是对局里；C 部分切成选英雄 / 房间 / 首页（set_phase）。phase() 是方法：
        # game_events_real.Timeline、lol_client_real.ClientFlow 按时间线覆盖它，存成属性会把它们的方法盖掉
        self._phase = "InProgress"

    def phase(self):
        return self._phase

    def set_phase(self, name):
        self._phase = name

    def synth_champ_select(self):
        gd = self.session["gameData"]
        me = self.me.get("puuid")
        mine = "teamOne" if any(m.get("puuid") == me for m in gd["teamOne"]) else "teamTwo"
        picks = {x.get("puuid"): x.get("championId") for x in gd.get("playerChampionSelections", [])}
        team = []
        local = 0
        for i, m in enumerate(gd[mine]):
            if m.get("puuid") == me:
                local = i
            team.append({"cellId": i, "championId": m.get("championId") or picks.get(m.get("puuid")) or 0,
                         "championPickIntent": 0, "assignedPosition": "", "puuid": m.get("puuid")})
        return {"localPlayerCellId": local, "myTeam": team, "theirTeam": [], "actions": [],
                "benchChampions": [{"championId": 21}, {"championId": 99}],
                "bans": {"myTeamBans": [], "theirTeamBans": []}, "isCustomGame": False}

    def queue(self):
        return self.session["gameData"]["queue"]["id"]

    def lcu(self, path):
        path, _, query = path.partition("?")
        if path == "/lol-gameflow/v1/gameflow-phase":
            return self.phase()
        if path == "/lol-chat/v1/conversations":
            return [{"id": "friend@pvp.net", "type": "chat"},
                    {"id": CHAMP_SELECT_CHAT, "type": "championSelect"},
                    {"id": LOBBY_CHAT, "type": "customGame"}]
        if path == "/lol-lobby/v2/lobby":
            return {"gameConfig": {"isCustom": False, "queueId": self.queue()}}
        if path == "/lol-gameflow/v1/session":
            return self.session
        if path == "/lol-summoner/v1/current-summoner":
            return self.me
        m = re.fullmatch(r"/lol-game-data/assets/v1/([\w.-]+\.json)", path)
        if m:
            f = os.path.join(self.gamedata, m.group(1))
            return read(f) if os.path.exists(f) else None
        m = re.fullmatch(r"/lol-match-history/v1/products/lol/([^/]+)/matches", path)
        if m and m.group(1) in self.players:
            return self.players[m.group(1)]["history"]
        m = re.fullmatch(r"/lol-summoner/v2/summoners/puuid/([^/]+)", path)
        if m and m.group(1) in self.players:
            return self.players[m.group(1)]["summoner"]
        if re.fullmatch(r"/lol-ranked/v1/ranked-stats/[^/]+", path):
            return {"queues": []}  # 录的时候没录段位
        if path == "/lol-champ-select/v1/session" and self.champ_select is not None:
            return self.champ_select
        if re.fullmatch(r"/lol-match-history/v1/games/\d+", path):
            return self.game_detail
        if path == "/lol-summoner/v1/alias/lookup":
            # 按 名字#编号 找人（页面里的搜索）：在录到的人里找
            from urllib.parse import parse_qs
            q = parse_qs(query)
            name, tag = (q.get("gameName") or [""])[0], (q.get("tagLine") or [""])[0]
            for v in self.players.values():
                s = v["summoner"]
                if s.get("gameName") == name and (not tag or s.get("tagLine") == tag):
                    return s
        return None

    def live_data(self, path):
        if path.partition("?")[0] != "/liveclientdata/allgamedata":
            return None
        data = json.loads(json.dumps(self.live))
        data["gameData"]["gameTime"] = self.recorded_time + time.time() - self.started
        return data

    def champion_names(self):
        """{championId: 名字}, enemies of the player first (from the session)."""
        names = {c["id"]: c.get("description") or c.get("name") for c in read(os.path.join(self.gamedata, "champion-summary.json"))}
        gd = self.session["gameData"]
        mine = "teamOne" if any(m.get("puuid") == self.me.get("puuid") for m in gd["teamOne"]) else "teamTwo"
        theirs = "teamTwo" if mine == "teamOne" else "teamOne"
        return [names.get(m.get("championId"), "?") for m in gd[theirs]], [names.get(m.get("championId"), "?") for m in gd[mine]]


REC = None


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        path = self.path.partition("?")[0]
        kind = re.sub(r"/[0-9A-Za-z_-]{30,}", "/<puuid>", path)
        if path.startswith("/liveclientdata/"):
            body = REC.live_data(path)
        elif self.headers.get("Authorization") != AUTH:
            MISSES["401 " + kind] += 1
            return self.reply(401, {"message": "bad auth"})
        else:
            body = REC.lcu(self.path)
        if body is None:
            MISSES["404 " + kind] += 1
            return self.reply(404, {"message": "not in the recording"})
        HITS[kind] += 1
        self.reply(200, body)

    def do_POST(self):  # noqa: N802
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode("utf-8", errors="replace")
        if self.headers.get("Authorization") != AUTH:
            MISSES["401 POST"] += 1
            return self.reply(401, {"message": "bad auth"})
        if not re.fullmatch(r"/lol-chat/v1/conversations/[^/]+/messages", self.path):
            MISSES["404 POST " + self.path] += 1
            return self.reply(404, {"message": "not in the recording"})
        POSTS.append((self.path, body))
        if CHAT_POST["mode"] == "hang":
            time.sleep(6)
        if CHAT_POST["mode"] == "500":
            return self.reply(500, {"message": "chat restricted"})
        self.reply(200, {"id": str(len(POSTS)), "type": "chat"})

    def reply(self, code, body):
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def openssl():
    for path in (shutil.which("openssl"), r"C:\Program Files\Git\mingw64\bin\openssl.exe",
                 r"C:\Program Files\Git\usr\bin\openssl.exe"):
        if path and os.path.exists(path):
            return path
    raise SystemExit("openssl not found (Git for Windows ships one)")


def serve():
    """Start the fake client on a free port; returns (server, port)."""
    cert, key = os.path.join(bt.WORK, "cert.pem"), os.path.join(bt.WORK, "key.pem")
    subprocess.run([openssl(), "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", cert,
                    "-days", "2", "-subj", "/CN=127.0.0.1"], check=True, capture_output=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, server.server_address[1]


def no_game_running():
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, encoding="mbcs",
                         errors="replace").stdout.lower()
    running = [g for g in ("league of legends.exe", "valorant", "cs2.exe", "dota2.exe") if g in out]
    if running:
        raise SystemExit(f"STOP: a game is running ({running}); not testing while the player plays")


def turn(main, text):
    """The messages of the turn that starts with the player's `text`."""
    msgs = bt.conversation(main).get("messages", []) if main else []
    start = max((i for i, m in enumerate(msgs) if m["role"] == "user" and m.get("text") == text), default=0)
    return msgs[start:]


def calls(msgs, name=None):
    return [c for m in msgs if m["role"] == "assistant" for c in m.get("toolCalls", [])
            if name is None or c["name"] == name]


def results(msgs, name):
    return [m.get("content", "") for m in msgs if m["role"] == "tool" and m.get("name") == name]


def displays(msgs, name):
    """ADR 038: the page data stored with a plugin tool result (chat card), never sent to the model."""
    return [m.get("display") for m in msgs if m["role"] == "tool" and m.get("name") == name and m.get("display")]


def say(name, text):
    r = bt.run_case(name, text, answer="deny", timeout=180)
    msgs = turn(r.get("main"), text)
    tools = [c["name"] for c in calls(msgs)]
    print(f"   [{name}] main tools: {tools}")
    return r, msgs


# ---------- C：发到游戏（ADR 041 第 2 期） ----------

user32 = ctypes.WinDLL("user32", use_last_error=True)
KOTONE_TAG = "4B544E49"
CLIENT_DIR = os.path.join(bt.WORK, "client-window")
SAFE_FOREGROUND = {"powershell.exe", "pwsh.exe", "windowsterminal.exe", "openconsole.exe", "conhost.exe",
                   "explorer.exe", "code.exe", "claude.exe", "python.exe", "warp.exe", "leagueclientux.exe", ""}


def front():
    """(进程名, 窗口标题) of the window in front."""
    path = bt.foreground_process()
    title = ctypes.create_unicode_buffer(256)
    user32.GetWindowTextW(user32.GetForegroundWindow(), title, 256)
    return os.path.basename(path), title.value


def chat_send(args, timeout_ms=8000):
    """game_chat.send over the tool service (caller = external Agent): (ok, result or error text, seconds)."""
    started = time.time()
    try:
        result = bt.call("game_chat.send", args, timeout_ms)
        # the tool service wraps a tool's result in {"content": ...} (as lol.shout / lol.spell below)
        if isinstance(result, dict) and isinstance(result.get("content"), dict):
            result = result["content"]
        return True, result, time.time() - started
    except Exception as e:  # noqa: BLE001
        return False, str(e), time.time() - started


def kotone_keys():
    """Key downs carrying Kotone's tag that reached the fake client window."""
    try:
        with open(os.path.join(CLIENT_DIR, "events.log"), encoding="utf-8") as f:
            return [l for l in f.read().splitlines() if " DOWN " in f" {l} " and f"x={KOTONE_TAG}" in l]
    except FileNotFoundError:
        return []


def chat_c0():
    """C0: the window in front is not the client -> nothing goes through the client, nothing is typed."""
    process, title = front()
    # 只在终端、资源管理器这类窗口在前台时跑：前台要是游戏，按键那条路会往游戏里打字（选中的是英雄联盟）
    if WHILE_PLAYING or process.lower() not in SAFE_FOREGROUND or process.lower() == "leagueclientux.exe":
        print(f"   C0 skipped: '{title}' ({process}) is in front{' (--while-playing)' if WHILE_PLAYING else ''}")
        return
    REC.set_phase("ChampSelect")
    before = len(POSTS)
    ok, out, _ = chat_send({"text": "C0 这句不该发出去"})
    # 前台是终端时是“不是「League of Legends」”；是 Kotone 自己 / 桌面时是那两句：都是没发
    check("C0: champion select but the client is not in front: game_chat.send is refused (the foreground decides)",
          not ok and "请先切到游戏" in str(out), f"front {process}: {str(out)[:200]}")
    check("C0: the fake client got no message", len(POSTS) == before, str(POSTS[before:])[:200])


def client_window():
    """A copy of powershell.exe named LeagueClientUx.exe showing target_window.ps1 -Chat. None = could not get it
    in front as LeagueClientUx.exe (the part is skipped)."""
    bin_dir = os.path.join(bt.WORK, "bin")
    os.makedirs(bin_dir, exist_ok=True)
    exe = os.path.join(bin_dir, "LeagueClientUx.exe")
    shutil.copy2(os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "WindowsPowerShell", "v1.0",
                              "powershell.exe"), exe)
    window = subprocess.Popen([exe, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                               os.path.join(HERE, "target_window.ps1"), "-Dir", CLIENT_DIR, "-Chat"])
    if not bt.wait_until(lambda: os.path.exists(os.path.join(CLIENT_DIR, "ready.txt")), 30, 0.5):
        print("   C: the fake client window did not come up")
        return window, False
    if front() != ("LeagueClientUx.exe", "KotoneScriptTarget"):
        process, title = front()
        if process.lower() not in SAFE_FOREGROUND and title != "KotoneScriptTarget":
            raise SystemExit(f"STOP: '{title}' ({process}) is in front - not touching it.")
        subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                        os.path.join(HERE, "fg.ps1"), "-Title", "KotoneScriptTarget"], capture_output=True, text=True)
        time.sleep(0.4)
    in_front = front() == ("LeagueClientUx.exe", "KotoneScriptTarget")
    if not in_front:
        print(f"   C1-C7 skipped: the window in front is {front()}, not the fake client (LeagueClientUx.exe)")
    return window, in_front


def client_in_front():
    if front() != ("LeagueClientUx.exe", "KotoneScriptTarget"):
        raise SystemExit(f"STOP: the fake client window is not in front ({front()}) - not sending.")


def chat_front(with_model):
    """C1-C8: the fake client window in front."""
    window, in_front = client_window()
    try:
        if not in_front:
            return
        # C1 选英雄：一条消息、几行用换行连起来、不按键
        REC.set_phase("ChampSelect")
        client_in_front()
        before = len(POSTS)
        ok, out, _ = chat_send({"text": "第一行\n第二行"})
        out = out if ok else {}
        check("C1: champion select: route client, plugin lol, team, 2/2 lines",
              ok and out.get("route") == "client" and out.get("plugin") == "lol" and out.get("channel") == "team"
              and out.get("sent") is True and out.get("sentLines") == 2, json.dumps(out, ensure_ascii=False)[:300])
        check("C1: the receipt says only the client took it", "只证明客户端收下了" in str(out.get("receipt")),
              str(out.get("receipt")))
        new = POSTS[before:]
        check("C1: ONE message to the champion select chat, the lines joined by a newline",
              len(new) == 1 and CHAMP_SELECT_CHAT in new[0][0]
              and json.loads(new[0][1]) == {"body": "第一行\n第二行", "type": "chat"}, str(new)[:300])
        check("C1: no Kotone key reached the window", not kotone_keys(), str(kotone_keys()[:5]))

        # C2 选英雄要“所有人”：不发
        client_in_front()
        before = len(POSTS)
        ok, out, _ = chat_send({"text": "gg", "channel": "all"})
        check("C2: all in champion select is refused", not ok and "只有队友看得到" in str(out), str(out)[:200])
        check("C2: no message, no key", len(POSTS) == before and not kotone_keys())

        # C3 房间
        REC.set_phase("Lobby")
        client_in_front()
        before = len(POSTS)
        ok, out, _ = chat_send({"text": "我先排位"})
        new = POSTS[before:]
        check("C3: the lobby chat", ok and (out or {}).get("channelName") == "房间的聊天" and len(new) == 1
              and LOBBY_CHAT in new[0][0], f"{out} {new}"[:300])

        # C4 首页：不发
        REC.set_phase("None")
        client_in_front()
        before = len(POSTS)
        ok, out, _ = chat_send({"text": "gg"})
        check("C4: client home: refused, says to go into a lobby or champion select",
              not ok and "进房间或选英雄" in str(out) and len(POSTS) == before, str(out)[:200])

        # C5 客户端不回话：期限内返回、说可能已经发出去了、只发一次
        REC.set_phase("ChampSelect")
        CHAT_POST["mode"] = "hang"
        client_in_front()
        before = len(POSTS)
        ok, out, took = chat_send({"text": "gg"}, timeout_ms=3000)
        CHAT_POST["mode"] = "ok"
        check("C5: no answer from the client: back within the deadline, says it may have gone out",
              not ok and "可能已经发出去了" in str(out) and took < 4.5, f"{took:.1f}s {str(out)[:160]}")
        check("C5: ONE POST (not retried), no key", len(POSTS) == before + 1 and not kotone_keys(), str(POSTS[before:]))

        # C6 关掉“在客户端里发聊天”：她不往客户端里按键
        bt.call("settings.update", {"patch": {"plugins": {"lol": {"sources": {"chat": False}}}}})
        client_in_front()
        before = len(POSTS)
        ok, out, _ = chat_send({"text": "gg"})
        bt.call("settings.update", {"patch": {"plugins": {"lol": {"sources": {"chat": True}}}}})
        check("C6: the switch off: refused with the switch named",
              not ok and "在客户端里发聊天" in str(out) and "关着" in str(out), str(out)[:200])
        check("C6: no message, no key", len(POSTS) == before and not kotone_keys())

        # C7 只打进聊天框不发：这里做不到，不发
        client_in_front()
        before = len(POSTS)
        ok, out, _ = chat_send({"text": "先别发", "send": False})
        check("C7: send:false is refused here", not ok and "没法只打进聊天框" in str(out) and len(POSTS) == before,
              str(out)[:200])

        # C8 真模型：选英雄里让她发（这一轮放开 game_chat.send）
        if with_model:
            rules = dict(DENIED, **{"game_chat.send": "allow"})
            bt.call("settings.update", {"patch": {"agent": {"permissions": {"mode": "allow-all", "rules": rules}}}})
            client_in_front()
            before = len(POSTS)
            text = "把对面谁是突破口发到聊天里"
            r, msgs = say("C8", text)
            bt.call("settings.update", {"patch": {"agent": {"permissions": {"mode": "allow-all", "rules": DENIED}}}})
            check("C8: she sent it through the client: one message, no key",
                  calls(msgs, "game_chat.send") and len(POSTS) == before + 1 and not kotone_keys(),
                  f"{[c['name'] for c in calls(msgs)]} {POSTS[before:]}"[:300])
        print(f"   C: {len(POSTS)} chat message(s) reached the fake client; Kotone keys in the window: {len(kotone_keys())}")
    finally:
        window.terminate()


def main():
    global REC
    if WHILE_PLAYING:
        bt.guard = lambda: None  # 不发任何系统按键，玩家同意对局中也跑（2026-11-23）
    else:
        bt.guard()
        no_game_running()
    for need in (os.path.join(PROBE, "lcu-gameflow-session.json"), os.path.join(PROBE, "live-allgamedata.json"),
                 os.path.join(GAMEDATA, "champion-summary.json")):
        if not os.path.exists(need):
            raise SystemExit(f"missing recording: {need} (record one with lol_probe.py)")
    shutil.rmtree(bt.WORK, ignore_errors=True)
    os.makedirs(bt.HOME)
    REC = Recording(PROBE, GAMEDATA)
    enemies, allies = REC.champion_names()
    print(f"recording: queue {REC.queue()}, {len(REC.players)} histories; enemies {enemies}; allies {allies}")
    server, port = serve()
    bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
                  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}",
                  KOTONE_LOL_LCU=f"{port}:{TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}")
    bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   # 选中英雄联盟：替玩家发言走按键时前台必须是游戏，C 部分不会往别的窗口里打字
                   "activeProfileId": "lol",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": DENIED}}}, f)
    app = subprocess.Popen([bt.EXE, "--background"], env=bt.ENV)
    try:
        if CHAT_ONLY:
            # 只要工具服务（C 部分不看界面，界面没嵌进 exe 也能跑）
            def service_up():
                try:
                    return bool(bt.call("app.status", {}, 3000))
                except Exception:  # noqa: BLE001
                    return False
            if not bt.wait_until(service_up, 90, 1):
                raise RuntimeError("the tool service did not come up")
            # 等插件连上假客户端（后台每 2–5 秒看一次）
            bt.wait_until(lambda: HITS["/lol-gameflow/v1/gameflow-phase"] > 0, 30, 0.5)
            return chat_part(with_model=False)
        bt.wait_ui()
        cid = real_deepseek.add_connection(bt.EXE, bt.ENV, model="deepseek-flash")
        bt.call("settings.update", {"patch": {"assistant": {"connectionId": cid}}})
        bt.invoke("agent_new_conversation", {})

        log = lambda: open(os.path.join(bt.HOME, "kotone.log"), encoding="utf-8", errors="replace").read()  # noqa: E731
        fetched = bt.wait_until(lambda: "个人的战绩查完了" in log(), 60, 1)
        check("P: connected to the fake client (no process lookup) and fetched every history in the background",
              fetched and "KOTONE_LOL_LCU" in log() and HITS["/lol-match-history/v1/products/lol/<puuid>/matches"] >= 10,
              f"history requests {HITS['/lol-match-history/v1/products/lol/<puuid>/matches']}")
        lissandra = next((n for n in enemies if "丽桑卓" in n), None)
        if not check("P: the recording has 丽桑卓 on the enemy side", lissandra, str(enemies)):
            return bt.finish()

        # T1 战绩和突破口
        text = "查一下这把的战绩，对面谁是突破口"
        r, msgs = say("T1", text)
        scout = results(msgs, "lol.scout")
        check("T1: she used lol.scout", scout, ",".join(c["name"] for c in calls(msgs)))
        body = scout[-1] if scout else ""
        # 开头那行“不看玩没玩过这个英雄”本身带“没玩过”：查的是标签和突破口里的写法
        check("T1: random heroes (queue %d): the result says so and has no “没玩过” tag" % REC.queue(),
              "英雄是随机的" in body and "最近没玩过" not in body and "局没玩过" not in body
              and "这个英雄 0 局" not in body, body[:120])
        said = r.get("said", "")
        check("T1: she names an enemy champion", any(n in said for n in enemies), said[:200])
        # ADR 038 第 2 步：结果带这局页面的快照（聊天卡片），给她的还是那几行字
        page = (displays(msgs, "lol.scout") or [{}])[-1]
        data = page.get("structuredContent") or {}
        check("T1: the result carries the match page (ui://kotone/lol/match) with 5 + 5 players",
              (page.get("ui") or {}).get("uri") == "ui://kotone/lol/match"
              and len(data.get("allies", [])) == 5 and len(data.get("enemies", [])) == 5,
              json.dumps(page, ensure_ascii=False)[:200])
        check("T1: the page data did not go into her context (content is the text only)",
              "allies" not in body and "structuredContent" not in body, body[:120])
        # 2026-11-25 G0.1：页面要的图只给 id 和客户端路径（素材层认得的），给她的字里没有
        people = data.get("allies", []) + data.get("enemies", [])
        check("T1: every player on the page has a champion icon path, a profile icon path and the summoner spells",
              people and all(p.get("champion", {}).get("icon", "").startswith("/lol-game-data/assets/v1/champion-icons/")
                             and p.get("profileIcon", "").startswith("/lol-game-data/assets/v1/profile-icons/")
                             and len(p.get("spells") or []) == 2 and "ranked" in p for p in people)
              and "/lol-game-data/" not in body,
              json.dumps(people[:1], ensure_ascii=False)[:300])
        check("T1: no labels like 牛马 / 下等马, no “没玩过”",
              not any(w in said for w in ("牛马", "下等马", "没玩过", "第一次玩")), said[:200])

        # T2 记闪
        text = "对面丽桑卓闪了"
        r, msgs = say("T2", text)
        spell = calls(msgs, "lol.spell")
        args = json.loads(spell[-1]["arguments"]) if spell else {}
        out = results(msgs, "lol.spell")
        check("T2: she used lol.spell for 丽桑卓", spell and "丽桑卓" in args.get("champion", ""), json.dumps(args, ensure_ascii=False))
        check("T2: recorded", out and re.search(r'"status"\s*:\s*"ok"', out[-1]), (out or ["-"])[-1][:200])
        chat = re.search(r'"chat"\s*:\s*"([^"]*)"', out[-1]) if out else None
        chat = chat.group(1) if chat else ""
        said = r.get("said", "")
        check("T2: she says when it's back", re.search(r"\d+\s*[:：分]\s*\d+", said), said[:200])

        # T3 谁没闪
        text = "谁没闪"
        r, msgs = say("T3", text)
        said = r.get("said", "")
        check("T3: she names 丽桑卓", "丽桑卓" in said, f"tools {[c['name'] for c in calls(msgs)]}: {said[:200]}")

        # T4 发出去（game_chat.send 在这个测试里禁掉了，只看她想不想发、发什么）
        text = "发出去"
        r, msgs = say("T4", text)
        send = calls(msgs, "game_chat.send")
        sent = json.loads(send[-1]["arguments"]) if send else {}
        line = json.dumps(sent, ensure_ascii=False)
        check("T4: she tried game_chat.send", send, ",".join(c["name"] for c in calls(msgs)))
        check("T4: the line names 丽桑卓 and the flash", "丽桑卓" in line and "闪" in line, line[:200])
        touched = [c["name"] for c in calls(msgs) if c["name"] in ("game.focus", "game.launch", "game.close")]
        focus_results = [r for r in results(msgs, "game.focus") if "focused" in r]
        check("T4: no real game window was touched", not focus_results, f"called {touched}, results {focus_results}")
        print(f"   lol.spell chat line: {chat!r}; she tried to send: {line[:200]}")
        print(f"   she said: {r.get('said', '')[:200]}")

        # T5 某个人的战绩（ADR 038 第 2 步：lol.player，带战绩页面）
        text = "对面丽桑卓最近战绩怎么样"
        r, msgs = say("T5", text)
        player = calls(msgs, "lol.player")
        args = json.loads(player[-1]["arguments"]) if player else {}
        check("T5: she used lol.player (or lol.scout for one player)",
              player or calls(msgs, "lol.scout"), ",".join(c["name"] for c in calls(msgs)))
        page = (displays(msgs, "lol.player") or displays(msgs, "lol.scout") or [{}])[-1]
        check("T5: the result carries a page", (page.get("ui") or {}).get("uri", "").startswith("ui://kotone/lol/"),
              json.dumps(page, ensure_ascii=False)[:160])
        print(f"   T5 args {args}; she said: {r.get('said', '')[:160]}")

        # P2 只给页面的工具和 ui.open（经工具服务，外部 Agent 也这样用）
        me = bt.call("lol.player", {})
        games = ((me.get("structuredContent") or {}).get("games") or [])
        check("P2: lol.player without args is the player themself, with games",
              (me.get("_meta") or {}).get("ui", {}).get("resourceUri") == "ui://kotone/lol/player" and games,
              json.dumps(me, ensure_ascii=False)[:200])
        if games:
            detail = bt.call("lol.game", {"gameId": games[0]["gameId"]})
            teams = ((detail.get("structuredContent") or {}).get("teams") or [])
            check("P2: lol.game (page only) gives both teams of one game",
                  len(teams) == 2 and all(len(t.get("players", [])) == 5 for t in teams),
                  json.dumps(detail, ensure_ascii=False)[:200])
            rows = [p for t in teams for p in t.get("players", [])]
            check("P2: lol.player and lol.game carry the icon paths (profile icon, level, spells, items, augments with rarity)",
                  (me.get("structuredContent") or {}).get("profileIcon", "").endswith(".jpg")
                  and (me.get("structuredContent") or {}).get("summonerLevel")
                  and rows and all(len(p.get("spells") or []) == 2 and p.get("items") and all(i.get("icon") for i in p["items"])
                                   and all(a.get("rarity") and a.get("icon") for a in p.get("augments") or [])
                                   for p in rows),
                  json.dumps(rows[:1], ensure_ascii=False)[:300])
        opened = bt.call("ui.open", {"page": "ui://kotone/lol/match"})
        # 2026-11-25 起不打断玩家：直接显示（shown）或放在右侧栏等玩家点（waiting），都是窗口回了话；sent = 窗口没回话
        check("P2: ui.open opens the match page (the window says shown or waiting)",
              opened.get("status") in ("shown", "waiting") and opened.get("opened") is (opened.get("status") == "shown"),
              json.dumps(opened, ensure_ascii=False))
        try:
            bt.call("ui.open", {"page": "ui://kotone/lol/nope"})
            check("P2: ui.open refuses an unknown page", False, "no error")
        except Exception as e:  # noqa: BLE001
            check("P2: ui.open refuses an unknown page", True, str(e)[:120])

        # T6 英雄的统计（ADR 038 第 3 步：lol.champion，真网络：腾讯 101 / aramhex-data）
        text = "刀妹出什么装，克制谁"
        r, msgs = say("T6", text)
        champ = calls(msgs, "lol.champion")
        check("T6: she used lol.champion", champ, ",".join(c["name"] for c in calls(msgs)))
        page = (displays(msgs, "lol.champion") or [{}])[-1]
        data = page.get("structuredContent") or {}
        check("T6: the result carries the champion page with a source line",
              (page.get("ui") or {}).get("uri") == "ui://kotone/lol/champion" and data.get("sources"),
              json.dumps(page, ensure_ascii=False)[:200])
        print(f"   T6 args {[c['arguments'] for c in champ]}; she said: {r.get('said', '')[:200]}")

        # P3 经工具服务：峡谷强度榜、海斗的英雄 × 海克斯（署名）
        tier = bt.call("lol.tier", {"mode": "rift", "lane": "上"})
        rows = ((tier.get("structuredContent") or {}).get("lanes") or {}).get("TOP") or []
        check("P3: lol.tier gives the top-lane tier list", len(rows) >= 20, json.dumps(tier, ensure_ascii=False)[:160])
        mayhem = bt.call("lol.champion", {"champion": "刀妹", "mode": "mayhem"})
        sc = mayhem.get("structuredContent") or {}
        check("P3: mayhem champion has augments and the aramhex-data attribution (CC BY-NC)",
              sc.get("augments") and "aramhex-data" in (sc.get("attribution") or "") and "CC BY-NC" in (sc.get("attribution") or ""),
              json.dumps(sc, ensure_ascii=False)[:200])
        opgg = bt.call("lol.champion", {"champion": "亚索", "mode": "rift"})
        check("P3: OP.GG is off by default (no OP.GG request)",
              not ((opgg.get("structuredContent") or {}).get("opgg")), json.dumps(opgg.get("structuredContent", {}).get("opgg"), ensure_ascii=False))
        return chat_part(with_model=True)
    finally:
        if not CHAT_ONLY:
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
        print("\nfake client hits:", dict(HITS))
        if MISSES:
            print("not served:", dict(MISSES))


PRESETS = ["lol.shout-enemy-flash", "lol.shout-enemy-records", "lol.shout-ally-records", "lol.shout-premade"]


def shout(args):
    """lol.shout over the tool service: (ok, content dict or error text)."""
    try:
        return True, (bt.call("lol.shout", args) or {}).get("content") or {}
    except Exception as e:  # noqa: BLE001
        return False, str(e)


def hotkey_status():
    try:
        return bt.call("hotkeys.status", {}) or {}
    except Exception:  # noqa: BLE001
        return {}


def player_names():
    names = {(v.get("summoner") or {}).get("gameName") for v in REC.players.values()}
    names |= {p.get("riotIdGameName") for p in REC.live.get("allPlayers") or []}
    return {n for n in names if n and len(n) >= 2}


def shout_part():
    """S: one-key shout (ADR 041 phase 3), no OS input."""
    REC.set_phase("InProgress")
    log = os.path.join(bt.HOME, "kotone.log")
    # 战绩是后台查好的（喊话不现查）：等它查完
    bt.wait_until(lambda: "个人的战绩查完了" in open(log, encoding="utf-8", errors="replace").read(), 60, 1)
    st = hotkey_status()
    rows = {b.get("id"): b for b in st.get("bindings") or []}
    check("S1: the four LoL shout presets are rows of the hotkey table (plugin lol, calls a tool, no key)",
          all(rows.get(p, {}).get("owner") == "lol" and rows[p].get("action") == "tool" and not rows[p].get("key")
              for p in PRESETS), json.dumps([rows.get(p) for p in PRESETS], ensure_ascii=False)[:300])

    bt.call("lol.spell", {"clear": True})
    ok, out = shout({"what": "flash", "preview": True})
    lines = out.get("lines") if ok else []
    check("S2: nothing recorded -> nothing to shout but who has no Flash (says how to record it)",
          (not ok and "对面亚索闪了" in out) or (ok and all(line.startswith("对面没带闪") for line in lines)),
          json.dumps(out, ensure_ascii=False)[:200])
    spell = (bt.call("lol.spell", {"champion": "丽桑卓"}) or {}).get("content") or {}
    ok, out = shout({"what": "flash", "preview": True})
    lines = out.get("lines") if ok else []
    check("S2: after lol.spell 丽桑卓 the first line is lol.spell's chat line",
          ok and spell.get("chat") and lines and lines[0] == spell["chat"],
          f"spell {json.dumps(spell, ensure_ascii=False)[:120]}; shout {json.dumps(out, ensure_ascii=False)[:200]}")

    names = player_names()
    ok, out = shout({"what": "records", "side": "enemy", "preview": True})
    lines = out.get("lines") if ok else []
    leaked = [n for n in names if any(n in line for line in lines)]
    check("S3: enemy records: one line per enemy, '对面 <champion>：…', no player names",
          ok and len(lines) == 5 and all(line.startswith("对面 ") for line in lines) and not leaked,
          json.dumps(out, ensure_ascii=False)[:300] + (f" leaked {leaked}" if leaked else ""))
    print("   S3 lines:", " | ".join(lines))
    ok, out = shout({"what": "premade", "preview": True})
    text = json.dumps(out, ensure_ascii=False)
    check("S3: premade: one line per group, or 'no premade' and nothing sent",
          (ok and all("开黑：" in line for line in out.get("lines") or [])) or (not ok and "没看出谁和谁开黑" in out),
          text[:200])
    check("S3: premade names no players", not [n for n in names if n in text], text[:200])

    process, title = front()
    if WHILE_PLAYING or process.lower() not in SAFE_FOREGROUND or process.lower() == "leagueclientux.exe":
        print(f"   S4 skipped: '{title}' ({process}) is in front{' (--while-playing)' if WHILE_PLAYING else ''}")
    else:
        before = len(POSTS)
        ok, out = shout({"what": "flash"})
        check("S4: a real send with a terminal in front is refused by the entry (the foreground decides)",
              not ok and "请先切到游戏" in str(out), f"front {process}: {str(out)[:200]}")
        check("S4: the fake client got no message", len(POSTS) == before, str(POSTS[before:])[:200])

    with open(os.path.join(bt.HOME, "config.json"), encoding="utf-8") as f:
        saved = ((json.load(f).get("hotkeys") or {}).get("bindings")) or []
    bt.call("settings.update", {"patch": {"hotkeys": {"bindings": saved + [{"key": "F19", "command": PRESETS[0]}]}}})
    named = None
    for _ in range(20):
        named = next((n for n in hotkey_status().get("named") or [] if n.get("id") == PRESETS[0]), None)
        if named:
            break
        time.sleep(0.25)
    check("S5: F19 on 喊对面闪现 is not registered while lol.shout is denied, and says so",
          named and not named.get("key") and "禁止" in (named.get("error") or ""), json.dumps(named, ensure_ascii=False))
    bt.call("settings.update", {"patch": {"hotkeys": {"bindings": saved}}})


def chat_part(with_model):
    """S and C0 always; C1-C8 only with --chat-front (the fake client window has to be in front)."""
    shout_part()
    chat_c0()
    if CHAT_FRONT and not WHILE_PLAYING:
        chat_front(with_model)
    else:
        print("   C1-C8 not run: they need the fake client window in front (--chat-front, the player away)")
    return bt.finish()


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
