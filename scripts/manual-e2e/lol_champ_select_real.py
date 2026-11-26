"""ADR 043 round B real-machine check without opening the game: the champ-select card grows with a ranked draft, and
one-click import writes the rune page, item set and summoner spells to the client — the fake one.

The fake client from lol_tools_real.py (same recording, HTTPS, random token, 127.0.0.1 only) plays a ranked (queue
420) champion select whose step the script moves on: planning → bans → my pick → locked → the game starts. It also
answers what the card and the import read (each champion's data `/lol-game-data/assets/v1/champions/<id>.json` — real
files from CommunityDragon, cached under target/research/cdragon-l1 — my own recent Rift games, the rune pages and
their inventory, the item sets) and records every write (PUT / POST / PATCH) without passing anything anywhere.
Setup as game_events_real.py: debug build copied to target/lol-champ-select-real/bin (a running exe would block other
builds), `--background`, isolated KOTONE_HOME=target/lol-champ-select-real/home, no DeepSeek key (no model call:
pushes don't call the model), the floating window and the push popup off. No OS input at all (the import is called
like the card's button does, through the page-only command `plugin_call` over the DevTools port), so it may run while
the player is on this machine. The 101 tables (Tencent, network) and one OP.GG lane matchup guide (network, on demand,
24 h cache) are real.

Checks:
CS1 planning: the champ-select card is in the chat with stage planning, ban candidates (Tencent 101: what counters my
    pre-pick 亚索 in mid + the lane's strongest, no teammate's champion), team-comp counts (client data), my champion
    (pre-pick: runes with 9 perks, item groups, summoner spells, importable) and control = false.
CS2 import with “操作客户端” off: `lol.import` answers disabled / control_off and the fake client got no write.
CS3 my pick: the same card (one card, same key) moves to picking: no bans, my champion pool from my own Rift games
    against their locked picks, the enemy side of the comp.
CS4 locked: the lane matchup (OP.GG, opponent guessed) and my champion marked locked.
CS5 control on (settings.update): the card says control = true; `lol.import` writes: the rune page (pages are full →
    the current page is replaced, then set current), the item set (read, our set first, the player's own set kept, PUT
    back), the summoner spells (flash stays on the side I keep it). Bodies checked against the card's data.
CS6 the game starts: the card is marked over (still one card) and one short note for her is appended.
CS7 a Mayhem champion select (queue 2400, a new card): my champion's best augments, core items and summoner spells from
    aramhex-data (network: core_builds.json / hero_spells.json); import writes the item set (Howling Abyss, uid
    kotone-<id>-mayhem) and the spells, no rune page (this data has none).
usage: python -X utf8 -I scripts/manual-e2e/lol_champ_select_real.py [--probe DIR] [--gamedata DIR]
"""
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lol_tools_real as lt  # noqa: E402

bt = lt.bt
bt.WORK = os.path.join(lt.REPO, "target", "lol-champ-select-real")
bt.HOME = os.path.join(bt.WORK, "home")
bt.PORT = 9347
bt.guard = lambda: None  # 不发任何系统按键
check = bt.check

RANKED = 420
MAYHEM = 2400
GAME_ID = 9200001
ME = 157  # 亚索，中路
CDRAGON_CACHE = os.path.join(lt.REPO, "target", "research", "cdragon-l1")
CDRAGON = "https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/zh_cn/v1/champions/{}.json"
# 写进假客户端的请求：(方法, 路径, 请求体)
WRITES = []
PERK_PAGES = [
    {"id": 1, "name": "精密", "isEditable": False, "isDeletable": False, "current": False, "isTemporary": False},
    {"id": 2, "name": "主宰", "isEditable": False, "isDeletable": False, "current": False, "isTemporary": False},
    {"id": 50, "name": "我的亚索", "isEditable": True, "isDeletable": True, "current": False, "isTemporary": False},
    {"id": 51, "name": "上单", "isEditable": True, "isDeletable": True, "current": True, "isTemporary": False},
]
OWN_SET = {"uid": "player-own-set", "title": "我自己的方案", "type": "custom", "blocks": []}


