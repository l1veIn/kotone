"""英雄联盟本地接口的只读探测（ADR 035 实测用）。

只发 GET，不按键、不写客户端。令牌只留在内存里，不打印、不写文件。
结果写到 target/lol-probe/<时间>/（target 已被 .gitignore 忽略）。

用法（在仓库根目录）：
    python -X utf8 -I scripts/manual-e2e/lol_probe.py            # 探测一次
    python -X utf8 -I scripts/manual-e2e/lol_probe.py --watch 900  # 盯 15 分钟：阶段变化、2999 什么时候能连
"""

import argparse
import base64
import ctypes
import ctypes.wintypes as wt
import json
import re
import ssl
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE


# ---------- 读 LeagueClientUx.exe 的命令行（照 LeagueAkari：NtQueryInformationProcess，不提权） ----------

PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_COMMAND_LINE_INFORMATION = 60


class UNICODE_STRING(ctypes.Structure):
    _fields_ = [("Length", wt.USHORT), ("MaximumLength", wt.USHORT), ("Buffer", ctypes.c_void_p)]


def pids(image: str) -> list[int]:
    out = subprocess.run(
        ["tasklist", "/fo", "csv", "/nh", "/fi", f"imagename eq {image}"],
        capture_output=True, text=True, encoding="mbcs", errors="replace",
    ).stdout
    found = []
    for line in out.splitlines():
        cells = [c.strip('"') for c in line.split('","')]
        if len(cells) > 1 and cells[0].lower() == image.lower():
            found.append(int(cells[1]))
    return found


