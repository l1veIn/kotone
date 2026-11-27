"""CS2 plugin real-machine check (ADR 036 D9, ADR 047 phase 3) without opening the game.

An isolated Kotone (debug build, `--background`, KOTONE_HOME=target/cs2-gsi-real/home, its own WebView2 data,
runAsAdminOnStart off: no UAC) with KOTONE_CS2_DIR pointing at a FAKE CS2 install under target/cs2-gsi-real/fake-cs2
(only `game/csgo/cfg` inside). The real Steam / CS2 folders are never looked at: with KOTONE_CS2_DIR set the plugin
uses it, and the script refuses to run if the override is missing. Before Kotone starts, the script holds the
default port 23730 on 127.0.0.1, so the plugin has to move to the next port and write that one into the cfg.

The script reads the cfg Kotone wrote (port + token) and POSTs a synthetic match (fields as in Valve's GSI docs) on a
timeline: menu → warmup → round 1 (I am T): the bomb is planted, ~31 s later the "10 seconds left" push, then it
explodes and T wins → rounds 2–12 quickly (7 : 5) → halftime (sides switch, I am CT) → rounds 13–22 (12 : 9 is
match point for us, once; 12 : 10 stays ours and is not said again) → round 23 we win → gameover 13 : 10. No key presses, no mouse, no sound (speech volume 0,
floating window off), no DeepSeek key (pushes need no model).

Checks:
K1 the cfg is in the fake dir: our marker, uri http://127.0.0.1:<port>/ with port != 23730 (23730 was taken), a
   32-hex token, the data sections (provider, map, round, player_*, bomb, phase_countdowns); plugins.cs2.settings
   has the same port and token;
K2 a POST with a wrong token gets 403 and changes nothing (game.state has no cs2 yet);
K3 game.state: cs2 in menu after the menu post, pre_match in warmup, in_match during the rounds, post_match at the end;
K4 the cs2.* events in kotone.log (round_started, bomb_planted, bomb_exploded, round_ended with winner T / won true /
   reason bomb, halftime, match_point ours, match_ended win 13 : 10 with my K/D/A);
K5 the pushes in the conversation: ONE bomb card for round 1 updated in place from planted to exploded (same key),
   the bomb-ten line "炸弹还剩 10 秒" (and what she said, if a voice was picked), halftime and match-point lines,
   the match-end card with result win and a note for her;
K6 cs2.state mid-match (status ok, 荒漠迷城, score, round) and after the end (result 赢了); cs2.rounds lists 23 rounds;
K7 settings.update writeCfg false → the cfg is deleted; true → written again (the plugin only touches its own file);
K8 no posts for 35 s → cs2 leaves the hub (cs2.exe is not running here).
usage: python -X utf8 -I -B scripts/manual-e2e/cs2_gsi_real.py [--exe PATH]
   build first: cargo build -p kotone-tauri  (no web UI needed: everything is checked through the tool service,
   kotone.log and the conversation files)
"""
import glob
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

WORK = os.path.join(REPO, "target", "cs2-gsi-real")
HOME = os.path.join(WORK, "home")
FAKE = os.path.join(WORK, "fake-cs2")
CFG_DIR = os.path.join(FAKE, "game", "csgo", "cfg")
CFG = os.path.join(CFG_DIR, "gamestate_integration_kotone.cfg")
EXE = sys.argv[sys.argv.index("--exe") + 1] if "--exe" in sys.argv else os.path.join(REPO, "target", "debug", "kotone.exe")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
DEFAULT_PORT = 23730
ENV = dict(os.environ, KOTONE_HOME=HOME, KOTONE_CS2_DIR=FAKE, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ME, MATE = "76561190000000001", "76561190000000002"
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def guard():
    running = subprocess.run(["tasklist"], capture_output=True, text=True, encoding="mbcs", errors="replace").stdout.lower()
    for exe in ("cs2.exe", "league of legends.exe", "valorant", "dota2.exe"):
        if exe in running:
            print(f"NOTE: {exe} is running (the test sends no input and never touches the real game folder)")
    path = foreground_process()
    if any(g in os.path.basename(path).lower() for g in GAMES):
        print(f"NOTE: a game is in front ({os.path.basename(path)}); this test sends no input, going on")
    if not ENV.get("KOTONE_CS2_DIR", "").startswith(os.path.join(REPO, "target")):
        raise SystemExit("KOTONE_CS2_DIR must point under target/")


def tools_call(tool, args=None, timeout_ms=20_000):
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", "-", "--timeout-ms", str(timeout_ms)],
                         input=json.dumps(args or {}, ensure_ascii=False), capture_output=True, text=True,
                         encoding="utf-8", env=ENV)
    try:
        envelope = json.loads(out.stdout)
    except json.JSONDecodeError:
        return {"error": (out.stdout + out.stderr)[-500:]}
    return envelope.get("result") if envelope.get("ok") else {"error": envelope.get("error")}