def ranked_session(stage):
    """和 plugins/lol/src/pushes/draft.rs 测试里的一样：我是 2 号位中路的亚索。stage: planning / ban / pick / locked。"""
    phase = "PLANNING" if stage == "planning" else "BAN_PICK"
    ban_done = stage in ("pick", "locked")
    picking, locked = stage == "pick", stage == "locked"
    enemies = stage in ("pick", "locked")
    return {
        "gameId": GAME_ID, "localPlayerCellId": 2, "timer": {"phase": phase},
        "myTeam": [
            {"cellId": 0, "championId": 0, "championPickIntent": 266, "assignedPosition": "top", "spell1Id": 4, "spell2Id": 12},
            {"cellId": 1, "championId": 64 if enemies else 0, "championPickIntent": 64, "assignedPosition": "jungle"},
            {"cellId": 2, "championId": ME if locked else 0, "championPickIntent": ME, "assignedPosition": "middle",
             "spell1Id": 14, "spell2Id": 4},
            {"cellId": 3, "championId": 0, "championPickIntent": 222, "assignedPosition": "bottom"},
            {"cellId": 4, "championId": 0, "championPickIntent": 412, "assignedPosition": "utility"},
        ],
        "theirTeam": [
            {"cellId": 5, "championId": 238 if enemies else 0},
            {"cellId": 6, "championId": 86 if enemies else 0},
            {"cellId": 7, "championId": 122 if enemies else 0},
            {"cellId": 8, "championId": 0}, {"cellId": 9, "championId": 0},
        ],
        "bans": {"myTeamBans": [], "theirTeamBans": []},
        "actions": [
            [{"id": 1, "actorCellId": 2, "type": "ban", "championId": 555 if ban_done else 0, "completed": ban_done,
              "isInProgress": stage == "ban"},
             {"id": 2, "actorCellId": 7, "type": "ban", "championId": 777 if ban_done else 0, "completed": ban_done,
              "isInProgress": stage == "ban"}],
            [{"id": 3, "actorCellId": 5, "type": "pick", "championId": 238, "completed": enemies, "isInProgress": False},
             {"id": 4, "actorCellId": 2, "type": "pick", "championId": ME, "completed": locked, "isInProgress": picking}],
        ],
        "benchChampions": [], "isCustomGame": False,
    }


def mayhem_session():
    """海斗的选英雄：我是艾瑞莉娅，备选席两个。"""
    team = [39, 223, 99, 412, 103]
    return {"gameId": GAME_ID + 1, "localPlayerCellId": 0, "timer": {"phase": "BAN_PICK"},
            "myTeam": [{"cellId": i, "championId": c, "championPickIntent": 0, "assignedPosition": "",
                        "spell1Id": 32, "spell2Id": 4} for i, c in enumerate(team)],
            "theirTeam": [], "actions": [], "bans": {"myTeamBans": [], "theirTeamBans": []},
            "benchChampions": [{"championId": 901}, {"championId": 50}], "isCustomGame": False}


def my_rift_games():
    """我自己最近的峡谷对局（英雄池用）：亚索 5、阿狸 3、辛德拉 2、乐芙兰 2、卡牌 1，都是单双排。"""
    games, n = [], 0
    for champion, count, wins in ((157, 5, 3), (103, 3, 2), (134, 2, 1), (7, 2, 1), (4, 1, 0)):
        for k in range(count):
            n += 1
            games.append({"gameId": 7000000 + n, "queueId": RANKED, "gameMode": "CLASSIC", "gameType": "MATCHED_GAME",
                          "gameCreation": 1_790_000_000_000 - n * 3_600_000, "gameDuration": 1800,
                          "participants": [{"championId": champion, "spell1Id": 4, "spell2Id": 14,
                                            "stats": {"win": k < wins, "kills": 5, "deaths": 4, "assists": 6}}]})
    return {"games": {"games": games}}