def command_line(pid: int) -> tuple[str | None, str]:
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    ntdll = ctypes.WinDLL("ntdll")
    k32.OpenProcess.restype = wt.HANDLE
    handle = k32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        return None, f"OpenProcess 失败 err={ctypes.get_last_error()}"
    try:
        size = wt.ULONG(0)
        ntdll.NtQueryInformationProcess(handle, PROCESS_COMMAND_LINE_INFORMATION, None, 0, ctypes.byref(size))
        if size.value == 0:
            return None, "拿不到长度"
        buf = ctypes.create_string_buffer(size.value)
        status = ntdll.NtQueryInformationProcess(
            handle, PROCESS_COMMAND_LINE_INFORMATION, buf, size, ctypes.byref(size)
        )
        if status != 0:
            return None, f"NtQueryInformationProcess status=0x{status & 0xFFFFFFFF:08x}"
        us = UNICODE_STRING.from_buffer(buf)
        return ctypes.wstring_at(us.Buffer, us.Length // 2), "ok"
    finally:
        k32.CloseHandle(handle)


def is_admin() -> bool:
    try:
        return bool(ctypes.windll.shell32.IsUserAnAdmin())
    except Exception:
        return False


def lcu_auth() -> tuple[dict | None, dict]:
    info: dict = {"kotone_probe_is_admin": is_admin()}
    ux = pids("LeagueClientUx.exe")
    info["LeagueClientUx_pids"] = ux
    if not ux:
        return None, info
    cmd, status = command_line(ux[0])
    info["command_line_read"] = status
    if not cmd:
        return None, info
    args = dict(re.findall(r'--([\w-]+)=("[^"]*"|\S+)', cmd))
    args = {k: v.strip('"') for k, v in args.items()}
    # 只记参数名和不敏感的值
    info["arg_names"] = sorted(args)
    for key in ("app-port", "rso_platform_id", "region", "locale", "riotclient-app-port"):
        if key in args:
            info[key] = args[key]
    if "app-port" not in args or "remoting-auth-token" not in args:
        return None, info
    return {"port": args["app-port"], "token": args["remoting-auth-token"]}, info


# ---------- HTTP ----------

def get(url: str, auth: dict | None = None, timeout: float = 10.0):
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    if auth:
        cred = base64.b64encode(f"riot:{auth['token']}".encode()).decode()
        req.add_header("Authorization", f"Basic {cred}")
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(req, context=CTX, timeout=timeout) as resp:
            body = resp.read()
            status = resp.status
    except urllib.error.HTTPError as e:
        body, status = e.read(), e.code
    except Exception as e:  # 连不上、超时
        return None, f"{type(e).__name__}: {e}", time.perf_counter() - started
    ms = time.perf_counter() - started
    try:
        return json.loads(body or b"null"), status, ms
    except ValueError:
        return body.decode("utf-8", "replace")[:500], status, ms


def lcu(auth, path, **kw):
    return get(f"https://127.0.0.1:{auth['port']}{path}", auth, **kw)


def live(path, **kw):
    return get(f"https://127.0.0.1:2999{path}", **kw)


def save(out: Path, name: str, data) -> None:
    out.mkdir(parents=True, exist_ok=True)
    (out / name).write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")


def find_keys(data, pattern: str, path="") -> list[str]:
    hits = []
    if isinstance(data, dict):
        for k, v in data.items():
            p = f"{path}.{k}"
            if re.search(pattern, k, re.I):
                hits.append(p)
            hits += find_keys(v, pattern, p)
    elif isinstance(data, list):
        for i, v in enumerate(data[:12]):
            hits += find_keys(v, pattern, f"{path}[{i}]")
    return hits


# ---------- 一次探测 ----------

def probe_once(out: Path) -> dict:
    report: dict = {}
    auth, info = lcu_auth()
    report["lcu"] = info
    if auth:
        phase, st, ms = lcu(auth, "/lol-gameflow/v1/gameflow-phase")
        report["lcu"]["phase"] = phase
        session, st, ms = lcu(auth, "/lol-gameflow/v1/session")
        save(out, "lcu-gameflow-session.json", session)
        if isinstance(session, dict):
            gd = session.get("gameData") or {}
            q = gd.get("queue") or {}
            report["lcu"]["queue"] = {k: q.get(k) for k in ("id", "gameMode", "type", "description")}
            report["lcu"]["teamOne"] = len(gd.get("teamOne") or [])
            report["lcu"]["teamTwo"] = len(gd.get("teamTwo") or [])
            report["lcu"]["playerChampionSelections"] = len(gd.get("playerChampionSelections") or [])
            report["lcu"]["team_keys"] = sorted((gd.get("teamOne") or [{}])[0].keys()) if gd.get("teamOne") else []
        me, st, ms = lcu(auth, "/lol-summoner/v1/current-summoner")
        save(out, "lcu-current-summoner.json", me)
        puuid = me.get("puuid") if isinstance(me, dict) else None
        if puuid:
            hist, st, ms = lcu(
                auth, f"/lol-match-history/v1/products/lol/{puuid}/matches?begIndex=0&endIndex=19", timeout=30
            )
            save(out, "lcu-match-history-self.json", hist)
            games = ((hist or {}).get("games") or {}).get("games") if isinstance(hist, dict) else None
            report["lcu"]["self_history"] = {
                "status": st, "ms": round(ms * 1000), "games": len(games or []),
                "queues": sorted({g.get("queueId") for g in (games or [])}),
            }
            if games:
                gid = games[0].get("gameId")
                game, st, ms = lcu(auth, f"/lol-match-history/v1/games/{gid}", timeout=30)
                save(out, "lcu-game-detail.json", game)
                report["lcu"]["game_detail"] = {"status": st, "ms": round(ms * 1000),
                                                "participants": len((game or {}).get("participants") or [])}
        # 这局的 10 个人：隐私设置、战绩能不能查到、要多久
        if isinstance(session, dict):
            gd = session.get("gameData") or {}
            scouted = []
            for side in ("teamOne", "teamTwo"):
                for member in gd.get(side) or []:
                    pu = member.get("puuid")
                    if not pu:
                        continue
                    summ, sst, _ = lcu(auth, f"/lol-summoner/v2/summoners/puuid/{pu}", timeout=10)
                    hist, hst, hms = lcu(
                        auth, f"/lol-match-history/v1/products/lol/{pu}/matches?begIndex=0&endIndex=19", timeout=30
                    )
                    games = ((hist or {}).get("games") or {}).get("games") if isinstance(hist, dict) else None
                    scouted.append({
                        "side": side,
                        "championId": member.get("championId"),
                        "summoner_status": sst,
                        "privacy": summ.get("privacy") if isinstance(summ, dict) else None,
                        "history_status": hst,
                        "history_ms": round(hms * 1000),
                        "games": len(games or []),
                        "error": (hist or {}).get("message") if isinstance(hist, dict) and not games else None,
                    })
                    save(out, f"scout-{side}-{len(scouted)}.json", {"summoner": summ, "history": hist})
            report["lcu"]["scout"] = scouted
        # 局内工具接口：每个都存一份，看有没有冷却
        if auth:
            for path in ("/lol-gameflow/v1/gameflow-metadata/player-status", "/lol-champ-select/v1/session"):
                body, st, _ = lcu(auth, path, timeout=5)
                save(out, "lcu" + path.replace("/", "_") + ".json", {"status": st, "body": body})
    data, st, ms = live("/liveclientdata/allgamedata", timeout=5)
    report["live"] = {"status": st, "ms": round(ms * 1000)}
    if isinstance(data, dict):
        save(out, "live-allgamedata.json", data)
        players = data.get("allPlayers") or []
        report["live"]["gameTime"] = (data.get("gameData") or {}).get("gameTime")
        report["live"]["gameMode"] = (data.get("gameData") or {}).get("gameMode")
        report["live"]["players"] = [
            {
                "team": p.get("team"),
                "champion": p.get("championName"),
                "raw": p.get("rawChampionName"),
                "spells": [
                    (p.get("summonerSpells") or {}).get(slot, {}).get("displayName")
                    for slot in ("summonerSpellOne", "summonerSpellTwo")
                ],
                "position": p.get("position"),
                "items": len(p.get("items") or []),
            }
            for p in players
        ]
        report["live"]["cooldown_like_keys"] = find_keys(data, r"cool|cd|ready|charge|recharge")[:40]
        swagger, st, ms = live("/swagger/v3/openapi.json", timeout=5)
        if swagger is not None:
            save(out, "live-openapi.json", swagger)
            text = json.dumps(swagger)
            report["live"]["openapi_paths"] = len((swagger or {}).get("paths") or {}) if isinstance(swagger, dict) else None
            report["live"]["openapi_mentions_cooldown"] = sorted(set(re.findall(r'"(\w*[Cc]ooldown\w*)"', text)))[:40]
        events, st, ms = live("/liveclientdata/eventdata", timeout=5)
        save(out, "live-eventdata.json", events)
        active, st, ms = live("/liveclientdata/activeplayer", timeout=5)
        save(out, "live-activeplayer.json", active)
        report["live"]["activeplayer_cooldown_keys"] = find_keys(active, r"cool|cd")[:20]
        # openapi 里列的每个 GET 都读一遍（带 riotId 参数的用第一个敌人），找冷却相关的字段
        if isinstance(swagger, dict):
            enemy = next((p for p in players if p.get("team") != (active or {}).get("team")
                          and p.get("riotId") != (active or {}).get("riotId")), players[-1] if players else {})
            rid = urllib.parse.quote(enemy.get("riotId") or enemy.get("summonerName") or "")
            hits = {}
            for path, ops in (swagger.get("paths") or {}).items():
                if "get" not in ops or "{" in path:
                    continue
                url = path + (f"?riotId={rid}" if "riotId" in json.dumps(ops.get("get")) else "")
                body, st, _ = live(url, timeout=5)
                save(out, "live" + path.replace("/", "_") + ".json", {"status": st, "body": body})
                keys = find_keys(body, r"cool|cd|ready|charge|recharge|timer")
                if keys:
                    hits[path] = keys[:10]
            report["live"]["all_get_paths_cooldown_keys"] = hits
    return report


def watch(out: Path, seconds: int) -> None:
    """记录阶段变化和 2999 的可用性变化，用来看加载界面期间返回什么。"""
    log = []
    last = None
    end = time.time() + seconds
    auth = None
    while time.time() < end:
        if auth is None:
            auth, _ = lcu_auth()
        phase = None
        if auth:
            phase, st, _ = lcu(auth, "/lol-gameflow/v1/gameflow-phase", timeout=3)
            if st != 200:
                auth = None
        data, st, ms = live("/liveclientdata/gamestats", timeout=3)
        players, pst, _ = live("/liveclientdata/playerlist", timeout=3) if st == 200 else (None, None, 0)
        state = (
            phase,
            st if isinstance(st, int) else str(st).split(":")[0],
            len(players) if isinstance(players, list) else None,
        )
        if state != last:
            entry = {"t": time.strftime("%H:%M:%S"), "phase": phase, "live_status": state[1],
                     "gameTime": data.get("gameTime") if isinstance(data, dict) else None,
                     "players": state[2]}
            print(json.dumps(entry, ensure_ascii=False), flush=True)
            log.append(entry)
            last = state
            if phase == "InProgress" and auth:
                session, _, _ = lcu(auth, "/lol-gameflow/v1/session", timeout=5)
                save(out, f"watch-session-{entry['t'].replace(':', '')}.json", session)
            if isinstance(players, list):
                save(out, f"watch-playerlist-{entry['t'].replace(':', '')}.json", players)
        time.sleep(1.5)
    save(out, "watch-log.json", log)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--watch", type=int, default=0, help="盯多少秒")
    a = ap.parse_args()
    out = ROOT / "target" / "lol-probe" / time.strftime("%Y%m%d-%H%M%S")
    if a.watch:
        watch(out, a.watch)
    report = probe_once(out)
    save(out, "report.json", report)
    print(json.dumps(report, ensure_ascii=False, indent=2))
    print(f"写到 {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
