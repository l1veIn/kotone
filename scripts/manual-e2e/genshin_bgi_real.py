"""Real-machine run of the genshin plugin (ADR 047 phase 3) against a FAKE BetterGI, on an isolated Kotone.

Never touches the player's BetterGI or Genshin: the install is synthetic under target/genshin-e2e/BetterGI/ (a fake
BetterGI.exe — `cargo build -p kotone-plugin-genshin --example fake_bettergi` — that only writes its arguments to
fake-bettergi-args.jsonl and exits; a synthetic User/ with a One Dragon config, a config group, a plan, config.json, a log).
KOTONE_HOME=target/genshin-e2e/home, `--background`, the plugin on and pointed at the fake install (plugins.genshin.settings.path),
permissions allow-all except input tools. No OS input, no game, no network.

Checks:
R1. genshin.status finds the fake install, BetterGI not running, gives the webhook address (127.0.0.1, with a token);
R2. genshin.list lists the synthetic config / group / plan;
R3. genshin.run {dragon, 默认配置} starts the fake exe with exactly `startOneDragon 默认配置` (unelevated start, ADR 044);
R4. POST dragon.start / dragon.end (BetterGI's webhook JSON, with a fake screenshot) to the address → 200; a wrong token → 404;
R5. genshin.status: the run is succeeded, by Kotone, with the right name; the push card "BetterGI 的运行" is in a
    conversation file (one card, same key, updated to succeeded) — or the log says why it wasn't pushed;
R6. the event genshin.run_finished ran an automation (on {event genshin.run_finished, game genshin}). The game hub only takes
    events for a running game (ADR 036), so a renamed PING.EXE plays “YuanShen.exe” for this part (not the game);
R7. with the genshin profile's allowAutomationInput off, genshin.run refuses (not_allowed) and nothing starts;
R8. genshin.stop says there is no stop API (nothing is ended).
usage: python -X utf8 -I scripts/manual-e2e/genshin_bgi_real.py   (after building target/debug/kotone.exe and the example)
"""
import glob
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "genshin-e2e")
HOME = os.path.join(WORK, "home")
BGI = os.path.join(WORK, "BetterGI")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
FAKE = os.path.join(REPO, "target", "debug", "examples", "fake_bettergi.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"))
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
ENV.pop("KOTONE_BETTERGI_DIR", None)
GAMES = ["yuanshen.exe", "genshinimpact.exe", "league of legends.exe", "cs2.exe", "dota2.exe", "valorant", "zenlesszonezero.exe"]
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny"}
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def call(tool, args=None, timeout_ms=30_000):
    path = os.path.join(WORK, "args.json")
    with open(path, "w", encoding="utf-8") as f:
        json.dump(args if args is not None else {}, f, ensure_ascii=False)
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", path, "--timeout-ms", str(timeout_ms)],
                         capture_output=True, text=True, encoding="utf-8", env=ENV)
    envelope = json.loads(out.stdout)
    if not envelope.get("ok"):
        raise RuntimeError(f"{tool}: {json.dumps(envelope.get('error'), ensure_ascii=False)}")
    return envelope["result"]


def wait_until(predicate, seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            value = predicate()
        except Exception:  # noqa: BLE001
            value = None
        if value:
            return value
        time.sleep(1)
    return None


def log_lines(word):
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return [line.strip() for line in f if word in line][-5:]
    except FileNotFoundError:
        return []


def running(name):
    out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, errors="replace").stdout.lower()
    return name.lower() in out


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def fake_install():
    shutil.rmtree(BGI, ignore_errors=True)
    os.makedirs(BGI)
    shutil.copy(FAKE, os.path.join(BGI, "BetterGI.exe"))
    write(os.path.join(BGI, "User", "OneDragon", "默认配置.json"), "{}")
    write(os.path.join(BGI, "User", "ScriptGroup", "锄地.json"), json.dumps({"name": "锄地", "projects": [{}]}, ensure_ascii=False))
    write(os.path.join(BGI, "User", "Pulonia", "plans", "pln_e2e.json"), json.dumps({"id": "pln_e2e", "name": "早上"}, ensure_ascii=False))
    write(os.path.join(BGI, "User", "config.json"), json.dumps({
        "notificationConfig": {"webhookEnabled": False, "webhookEndpoint": "", "webhookSendTo": ""},
        "selectedOneDragonFlowConfigName": "默认配置", "hotKeyConfig": {"cancelTaskHotkey": ""}}, ensure_ascii=False, indent=2))
    write(os.path.join(BGI, "log", "better-genshin-impact20261128.log"), "[09:00:00.000] [INF] [Primary:S1:P1:T1] X\n一条龙启动\n\n")


