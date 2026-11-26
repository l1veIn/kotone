"""Real-machine test of the interpreter (同传, ADR 021) as a background task (ADR 029 phase 2) on an isolated Kotone.

琴音 (mock AI, scripted by mock_interpreter.py) hands “帮我盯着队友” to a background task with `agent.start`; the
background task starts listening to kotone-speaker.exe (a renamed copy of powershell.exe that this script uses to
play an English sentence, at SAPI volume 40); the background task must receive the recognised sentence and show a (mock) translation
with interpreter.show (ADR 043 D11), which pushes it: the main conversation gets one “这局的同传” card with the line (and the
floating window's bubble shows it; not checked here);
“停止同传” must stop it (`agent.stop`) together with its listener. Then the same through the interpreter plugin's page
(ADR 038 step 4): “开始” says the sentence for the player (it shows in the chat), she hands it to the background; “停下”
says “停止同传”.

Messages go in through the chat page (WebView2 DevTools port → `agent_send`): only the player's own words can hand
work to the background, so `assistant.submit` (an external agent's message) would be refused. No OS input.
Needs target/debug/kotone.exe (embedded frontend build). KOTONE_HOME=target/interp-e2e/home, never ~/.kotone.
The SenseVoice model (~240 MB) is downloaded into the isolated home on first run (kept between runs).
usage: python -X utf8 -I scripts/manual-e2e/interpreter_e2e.py
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from voice_turn_real import GAMES, foreground_process  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "interp-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
DEVTOOLS = 9338
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={DEVTOOLS}")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)
PORT = 18766
SENSEVOICE = "sense-voice-zh-en-ja-ko-yue-2024-07-17"
# “队友”的声音从一个改了名的 powershell.exe 放出来（照 recap_real.py 改名的 PING.EXE）：机器上的 python.exe
# 可能不止一个，还可能带着别的出声的程序（2026-11-20 真机：12 个 python.exe，其中一个的子进程在放中文视频）。
SPEAKER = "kotone-speaker.exe"
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
    try:
        envelope = json.loads(out.stdout)
    except json.JSONDecodeError:
        raise RuntimeError(f"{tool}: {out.stdout} {out.stderr}")
    if not envelope.get("ok"):
        raise RuntimeError(f"{tool}: {json.dumps(envelope.get('error'), ensure_ascii=False)}")
    return envelope["result"]


def invoke(command, args):
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
  const r = await page.evaluate(async ([c, a]) => {
    try { return { ok: await window.__TAURI_INTERNALS__.invoke(c, a) }; } catch (e) { return { error: String(e) }; }
  }, [%s, %s]);
  console.log(JSON.stringify(r));
} finally { await browser.close(); }
""" % (DEVTOOLS, json.dumps(command), json.dumps(args, ensure_ascii=False))
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    try:
        return json.loads(out.stdout.strip().splitlines()[-1])
    except Exception:  # noqa: BLE001
        raise RuntimeError((out.stdout + out.stderr)[-2000:])


def wait_until(predicate, seconds, what=None):
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            value = predicate()
        except Exception:  # noqa: BLE001
            value = None
        if value:
            return value
        time.sleep(1)
    if what:
        raise AssertionError(f"timed out waiting for {what}")
    return None


def log_text():
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return f.read()
    except FileNotFoundError:
        return ""