def wait_until(predicate, timeout, step=0.5):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return None


def read_cfg():
    try:
        text = open(CFG, encoding="utf-8").read()
    except OSError:
        return None
    port = re.search(r'"uri"\s+"http://127\.0\.0\.1:(\d+)/"', text)
    token = re.search(r'"token"\s+"([0-9a-f]{32})"', text)
    return (text, int(port.group(1)), token.group(1)) if port and token else None


def payload(token, map_name="de_mirage", phase=None, round_phase="freezetime", bomb="", win="", ct=0, t=0, team="T",
            player=ME, health=100, stats=(0, 0, 0, 0)):
    p = {
        "provider": {"name": "Counter-Strike 2", "appid": 730, "version": 14070, "steamid": ME, "timestamp": int(time.time())},
        "player": {"steamid": player, "name": "Tester", "team": team, "activity": "playing" if phase else "menu",
                   "state": {"health": health, "armor": 100, "helmet": True, "money": 4000, "round_kills": 0,
                             "round_killhs": 0, "equip_value": 4700},
                   "match_stats": {"kills": stats[0], "deaths": stats[1], "assists": stats[2], "mvps": stats[3], "score": 30}},
        "auth": {"token": token},
    }
    if phase:
        p["map"] = {"mode": "competitive", "name": map_name, "phase": phase, "round": ct + t,
                    "team_ct": {"score": ct}, "team_t": {"score": t}}
        p["round"] = {"phase": round_phase}
        if bomb:
            p["round"]["bomb"] = bomb
        if win:
            p["round"]["win_team"] = win
    return p