def champion_file(champion_id):
    """一个英雄的客户端资料（阵容用 tacticalInfo / playstyleInfo / roles）：CommunityDragon 上的真文件，缓存在本机。"""
    path = os.path.join(CDRAGON_CACHE, f"{champion_id}.json")
    if not os.path.exists(path):
        os.makedirs(CDRAGON_CACHE, exist_ok=True)
        try:
            request = urllib.request.Request(CDRAGON.format(champion_id), headers={"User-Agent": "kotone-test"})
            with urllib.request.urlopen(request, timeout=20) as r:
                data = r.read()
            with open(path, "wb") as f:
                f.write(data)
        except Exception as e:  # noqa: BLE001
            print(f"   (CommunityDragon {champion_id}: {e})")
            return None
    return lt.read(path)


class Draft(lt.Recording):
    """The recording's player, in a ranked champion select the script moves on."""

    stage = None  # None = 房间；planning / ban / pick / locked；game = 开始了

    def phase(self):
        if self.stage is None:
            return "Lobby"
        return "InProgress" if self.stage == "game" else "ChampSelect"

    def lcu(self, path):
        bare = path.partition("?")[0]
        if bare == "/lol-gameflow/v1/gameflow-phase":
            return self.phase()
        if bare == "/lol-gameflow/v1/session":
            session = json.loads(json.dumps(self.session))
            session["gameData"]["queue"]["id"] = MAYHEM if self.stage == "mayhem" else RANKED
            session["gameData"]["gameId"] = GAME_ID + 1 if self.stage == "mayhem" else GAME_ID
            return session
        if bare == "/lol-champ-select/v1/session":
            if self.stage == "mayhem":
                return mayhem_session()
            return ranked_session(self.stage) if self.stage not in (None, "game") else None
        if bare.startswith("/lol-game-data/assets/v1/champions/") and bare.endswith(".json"):
            return champion_file(bare.rsplit("/", 1)[1][:-5])
        if bare == f"/lol-match-history/v1/products/lol/{self.me.get('puuid')}/matches":
            return my_rift_games()
        if bare == "/lol-perks/v1/pages":
            return PERK_PAGES
        if bare == "/lol-perks/v1/inventory":
            return {"canAddCustomPage": False, "ownedPageCount": 2}
        if bare == f"/lol-item-sets/v1/item-sets/{self.me.get('summonerId')}/sets":
            return {"accountId": self.me.get("accountId"), "itemSets": [OWN_SET], "timestamp": 1}
        return super().lcu(path)


def _write(handler, method):
    length = int(handler.headers.get("Content-Length") or 0)
    body = handler.rfile.read(length).decode("utf-8", errors="replace")
    if handler.headers.get("Authorization") != lt.AUTH:
        return handler.reply(401, {"message": "bad auth"})
    path = handler.path.partition("?")[0]
    if method == "POST" and path.startswith("/lol-chat/"):
        return ORIGINAL_POST(handler)
    WRITES.append((method, path, body))
    if method == "POST" and path == "/lol-perks/v1/pages":
        return handler.reply(200, {"id": 501, "name": json.loads(body or "{}").get("name")})
    handler.reply(200, {})


ORIGINAL_POST = lt.Handler.do_POST


def _post(self):
    if self.path.partition("?")[0].startswith("/lol-chat/"):
        return ORIGINAL_POST(self)
    # 先读掉请求体再交给 _write（ORIGINAL_POST 自己读）
    return _write(self, "POST")


lt.Handler.do_PUT = lambda self: _write(self, "PUT")  # noqa: E731
lt.Handler.do_PATCH = lambda self: _write(self, "PATCH")  # noqa: E731
lt.Handler.do_POST = _post


def cards():
    """当前对话里选英雄的推送（Notice 的 push）。"""
    cid = bt.state().get("current") or ""
    msgs = bt.conversation(cid).get("messages", []) if cid else []
    return [m["push"] for m in msgs if m.get("role") == "notice" and (m.get("push") or {}).get("trigger") == "champ-select"]


def panel():
    found = cards()
    return ((found[-1].get("card") or {}).get("structuredContent") or {}) if found else {}