def make_speech(path):
    """English sentence via Windows SAPI (no network)."""
    script = (
        "Add-Type -AssemblyName System.Speech;"
        "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer;"
        "$v = $s.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -like 'en-*' } | Select-Object -First 1;"
        "if ($v) { $s.SelectVoice($v.VoiceInfo.Name) };"
        "$s.Rate = -1; $s.Volume = 40;"
        f"$s.SetOutputToWaveFile('{path}');"
        "$s.Speak('Enemy behind the red house. There are two of them.');"
        "$s.Dispose()"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


def start_speaker(wav, delay_s):
    """Play `wav` twice from a process named SPEAKER, after `delay_s` seconds."""
    exe = os.path.join(WORK, SPEAKER)
    if not os.path.exists(exe):
        shutil.copy(os.path.join(os.environ["SystemRoot"], "System32", "WindowsPowerShell", "v1.0", "powershell.exe"), exe)
    script = (f"Start-Sleep -Seconds {delay_s}; $p = New-Object Media.SoundPlayer '{wav}'; "
              "$p.PlaySync(); Start-Sleep -Milliseconds 1500; $p.PlaySync()")
    return subprocess.Popen([exe, "-NoProfile", "-NonInteractive", "-Command", script])


def guard():
    path = foreground_process()
    name = os.path.basename(path).lower()
    if any(g in name for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


def main():
    guard()  # 会从扬声器放两遍英语句子
    shutil.rmtree(os.path.join(HOME, "agent"), ignore_errors=True)
    shutil.rmtree(os.path.join(WORK, "webview"), ignore_errors=True)
    os.makedirs(HOME, exist_ok=True)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"toolService": {"enabled": True}, "ui": {"firstRunCompleted": True}, "adminPromptDismissed": True,
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0,
                                 "wake": {"enabled": False}}}, f)
    open(os.path.join(WORK, "requests.jsonl"), "w").close()
    mock = subprocess.Popen([sys.executable, "-X", "utf8", "-I", os.path.join(HERE, "mock_interpreter.py"), str(PORT), WORK],
                            env=dict(os.environ, MOCK_LISTEN_APP=SPEAKER))
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    player = None
    try:
        wait_until(lambda: "ok" in invoke("agent_state", {}), 90, "the settings page")

        def sensevoice_ready():
            m = next(m for m in call("models.list", {"category": "asr"})["models"] if m["id"] == SENSEVOICE)
            return m.get("state") == "ready" or m.get("installed") is True or m.get("downloaded") is True
        if not sensevoice_ready():
            print("downloading SenseVoice into the isolated home ...")
            call("models.download", {"id": SENSEVOICE, "category": "asr"})
            wait_until(sensevoice_ready, 900, "SenseVoice download")
        call("connections.upsert", {"connection": {"id": "mock", "displayName": "mock", "kind": "remote", "provider": "custom",
                                                    "baseUrl": f"http://127.0.0.1:{PORT}/v1", "model": "mock"}, "apiKey": "sk-local-fixture"})
        call("settings.update", {"patch": {"assistant": {"connectionId": "mock", "defaultDelivery": ["danmaku"]},
                                           "agent": {"progressSpeech": False}}})
        speech = os.path.join(WORK, "speech.wav")
        make_speech(speech)
        player = start_speaker(speech, 12)

        t0 = time.time()
        r = invoke("agent_send", {"text": "帮我盯着队友"})
        check("typed request accepted", "ok" in r, json.dumps(r, ensure_ascii=False)[:200])
        main_id = r["ok"]["conversationId"]

        def card():
            conv = invoke("agent_conversation", {"id": main_id}).get("ok") or {}
            return next((a for a in conv.get("subagents", []) if a["name"] == "interpreter"), None)
        c = wait_until(card, 60, "the interpreter card")
        check("the interpreter runs as a background task (card on the main conversation)", c,
              f"{time.time() - t0:.1f} s, status {c.get('status')}")
        check(f"listening to {SPEAKER}", wait_until(lambda: f"listens to {SPEAKER}" in log_text(), 60))
        listed = [x["id"] for x in (invoke("agent_state", {}).get("ok") or {}).get("conversations", [])]
        check("the background record is not in the conversation list", c["conversationId"] not in listed)

        # 在全部听到的句子里找这段英语。
        def heard():
            msgs = (invoke("agent_subagent_messages", {"conversationId": main_id, "name": "interpreter"}).get("ok")
                    or {}).get("messages", [])
            for i, m in enumerate(msgs):
                if m.get("role") == "user" and m.get("source") == "heard":
                    data = " ".join(a.get("text", "") for a in m.get("attachments", []))
                    said = [t for x in msgs[i + 1:] if x["role"] == "assistant" for t in x.get("toolCalls", [])
                            if t["name"] in ("interpreter.show", "danmaku")]
                    if ("enemy" in data.lower() or "house" in data.lower()) and said:
                        return data, said
            return None
        got = wait_until(heard, 90)
        sentence, said = got or ("", [])
        check("recognised the English sentence", got, sentence)
        check("translation shown with interpreter.show (the plugin's tool in the background)",
              said and said[0]["name"] == "interpreter.show",
              json.loads(said[0]["arguments"]).get("text", "")[:80] if said else "")

        # 推送（ADR 043 D11）：主对话里一张“这局的同传”卡片，卡片里是那一句（译文、原话、在听的程序）
        def lines_card():
            conv = invoke("agent_conversation", {"id": main_id}).get("ok") or {}
            pushes = [m.get("push") for m in conv.get("messages", []) if m.get("role") == "notice" and m.get("push")]
            return [p for p in pushes if p.get("plugin") == "interpreter" and p.get("trigger") == "lines"]
        cards = wait_until(lines_card, 30) or []
        data = ((cards[0].get("card") or {}).get("structuredContent") or {}) if cards else {}
        first = (data.get("lines") or [{}])[0]
        check("one “这局的同传” card in the main conversation", len(cards) == 1 and cards[0].get("title") == "这局的同传",
              f"{len(cards)} card(s)")
        check("the card has the translated line, what was heard and the program",
              first.get("text", "").startswith("（译）") and first.get("app") == SPEAKER and first.get("heard"),
              json.dumps(first, ensure_ascii=False)[:160])
        check("the push's line is the translation (the floating window's bubble)",
              cards and ((cards[0].get("line") or {}).get("cells") or [{}])[0].get("text", "").startswith("（译）"),
              json.dumps((cards[0].get("line") if cards else None), ensure_ascii=False)[:120])
        check("no note for her (strangers' words stay out of her main conversation)",
              not any("这局的同传" in (m.get("text") or "") for m in (invoke("agent_conversation", {"id": main_id}).get("ok") or {})
                      .get("messages", []) if m.get("role") == "user" and m.get("source") == "auto-card"))
        card_now = card() or {}
        check("still running after its first round (it keeps listening)", card_now.get("status") == "running",
              card_now.get("status"))

        invoke("agent_send", {"text": "停止同传"})
        stopped = wait_until(lambda: "background task interpreter stopped" in log_text()
                             and f"listen: {c['conversationId']} stopped" in log_text(), 60)
        check("“停止同传” stopped the background task and its listener", stopped)
        check("the card says stopped", (card() or {}).get("status") == "stopped")

        # 同传插件的页面（ADR 038 第 4 步）：设好听哪个程序，点“开始”= 玩家对她说了这句话（出现在聊天里），
        # 她照样交给后台；点“停下”= “停止同传”。新开一段对话（上一段里已经有叫 interpreter 的后台了）。
        invoke("agent_new_conversation", {})
        call("settings.update", {"patch": {"plugins": {"interpreter": {"settings": {"app": SPEAKER, "target": "中文"}}}}})

        def page(tool):
            r = invoke("plugin_call", {"plugin": "interpreter", "tool": tool, "arguments": {}})
            return (r.get("ok") or {}).get("content") or r
        said = page("interpreter.start").get("said", "")
        check("page: “开始” said the sentence for the player", said.startswith("帮我盯着队友") and SPEAKER in said, said)
        page_id = wait_until(lambda: (invoke("agent_state", {}).get("ok") or {}).get("current"), 10, "current conversation")
        check("page: a new conversation (not the one above)", page_id and page_id != main_id, page_id)

        def page_card():
            conv = invoke("agent_conversation", {"id": page_id}).get("ok") or {}
            return next((a for a in conv.get("subagents", []) if a["name"] == "interpreter"), None)
        conv = invoke("agent_conversation", {"id": page_id}).get("ok") or {}
        mine = [m for m in conv.get("messages", []) if m.get("role") == "user" and m.get("source") == "chat"]
        check("page: it shows in the chat as the player's message", mine and said in mine[0].get("text", ""),
              (mine[0].get("text", "") if mine else "")[:80])
        pc = wait_until(page_card, 60)
        check("page: she handed it to the background (card)", pc, (pc or {}).get("status"))
        listening = wait_until(lambda: SPEAKER in page("interpreter.status").get("listening", []), 60)
        check(f"page: the status shows it listening to {SPEAKER}", listening, page("interpreter.status").get("listening"))
        page("interpreter.stop")
        check("page: “停下” stopped the background task", wait_until(lambda: (page_card() or {}).get("status") == "stopped", 60),
              (page_card() or {}).get("status"))
        check("page: nothing is listening any more", wait_until(lambda: not page("interpreter.status").get("listening"), 30),
              page("interpreter.status").get("listening"))
    finally:
        if player:
            try:
                player.wait(timeout=60)
            except Exception:  # noqa: BLE001
                player.kill()
        try:
            call("app.quit")
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()
        mock.terminate()
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