def post(port, body):
    req = urllib.request.Request(f"http://127.0.0.1:{port}/", data=json.dumps(body).encode(), method="POST",
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code


def cs2_state():
    state = tools_call("game.state") or {}
    games = state.get("games") or state.get("playing") or []
    if isinstance(games, dict):
        games = list(games.values())
    return next((g for g in games if isinstance(g, dict) and g.get("game") == "cs2"), None), state


def cs2_phase():
    game, _ = cs2_state()
    return (game or {}).get("phase")


def log_events():
    try:
        text = open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except OSError:
        return []
    out = []
    for line in text.splitlines():
        m = re.search(r"game: cs2 (cs2\.[a-z_]+) (\{.*\})\s*$", line)
        if m:
            try:
                out.append((m.group(1), json.loads(m.group(2))))
            except json.JSONDecodeError:
                pass
    return out


def pushes():
    """Every push record of the cs2 plugin in the conversation files (latest content of each)."""
    found = {}

    def walk(v):
        if isinstance(v, dict):
            push = v.get("push")
            if isinstance(push, dict) and push.get("plugin") == "cs2":
                found.setdefault(push.get("key"), []).append(push)
            for x in v.values():
                walk(x)
        elif isinstance(v, list):
            for x in v:
                walk(x)
    for path in glob.glob(os.path.join(HOME, "agent", "**", "*.json"), recursive=True):
        try:
            walk(json.load(open(path, encoding="utf-8")))
        except (OSError, json.JSONDecodeError):
            pass
    return found


def notes():
    out = []

    def walk(v):
        if isinstance(v, dict):
            if v.get("role") == "user" and v.get("source") == "auto-card" and "CS2" in json.dumps(v, ensure_ascii=False):
                out.append(v)
            for x in v.values():
                walk(x)
        elif isinstance(v, list):
            for x in v:
                walk(x)
    for path in glob.glob(os.path.join(HOME, "agent", "**", "*.json"), recursive=True):
        try:
            walk(json.load(open(path, encoding="utf-8")))
        except (OSError, json.JSONDecodeError):
            pass
    return out


def main():
    guard()
    os.makedirs(os.path.dirname(LOCK), exist_ok=True)
    if not wait_until(lambda: _take_lock(), 600, 5):
        raise SystemExit(f"another real-machine test holds {LOCK}")
    app = None
    holder = None
    try:
        shutil.rmtree(WORK, ignore_errors=True)
        os.makedirs(HOME)
        os.makedirs(CFG_DIR)
        with open(os.path.join(CFG_DIR, "gamestate_integration_other.cfg"), "w") as f:
            f.write('"Other"\n{\n}\n')
        with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
            json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                       "runAsAdminOnStart": False,
                       "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                       "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                     "wake": {"enabled": False}, "bubble": {"enabled": False}},
                       "agent": {"autoCards": {"popup": False}, "matchRecap": False}}, f)
        # 占住默认端口：插件要换下一个
        holder = socket.socket()
        try:
            holder.bind(("127.0.0.1", DEFAULT_PORT))
            holder.listen(1)
        except OSError:
            print(f"NOTE: {DEFAULT_PORT} was already taken by someone else")
        app = subprocess.Popen([EXE, "--background"], env=ENV)

        # K1 cfg
        got = wait_until(read_cfg, 90)
        if not check("K1 the cfg was written in the fake CS2 dir", got, CFG):
            return False
        text, port, token = got
        check("K1 marker, loopback uri, data sections", text.startswith("// Written by Kotone") and all(
            f'"{s}"' in text for s in ("provider", "map", "round", "player_id", "player_state", "player_match_stats", "bomb",
                                      "phase_countdowns")), f"port {port}")
        check("K1 23730 was taken: moved to the next port", port != DEFAULT_PORT and DEFAULT_PORT < port < DEFAULT_PORT + 20, str(port))
        check("K1 the other GSI file is untouched",
              open(os.path.join(CFG_DIR, "gamestate_integration_other.cfg")).read() == '"Other"\n{\n}\n')
        saved = json.load(open(os.path.join(HOME, "config.json"), encoding="utf-8")).get("plugins", {}).get("cs2", {}).get("settings", {})
        check("K1 plugins.cs2.settings has the same port and token", (saved.get("port"), saved.get("token")) == (port, token),
              json.dumps({"port": saved.get("port"), "writeCfg": saved.get("writeCfg")}))
        listening = wait_until(lambda: _connectable(port), 20)
        check("K1 Kotone listens on 127.0.0.1:<port>", listening, str(port))

        # K2 wrong token
        code = post(port, payload("0" * 32))
        check("K2 a wrong token gets 403", code == 403, str(code))
        time.sleep(2.5)
        check("K2 … and changes nothing", cs2_phase() in (None, "stopped"), str(cs2_phase()))

        # K3 menu / warmup
        post(port, payload(token))
        check("K3 menu", wait_until(lambda: cs2_phase() == "menu", 10), str(cs2_phase()))
        post(port, payload(token, phase="warmup", round_phase="warmup"))
        check("K3 warmup = pre_match", wait_until(lambda: cs2_phase() == "pre_match", 10), str(cs2_phase()))

        # round 1: plant, 10 s left, explode, T wins (I am T)
        post(port, payload(token, phase="live", round_phase="freezetime"))
        time.sleep(0.5)
        post(port, payload(token, phase="live", round_phase="live"))
        check("K3 live = in_match", wait_until(lambda: cs2_phase() == "in_match", 10), str(cs2_phase()))
        time.sleep(1)
        planted_at = time.time()
        post(port, payload(token, phase="live", round_phase="live", bomb="planted", stats=(1, 0, 0, 0)))
        time.sleep(2)
        mid = tools_call("cs2.state") or {}
        content = mid.get("content") if isinstance(mid.get("content"), dict) else mid
        check("K6 cs2.state mid-match", content.get("status") == "ok" and content.get("map") == "荒漠迷城"
              and "约" in str(content.get("bomb", "")), json.dumps(content, ensure_ascii=False)[:300])
        # 等“还剩 10 秒”：游戏每 10 秒有一次心跳，这里每 3 秒补一份一样的（不会多出事件），免得 30 秒收不到算断了
        ten_at = None
        while time.time() - planted_at < 39 and ten_at is None:
            post(port, payload(token, phase="live", round_phase="live", bomb="planted", stats=(1, 0, 0, 0)))
            if any(k.startswith("bomb-ten:") for k in pushes()):
                ten_at = time.time() - planted_at
            time.sleep(1)
        check("K5 the 10-seconds-left push came ~30 s after the plant", ten_at is not None and 29 <= ten_at <= 34,
              f"{ten_at} s after the plant")
        time.sleep(max(0, planted_at + 40.5 - time.time()))
        post(port, payload(token, phase="live", round_phase="live", bomb="exploded", stats=(1, 0, 0, 0)))
        time.sleep(0.5)
        post(port, payload(token, phase="live", round_phase="over", bomb="exploded", win="T", t=1, stats=(1, 0, 0, 0)))

        # rounds 2–12：上半场打完 T（我）7 : 5
        ct, t = 0, 1
        first_half = ["T", "CT", "T", "CT", "T", "CT", "T", "CT", "T", "T", "CT"]  # 再赢 6 输 5
        for winner in first_half:
            post(port, payload(token, phase="live", round_phase="freezetime", ct=ct, t=t))
            post(port, payload(token, phase="live", round_phase="live", ct=ct, t=t))
            if winner == "CT":
                ct += 1
            else:
                t += 1
            post(port, payload(token, phase="live", round_phase="over", win=winner, ct=ct, t=t))
            time.sleep(0.2)
        check("first half 5 : 7 (T is me)", (ct, t) == (5, 7), f"{ct}:{t}")
        post(port, payload(token, phase="intermission", round_phase="over", win=first_half[-1], ct=ct, t=t))
        time.sleep(1)
        # 换边：我到 CT，比分跟着队走
        ct, t = 7, 5
        second_half = ["CT", "T", "CT", "T", "CT", "T", "CT", "T", "CT", "T"]  # 5 : 5 → 12 : 10
        for winner in second_half:
            post(port, payload(token, phase="live", round_phase="freezetime", ct=ct, t=t, team="CT"))
            post(port, payload(token, phase="live", round_phase="live", ct=ct, t=t, team="CT"))
            if winner == "CT":
                ct += 1
            else:
                t += 1
            post(port, payload(token, phase="live", round_phase="over", win=winner, ct=ct, t=t, team="CT",
                               stats=(15, 9, 3, 3)))
            time.sleep(0.2)
        check("second half 12 : 10 (CT is me)", (ct, t) == (12, 10), f"{ct}:{t}")
        post(port, payload(token, phase="live", round_phase="freezetime", ct=ct, t=t, team="CT", stats=(15, 9, 3, 3)))
        post(port, payload(token, phase="live", round_phase="live", ct=ct, t=t, team="CT", stats=(15, 9, 3, 3)))
        post(port, payload(token, phase="gameover", round_phase="over", win="CT", ct=13, t=10, team="CT",
                           stats=(16, 9, 3, 4)))
        check("K3 gameover = post_match", wait_until(lambda: cs2_phase() == "post_match", 10), str(cs2_phase()))
        time.sleep(2)

        # K4 events
        events = log_events()
        kinds = [k for k, _ in events]
        print("INFO events:", {k: kinds.count(k) for k in sorted(set(kinds))})
        check("K4 round_started ×23", kinds.count("cs2.round_started") == 23, str(kinds.count("cs2.round_started")))
        check("K4 round_ended ×23", kinds.count("cs2.round_ended") == 23, str(kinds.count("cs2.round_ended")))
        first_end = next((d for k, d in events if k == "cs2.round_ended"), {})
        check("K4 round 1: T won by the bomb, won true", first_end.get("winner") == "T" and first_end.get("won") is True
              and first_end.get("reason") == "bomb", json.dumps(first_end))
        for kind in ("cs2.bomb_planted", "cs2.bomb_exploded", "cs2.halftime", "cs2.match_point", "cs2.match_ended"):
            check(f"K4 {kind} once", kinds.count(kind) == 1, str(kinds.count(kind)))
        mp = next((d for k, d in events if k == "cs2.match_point"), {})
        check("K4 match point is ours when we reach 12 (12 : 9)", mp.get("ours") is True and (mp.get("ctScore"), mp.get("tScore")) == (12, 9),
              json.dumps(mp))
        end = next((d for k, d in events if k == "cs2.match_ended"), {})
        check("K4 match_ended win 13 : 10, 16/9/3, MVP 4", end.get("result") == "win" and end.get("myScore") == 13
              and end.get("enemyScore") == 10 and (end.get("kills"), end.get("deaths"), end.get("assists"), end.get("mvps"))
              == (16, 9, 3, 4), json.dumps(end))

        # K5 pushes
        found = pushes()
        bomb_keys = [k for k in found if k.startswith("bomb:")]
        check("K5 one bomb card", len(bomb_keys) == 1, str(bomb_keys))
        if bomb_keys:
            states = [((p.get("card") or {}).get("structuredContent") or (p.get("card") or {}).get("data") or {}).get("state")
                      for p in found[bomb_keys[0]]]
            check("K5 the bomb card ended as exploded (updated in place)", states and states[-1] == "exploded", str(states))
        ten_keys = [k for k in found if k.startswith("bomb-ten:")]
        if ten_keys:
            p = found[ten_keys[0]][-1]
            cell = (((p.get("line") or {}).get("cells") or [{}])[0]).get("text")
            check("K5 bomb-ten line", cell == "炸弹还剩 10 秒", f"{cell} / said {p.get('said')!r}")
        half = [found[k][-1] for k in found if k.startswith("halftime:")]
        check("K5 halftime line", half and "半场" in json.dumps(half[0], ensure_ascii=False),
              json.dumps(half[0].get("line") if half else None, ensure_ascii=False))
        mpp = [found[k][-1] for k in found if k.startswith("match-point:")]
        check("K5 match-point line (once)", len(mpp) == 1 and "赛点" in json.dumps(mpp[0], ensure_ascii=False),
              json.dumps(mpp[0].get("line") if mpp else None, ensure_ascii=False))
        ends = [found[k][-1] for k in found if k.startswith("match-end:")]
        card = ((ends[0].get("card") or {}) if ends else {})
        data = card.get("structuredContent") or card.get("data") or {}
        check("K5 match-end card: win, 23 rounds", data.get("result") == "win" and len(data.get("rounds") or []) == 23,
              json.dumps({k: data.get(k) for k in ("result", "myScore", "enemyScore")}, ensure_ascii=False))
        n = notes()
        check("K5 a note for her at the end", any("这张图打完了" in json.dumps(x, ensure_ascii=False) for x in n), f"{len(n)} note(s)")

        # K6 after the end
        after = tools_call("cs2.state") or {}
        content = after.get("content") if isinstance(after.get("content"), dict) else after
        check("K6 cs2.state after the end says 赢了", content.get("result") == "赢了", json.dumps(content, ensure_ascii=False)[:300])
        rounds = tools_call("cs2.rounds") or {}
        rc = rounds.get("content") if isinstance(rounds.get("content"), dict) else rounds
        check("K6 cs2.rounds lists 23 rounds", len(rc.get("rounds") or []) == 23, str(len(rc.get("rounds") or [])))

        # K7 the switch
        tools_call("settings.update", {"patch": {"plugins": {"cs2": {"settings": {"writeCfg": False}}}}})
        check("K7 writeCfg false → our cfg is deleted", wait_until(lambda: not os.path.exists(CFG), 15))
        check("K7 … the other file stays", os.path.exists(os.path.join(CFG_DIR, "gamestate_integration_other.cfg")))
        tools_call("settings.update", {"patch": {"plugins": {"cs2": {"settings": {"writeCfg": True}}}}})
        again = wait_until(read_cfg, 15)
        check("K7 writeCfg true → written again, same port and token", again and again[1:] == (port, token))

        # K8 stale
        left = wait_until(lambda: cs2_phase() in (None, "stopped"), 50, 2)
        check("K8 no posts for 30 s → cs2 handed back (stopped)", left, str(cs2_phase()))
    finally:
        if holder:
            holder.close()
        if app:
            try:
                tools_call("app.quit")
            except Exception as e:  # noqa: BLE001
                print("quit:", e)
            time.sleep(3)
            if app.poll() is None:
                app.terminate()
        try:
            os.rmdir(LOCK)
        except OSError:
            pass
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


def _take_lock():
    try:
        os.mkdir(LOCK)
        return True
    except FileExistsError:
        return False


def _connectable(port):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            return True
    except OSError:
        return False


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