def wait_panel(predicate, timeout=40):
    return bt.wait_until(lambda: (lambda p: p if p and predicate(p) else None)(panel()), timeout, 1)


def import_now():
    r = bt.invoke("plugin_call", {"plugin": "lol", "tool": "lol.import", "arguments": {}})
    if "error" in r:
        return {"status": "invoke_error", "text": r["error"]}
    content = (r.get("ok") or {}).get("content")
    return content if isinstance(content, dict) else json.loads(content or "{}")


def log_text():
    try:
        return open(os.path.join(bt.HOME, "kotone.log"), encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        return ""


def main():
    shutil.rmtree(bt.WORK, ignore_errors=True)
    os.makedirs(bt.HOME)
    os.makedirs(os.path.join(bt.WORK, "bin"))
    bt.EXE = shutil.copy2(bt.EXE, os.path.join(bt.WORK, "bin", "kotone.exe"))
    lt.REC = Draft(lt.PROBE, lt.GAMEDATA)
    server, port = lt.serve()
    bt.ENV = dict(os.environ, KOTONE_HOME=bt.HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(bt.WORK, "webview"),
                  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={bt.PORT}",
                  KOTONE_LOL_LCU=f"{port}:{lt.TOKEN}", KOTONE_LOL_LIVE=f"https://127.0.0.1:{port}")
    bt.ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
    with open(os.path.join(bt.HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}, "bubble": {"enabled": False}},
                   # 推送不弹悬浮窗的小窗：玩家可能正在用这台电脑
                   "agent": {"autoCards": {"popup": False},
                             "permissions": {"mode": "allow-all", "rules": lt.DENIED}},
                   # “操作客户端”默认关（CS5 再打开）
                   "plugins": {"lol": {"sources": {"control": False}}}}, f)
    app = subprocess.Popen([bt.EXE, "--background"], env=bt.ENV)
    try:
        bt.wait_ui()
        bt.invoke("agent_new_conversation", {})
        bt.wait_until(lambda: lt.HITS["/lol-gameflow/v1/gameflow-phase"] > 0, 30, 0.5)

        # CS1 预选
        lt.REC.stage = "planning"
        p = wait_panel(lambda p: p.get("stage") == "planning" and p.get("mine"))
        bans = ((p or {}).get("bans") or {}).get("list") or []
        mine = (p or {}).get("mine") or {}
        ally = ((p or {}).get("comp") or {}).get("ally") or {}
        check("CS1: planning → the card is in the chat with stage planning", p and cards()[0]["card"]["ui"]["uri"] == "ui://kotone/lol/champ-select",
              json.dumps(p, ensure_ascii=False)[:300] if p else [l for l in log_text().splitlines() if "lol:" in l][-5:])
        check("CS1: ban candidates from Tencent 101, none of them my team's",
              bans and p["bans"]["source"] == "qq101" and p["bans"]["lane"] == "中路"
              and not ({b["champion"]["id"] for b in bans} & {266, 64, ME, 222, 412}),
              json.dumps(bans, ensure_ascii=False)[:300])
        print("   bans:", [(b["champion"]["short"], b["reason"], b.get("winRate"), b.get("tier")) for b in bans])
        check("CS1: team comp counted from the client's champion data (5 of my team)",
              ally.get("count") == 5 and p["comp"]["source"] == "client" and lt.HITS["/lol-game-data/assets/v1/champions/157.json"] >= 1,
              json.dumps(p.get("comp") if p else None, ensure_ascii=False))
        runes = (mine.get("runes") or {}).get("perks") or []
        check("CS1: my champion (pre-pick): 101 runes (9), item groups, summoner spells, importable",
              mine.get("champion", {}).get("id") == ME and not mine.get("locked") and len(runes) == 9
              and len(mine.get("items") or []) >= 3 and len(mine.get("spells") or []) == 2
              and mine.get("importable") == {"runes": True, "items": True, "spells": True} and p.get("control") is False,
              json.dumps(mine, ensure_ascii=False)[:400])

        # CS2 操作客户端关着：不写
        out = import_now()
        check("CS2: import with the switch off → disabled / control_off, nothing written",
              out.get("status") == "disabled" and out.get("reason") == "control_off" and not WRITES,
              json.dumps(out, ensure_ascii=False)[:200])

        # CS3 轮到我选
        lt.REC.stage = "pick"
        p = wait_panel(lambda p: p.get("stage") == "picking")
        pool = ((p or {}).get("pool") or {}).get("list") or []
        check("CS3: my pick → same card, no bans, my pool from my own Rift games",
              p and len(cards()) == 1 and not p.get("bans") and pool
              and {x["champion"]["id"] for x in pool} <= {157, 103, 134, 7, 4},
              json.dumps(p.get("pool") if p else None, ensure_ascii=False)[:400])
        print("   pool:", [(x["champion"]["short"], x["games"], [g["champion"]["short"] for g in x["good"]],
                            [b["champion"]["short"] for b in x["bad"]], x.get("tier")) for x in pool])
        check("CS3: the enemy side of the comp (3 locked)", ((p or {}).get("comp") or {}).get("enemy", {}).get("count") == 3,
              json.dumps((p or {}).get("comp"), ensure_ascii=False))

        # CS4 锁定
        lt.REC.stage = "locked"
        p = wait_panel(lambda p: p.get("stage") == "locked" and (p.get("mine") or {}).get("locked"))
        m = (p or {}).get("matchup")
        check("CS4: locked → my champion marked locked, the pool is gone", p and not p.get("pool"),
              json.dumps(p, ensure_ascii=False)[:200] if p else "")
        m = m or (wait_panel(lambda p: p.get("matchup"), 25) or {}).get("matchup")
        check("CS4: the lane matchup from OP.GG (opponent guessed)",
              m and m.get("source") == "opgg" and m.get("guessed") is True and m.get("opponent", {}).get("id") in (238, 86, 122),
              json.dumps(m, ensure_ascii=False)[:300] if m else [l for l in log_text().splitlines() if "OP.GG" in l][-3:])
        if m:
            print("   matchup:", m.get("opponent", {}).get("short"), m.get("laneAdvantage"), m.get("soloKillAdvantage"),
                  m.get("playStyle"), (m.get("tip") or "")[:80])

        # CS5 打开“操作客户端”，一键导入
        bt.call("settings.update", {"patch": {"plugins": {"lol": {"sources": {"control": True}}}}})
        p = wait_panel(lambda p: p.get("control") is True, 20)
        check("CS5: the card follows the switch (control = true)", p, "")
        mine = (p or {}).get("mine") or {}
        out = import_now()
        print("   import:", json.dumps(out, ensure_ascii=False)[:400])
        check("CS5: import ok: rune page replaced (pages full), item set written, spells written",
              out.get("status") == "ok" and out["runes"]["result"] == "replaced" and out["runes"]["old"] == "上单"
              and out["items"]["result"] == "written" and out["spells"]["result"] == "written",
              json.dumps(out, ensure_ascii=False)[:300])
        sent = [(mth, path) for mth, path, _ in WRITES]
        sid = lt.REC.me.get("summonerId")
        check("CS5: the writes the fake client got",
              sent == [("PUT", "/lol-perks/v1/pages/51"), ("PUT", "/lol-perks/v1/currentpage"),
                       ("PUT", f"/lol-item-sets/v1/item-sets/{sid}/sets"),
                       ("PATCH", "/lol-champ-select/v1/session/my-selection")], str(sent))
        body = {path: json.loads(b or "null") for _, path, b in WRITES}
        page = body.get("/lol-perks/v1/pages/51") or {}
        card_perks = [x["id"] for x in ((mine.get("runes") or {}).get("perks") or [])]
        check("CS5: the rune page is the card's (9 perks, styles), named Kotone 亚索 中路, set current",
              page.get("selectedPerkIds") == card_perks and page.get("name") == "Kotone 亚索 中路"
              and page.get("primaryStyleId") == mine["runes"]["styles"][0]["id"]
              and body.get("/lol-perks/v1/currentpage") == 51, json.dumps(page, ensure_ascii=False)[:300])
        sets = (body.get(f"/lol-item-sets/v1/item-sets/{sid}/sets") or {}).get("itemSets") or []
        check("CS5: the item set (ours first, uid kotone-157-rift, Rift, this champion), the player's own set kept",
              len(sets) == 2 and sets[0]["uid"] == "kotone-157-rift" and sets[0]["associatedChampions"] == [ME]
              and sets[0]["associatedMaps"] == [11] and sets[1]["uid"] == OWN_SET["uid"] and sets[0]["blocks"],
              json.dumps(sets, ensure_ascii=False)[:300])
        spells = body.get("/lol-champ-select/v1/session/my-selection") or {}
        want = [s["id"] for s in mine.get("spells") or []]
        # 我现在是 点燃(14) + 闪现(4)：闪现在 F（第二个）；数据的闪现在前时对调过来
        check("CS5: the summoner spells are the card's, flash kept on my side (F)",
              sorted([spells.get("spell1Id"), spells.get("spell2Id")]) == sorted(want)
              and (4 not in want or spells.get("spell2Id") == 4), json.dumps(spells))

        # CS6 开始游戏：卡片标上结束了，给她一句
        lt.REC.stage = "game"
        p = wait_panel(lambda p: p.get("ended"), 20)
        check("CS6: the game starts → the card is marked over, still one card", p and len(cards()) == 1, "")
        cid = bt.state().get("current") or ""
        msgs = bt.conversation(cid).get("messages", []) if cid else []
        # 给她的那句是一条隐藏的说明（来源 auto-card），话在附件里
        notes = [a.get("text", "") for m in msgs if m.get("role") == "user" for a in m.get("attachments") or []
                 if "选英雄结束了（峡谷）" in a.get("text", "")]
        check("CS6: one short note for her (facts only)", len(notes) == 1, str(notes)[:200])
        if notes:
            print("   note:", notes[0][:200])

        # CS7 海斗：新的一张选英雄卡片，我的英雄是 aramhex-data 的
        WRITES.clear()
        lt.REC.stage = "mayhem"
        p = bt.wait_until(lambda: (lambda c: ((c[-1].get("card") or {}).get("structuredContent") or {})
                                   if len(c) == 2 and (((c[-1].get("card") or {}).get("structuredContent") or {}).get("mine")) else None)(cards()),
                          60, 1)
        mine = (p or {}).get("mine") or {}
        check("CS7: Mayhem → a new card; my champion from aramhex-data: augments, core items, spells, no runes",
              p and p.get("mode") == "mayhem" and mine.get("source") == "aramhex" and mine.get("augments")
              and mine.get("items") and len(mine.get("spells") or []) == 2 and not mine.get("runes")
              and mine.get("importable") == {"runes": False, "items": True, "spells": True},
              json.dumps(mine, ensure_ascii=False)[:400] if mine else json.dumps(p, ensure_ascii=False)[:300])
        if mine:
            print("   mayhem:", [a["name"] for a in mine.get("augments", [])], [[i["name"] for i in g["items"]] for g in mine.get("items", [])],
                  [s_["name"] for s_ in mine.get("spells", [])])
        out = import_now()
        sets = next((json.loads(b)["itemSets"] for mth, path, b in WRITES if path.startswith("/lol-item-sets/")), [])
        check("CS7: Mayhem import: item set for the Howling Abyss + spells, no rune page written",
              out.get("status") == "ok" and out["runes"]["result"] == "no_data" and out["items"]["result"] == "written"
              and out["spells"]["result"] == "written" and not any(path.startswith("/lol-perks/") for _, path, _ in WRITES)
              and sets and sets[0]["uid"] == "kotone-39-mayhem" and sets[0]["associatedMaps"] == [12],
              json.dumps(out, ensure_ascii=False)[:300] + " " + str([(m_, p_) for m_, p_, _ in WRITES]))
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