def post(url, body):
    data = json.dumps(body, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code


def push_cards():
    cards = []
    for path in glob.glob(os.path.join(HOME, "**", "conversations", "*.json"), recursive=True):
        try:
            with open(path, encoding="utf-8") as f:
                conv = json.load(f)
        except Exception:  # noqa: BLE001
            continue
        for m in conv.get("messages", []):
            push = m.get("push") if isinstance(m, dict) else None
            if push and push.get("plugin") == "genshin":
                cards.append(push)
    return cards


def main():
    if any(running(g) for g in GAMES):
        raise SystemExit("STOP: a game is running; not testing")
    if running("BetterGI.exe"):
        raise SystemExit("STOP: a BetterGI.exe is running (the player's?); the plugin would see it as open")
    for path in (EXE, FAKE):
        if not os.path.isfile(path):
            raise SystemExit(f"missing {path}: build it first")
    shutil.rmtree(HOME, ignore_errors=True)
    os.makedirs(HOME)
    fake_install()
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"toolService": {"enabled": True}, "ui": {"firstRunCompleted": True}, "adminPromptDismissed": True,
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False},
                   "plugins": {"genshin": {"enabled": True, "riskSeen": 1, "settings": {"path": BGI}}}}, f, ensure_ascii=False)
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    try:
        status = wait_until(lambda: (lambda s: s if (s.get("content") or {}).get("status") == "ok" else None)(call("genshin.status")), 90)
        full = (status or {}).get("structuredContent") or status or {}
        content = (status or {}).get("content") or status or {}
        url = ((full.get("webhook") or {}).get("url")) or ""
        check("R1: genshin.status finds the fake install; not running; a local webhook address with a token",
              content.get("running") is False and url.startswith("http://127.0.0.1:") and "/genshin/" in url,
              f"{(full.get('install') or {}).get('dir')} {url[:40]}…")
        listed = call("genshin.list")
        listed = listed.get("content", listed)
        check("R2: genshin.list lists the synthetic config, group and plan",
              [d["name"] for d in listed.get("oneDragon", [])] == ["默认配置"] and listed.get("groups", [{}])[0].get("name") == "锄地"
              and listed.get("plans", [{}])[0].get("id") == "pln_e2e", json.dumps(listed, ensure_ascii=False)[:200])
        wait_until(lambda: (call("genshin.status").get("structuredContent") or {}).get("webhook", {}).get("listening"), 15)

        ran = call("genshin.run", {"kind": "dragon", "name": "默认配置"})
        ran = ran.get("content", ran)
        args_log = os.path.join(BGI, "fake-bettergi-args.jsonl")
        got = wait_until(lambda: open(args_log, encoding="utf-8").read().strip().splitlines(), 15) or []
        check("R3: genshin.run started the fake BetterGI with exactly `startOneDragon 默认配置`",
              ran.get("status") == "started" and got and json.loads(got[-1])["args"] == ["startOneDragon", "默认配置"],
              f"{ran.get('status')} {got[-1] if got else ''}")

        shot = "/9j/" + "A" * 2000
        codes = [post(url, {"send_to": "", "event": "dragon.start", "result": 0, "timestamp": "2026-11-28T09:00:00",
                            "screenshot": shot, "message": "一条龙启动", "data": None}),
                 post(url, {"send_to": "", "event": "dragon.end", "result": 0, "timestamp": "2026-11-28T09:30:00",
                            "screenshot": shot, "message": "一条龙和配置组任务结束", "data": None})]
        wrong = post(url.rsplit("/", 1)[0] + "/" + "0" * 32, {"event": "dragon.end"})
        check("R4: the webhook took both notices (200) and refused a wrong token (404)", codes == [200, 200] and wrong == 404,
              f"{codes} {wrong}")

        def finished():
            s = call("genshin.status")
            recent = (s.get("structuredContent") or {}).get("recent") or []
            return recent[0] if recent and recent[0].get("status") == "succeeded" else None
        run = wait_until(finished, 15) or {}
        check("R5: status shows the run succeeded, started by Kotone, named 默认配置",
              run.get("byKotone") is True and run.get("name") == "默认配置", json.dumps(run, ensure_ascii=False)[:200])
        cards = wait_until(push_cards, 10) or []
        keys = {c.get("key") for c in cards}
        last = cards[-1] if cards else {}
        state = (((last.get("card") or {}).get("structuredContent")) or {}).get("status")
        check("R5: one “BetterGI 的运行” card in the chat, updated in place to succeeded",
              len(keys) == 1 and state == "succeeded", f"{len(cards)} card message(s), keys {keys}, status {state}; "
              f"log: {log_lines('push: genshin')}")

        # 自动化：当 genshin.run_finished 的时候（程序只把名字写进运行记录，不出声、不操作窗口）
        saved = call("automation.save", {"name": "原神跑完了", "params": [{"name": "name", "type": "text"}],
                                          "action": {"kind": "program", "source": "return args.name"},
                                          "on": {"event": ["genshin.run_finished"], "game": "genshin", "enabled": True}})
        auto_id = (saved.get("content") or saved).get("id") if isinstance(saved, dict) else None
        # 游戏中心只收在跑的游戏的事（ADR 036）：BetterGI 跑一条龙时原神开着。这里用一个改了名的 PING.EXE 充当
        # “原神在跑”（照 recap_real.py 的做法；不是游戏、不碰游戏），ping 一会儿自己结束
        def fired():
            result = call("automation.runs", {})
            runs = (result.get("content") or result).get("runs", [])
            return [r for r in runs if r.get("automationId") == auto_id or (r.get("action") or {}).get("name") == "原神跑完了"]

        fake_game = os.path.join(WORK, "YuanShen.exe")
        shutil.copy(os.path.join(os.environ["SystemRoot"], "System32", "PING.EXE"), fake_game)
        game = subprocess.Popen([fake_game, "-n", "90", "127.0.0.1"], stdout=subprocess.DEVNULL)
        try:
            playing = wait_until(lambda: any(p.get("game") == "genshin" for p in
                                             ((call("game.state", {}).get("content") or call("game.state", {})).get("playing") or [])), 30)
            check("R6: the game hub sees “Genshin” running (a renamed PING.EXE)", playing)
            post(url, {"event": "dragon.start", "result": 0, "message": "一条龙启动"})
            post(url, {"event": "dragon.end", "result": 0, "message": "又跑完了"})
            fired_runs = wait_until(fired, 20) or []
            check("R6: genshin.run_finished ran the automation", fired_runs,
                  json.dumps(fired_runs[:1], ensure_ascii=False)[:300] + f" log: {log_lines('game: genshin')}")
        finally:
            game.kill()

        profile = os.path.join(HOME, "profiles", "genshin.json")
        with open(profile, encoding="utf-8") as f:
            data = json.load(f)
        data["allowAutomationInput"] = False
        with open(profile, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
        before = len(open(args_log, encoding="utf-8").read().strip().splitlines())
        refused = call("genshin.run", {"kind": "dragon"})
        refused = refused.get("content", refused)
        time.sleep(2)
        after = len(open(args_log, encoding="utf-8").read().strip().splitlines())
        check("R7: no automation input for Genshin → genshin.run refuses and starts nothing",
              refused.get("status") == "not_allowed" and before == after, refused.get("text", ""))
        stop = call("genshin.stop")
        stop = stop.get("content", stop)
        check("R8: genshin.stop says there is no stop API", stop.get("status") == "no_stop_api", stop.get("text", ""))
    finally:
        try:
            call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
