"""Real-machine check: the interpreter (同传, ADR 021) uses Kotone's one speech recognition (shared-resources audit §2 #7).

Two isolated Kotone instances (target/interp-asr-real/, `--background`, a copy of target/debug/kotone.exe so other builds
are not blocked), a mock AI (mock_interpreter.py: “帮我盯着队友” → agent.start → voice.listen.start kotone-speaker.exe),
no cost, no network (the mock also answers DeepSeek's speech recognition, `/v1/tools/asr`, with a fixed sentence).

A. Online-ASR player (DeepSeek ASR selected through a fake DeepSeek connection; the models directory is empty):
   (a) 同传 starts without downloading SenseVoice, recognises the teammate through the online recognition (the mock
       gets the audio, the sentence reaches the background task) and the cost is in usage.jsonl as ASR.
B. Local SenseVoice player (models.dir = target/interp-asr-real/models-sv, a copy of the model; mic = CABLE Output):
   (b) with SenseVoice already loaded by “启动本地语音识别”, starting 同传 does not load a second copy
       (private bytes of kotone.exe before / after);
   (d) a call (the chat page's call button, clicked through the WebView2 DevTools port) while 同传 decodes the
       teammate's sentence: both results come, one copy in memory; prints vad→text of the call alone vs. during 同传;
   (c) after “停止本地语音识别” (runtime.stop) 同传 stops too (nothing heard afterwards, memory back, not listening)
       and its background conversation gets a note.
Before the fix (2026-11-25, commit 62557aaf) the four checks failed — that was the reproduction; after the fix
(ADR 039 实施记录“同传用本体的识别”) they pass.

What it does to the machine: NO keyboard or mouse input at all (clicks go into the page through DevTools). It plays
sound: the “teammate” (an English SAPI sentence, SAPI volume 40) from kotone-speaker.exe (a renamed powershell.exe)
on the default speakers three times (A once, B twice), and the player's Chinese fixture (zh-game-3s.wav) into “CABLE Input” (silent).
Her voice is at volume 0. Wake word, hotkeys off; typing tools denied. Never touches ~/.kotone, the player's WebView2
data or credential store (KOTONE_HOME isolates the credential service name). Stops if a game process is running or a
game is in front, and checks again before each sound. Takes target/tmp/real.lock (waits up to 30 minutes).
Needs: target/debug/kotone.exe (embedded frontend), VB-CABLE, Python numpy + sounddevice, node + @playwright/test in
apps/desktop, the SenseVoice model in target/interp-e2e/home/models or target/interp-asr-real/models-sv.
usage: python -X utf8 -I scripts/manual-e2e/interpreter_asr_real.py
"""
import ctypes
import ctypes.wintypes as wt
import json
import os
import re
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "interp-asr-real")
SRC_EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
EXE = os.path.join(WORK, "bin", "kotone.exe")
DEVTOOLS = 9381
MOCK_PORT = 18781
SPEAKER = "kotone-speaker.exe"
SENSEVOICE = "sense-voice-zh-en-ja-ko-yue-2024-07-17"
SENSEVOICE_DIR = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
MODELS_SV = os.path.join(WORK, "models-sv")
MODELS_EMPTY = os.path.join(WORK, "models-empty")
ENGLISH = os.path.join(WORK, "english.wav")
CHINESE = os.path.join(REPO, "crates", "kotone-stt", "tests", "fixtures", "zh-game-3s.wav")
TRIGGER = os.path.join(WORK, "speak.trigger")
STOP = os.path.join(WORK, "speak.stop")
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}
GAMES = ("league of legends", "leagueclient", "riotclient", "valorant", "genshinimpact", "yuanshen", "starrail",
         "zenlesszonezero", "dota2", "cs2", "wow", "overwatch", "pubg", "tslgame", "naraka", "eldenring")
LOCK = os.path.join(REPO, "target", "tmp", "real.lock")
RESULTS = []
HOME = ""
ENV = {}


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""), flush=True)
    return bool(ok)


def info(text):
    print("INFO " + text, flush=True)


# ---------- safety ----------

def foreground_process():
    user32, kernel32 = ctypes.WinDLL("user32"), ctypes.WinDLL("kernel32")
    hwnd = user32.GetForegroundWindow()
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    handle = kernel32.OpenProcess(0x1000, False, pid.value)
    if not handle:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
        return buf.value
    finally:
        kernel32.CloseHandle(handle)


def guard():
    out = subprocess.run(["tasklist", "/fo", "csv", "/nh"], capture_output=True, text=True, errors="replace").stdout.lower()
    running = [g for g in GAMES if g in out]
    if running:
        raise SystemExit(f"STOP: a game is running ({running}); not testing")
    path = foreground_process()
    if any(g in os.path.basename(path).lower() for g in GAMES) or "riot games" in path.lower():
        raise SystemExit(f"STOP: a game is in front ({os.path.basename(path)}); not testing")


# ---------- the isolated instance ----------

def set_home(name):
    global HOME, ENV
    HOME = os.path.join(WORK, name)
    ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview-" + name),
               WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={DEVTOOLS}")
    ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)


def write_config(models_dir):
    shutil.rmtree(HOME, ignore_errors=True)
    shutil.rmtree(ENV["WEBVIEW2_USER_DATA_FOLDER"], ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"toolService": {"enabled": True}, "ui": {"firstRunCompleted": True, "autoStart": False},
                   "adminPromptDismissed": True, "models": {"dir": models_dir},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "speechVolume": 0, "callIdleSeconds": 0,
                                 "wake": {"enabled": False}},
                   "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False}}, f)


def call(tool, args=None, timeout_ms=30_000):
    out = subprocess.run([EXE, "tools", "call", tool, "--args-file", "-", "--timeout-ms", str(timeout_ms)],
                         input=json.dumps(args or {}, ensure_ascii=False), capture_output=True, text=True,
                         encoding="utf-8", env=ENV)
    try:
        envelope = json.loads(out.stdout)
    except json.JSONDecodeError:
        raise RuntimeError(f"{tool}: {out.stdout[-500:]} {out.stderr[-500:]}")
    if not envelope.get("ok"):
        raise RuntimeError(f"{tool}: {json.dumps(envelope.get('error'), ensure_ascii=False)}")
    return envelope["result"]


def page_js(body):
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
const out = {};
try {
%s
} catch (e) {
  out.url = page.url();
  out.text = (await page.evaluate(() => document.body.innerText)).slice(0, 600);
  await page.screenshot({ path: %s }).catch(() => {});
  throw e;
} finally { console.log(JSON.stringify(out)); await browser.close(); }
""" % (DEVTOOLS, body, json.dumps(os.path.join(WORK, "page-failure.png")))
    proc = subprocess.run(["node", "--input-type=module", "-e", code], capture_output=True, text=True,
                          encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    if proc.returncode != 0:
        raise RuntimeError((proc.stdout + proc.stderr)[-2000:])
    return json.loads(proc.stdout.strip().splitlines()[-1])


def invoke(command, args):
    body = """
  out.r = await page.evaluate(async ([c, a]) => {
    try { return { ok: await window.__TAURI_INTERNALS__.invoke(c, a) }; } catch (e) { return { error: String(e) }; }
  }, [%s, %s]);""" % (json.dumps(command), json.dumps(args, ensure_ascii=False))
    return page_js(body)["r"]


def click_call():
    r = page_js("""
const button = page.locator('[data-testid=chat-call], [data-testid=call-kotone]').first();
await button.waitFor({ state: "attached", timeout: 10000 });
out.at = Date.now() / 1000;
await button.evaluate((el) => el.click());
""")
    return r["at"]


def wait_until(predicate, seconds, what=None):
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            value = predicate()
        except Exception:  # noqa: BLE001
            value = None
        if value:
            return value
        time.sleep(0.5)
    if what:
        raise AssertionError(f"timed out waiting for {what}")
    return None


def log_text():
    try:
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            return f.read()
    except FileNotFoundError:
        return ""


def private_mb(pid):
    """Private bytes of the kotone.exe process (MB)."""
    class Counters(ctypes.Structure):
        _fields_ = [("cb", wt.DWORD), ("PageFaultCount", wt.DWORD)] + [
            (n, ctypes.c_size_t) for n in ("PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                                           "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                                           "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage",
                                           "PrivateUsage")]
    kernel32, psapi = ctypes.WinDLL("kernel32"), ctypes.WinDLL("psapi")
    handle = kernel32.OpenProcess(0x1000 | 0x0010, False, pid)
    if not handle:
        return -1.0
    try:
        c = Counters()
        c.cb = ctypes.sizeof(Counters)
        if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(c), c.cb):
            return -1.0
        return c.PrivateUsage / 1024 / 1024
    finally:
        kernel32.CloseHandle(handle)


def settled_mb(pid, seconds=4):
    time.sleep(seconds)
    return private_mb(pid)


def start_app():
    app = subprocess.Popen([EXE, "--background"], env=ENV)
    wait_until(lambda: "ok" in invoke("agent_state", {}), 90, "the settings page")
    return app


def stop_app(app):
    try:
        call("app.quit")
    except Exception as e:  # noqa: BLE001
        print("quit:", e)
    for _ in range(20):
        if app.poll() is not None:
            return
        time.sleep(0.5)
    app.terminate()


def start_speaker():
    """A long-lived “teammate” process: plays english.wav each time TRIGGER appears (it must exist before
    voice.listen.start — processes started later are not recorded, ADR 021 implementation note)."""
    for path in (TRIGGER, STOP):
        if os.path.exists(path):
            os.remove(path)
    exe = os.path.join(WORK, SPEAKER)
    if not os.path.exists(exe):
        shutil.copy(os.path.join(os.environ["SystemRoot"], "System32", "WindowsPowerShell", "v1.0", "powershell.exe"), exe)
    script = (f"$p = New-Object Media.SoundPlayer '{ENGLISH}'; $p.Load(); $end = (Get-Date).AddSeconds(900); "
              f"while ((Get-Date) -lt $end -and -not (Test-Path '{STOP}')) {{ "
              f"if (Test-Path '{TRIGGER}') {{ Remove-Item '{TRIGGER}'; $p.PlaySync() }}; Start-Sleep -Milliseconds 100 }}")
    return subprocess.Popen([exe, "-NoProfile", "-NonInteractive", "-Command", script])


def teammate_speaks():
    guard()
    open(TRIGGER, "w").close()
    return time.time()


def play_into_cable(path, lead_s=0.8):
    import numpy as np
    import sounddevice as sd
    import wave
    guard()
    w = wave.open(path)
    data = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768
    data = np.concatenate([np.zeros(int(16000 * lead_s), dtype=np.float32), data, np.zeros(16000, dtype=np.float32)])
    dev = next(i for i, d in enumerate(sd.query_devices())
               if d["name"].startswith("CABLE Input") and d["max_output_channels"] > 0
               and sd.query_hostapis(d["hostapi"])["name"] == "MME")
    sd.play(data, 16000, device=dev, blocking=True)


def make_english():
    if os.path.exists(ENGLISH):
        return
    script = (
        "Add-Type -AssemblyName System.Speech;"
        "$s = New-Object System.Speech.Synthesis.SpeechSynthesizer;"
        "$v = $s.GetInstalledVoices() | Where-Object { $_.VoiceInfo.Culture.Name -like 'en-*' } | Select-Object -First 1;"
        "if ($v) { $s.SelectVoice($v.VoiceInfo.Name) };"
        "$s.Rate = -1; $s.Volume = 40;"
        "$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono);"
        f"$s.SetOutputToWaveFile('{ENGLISH}', $f);"
        "$s.Speak('Enemy behind the red house. There are two of them.');"
        "$s.Dispose()"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


def setup_connections(asr_deepseek):
    call("connections.upsert", {"connection": {"id": "mock", "displayName": "mock", "kind": "remote", "provider": "custom",
                                                "baseUrl": f"http://127.0.0.1:{MOCK_PORT}/v1", "model": "mock"},
                                 "apiKey": "sk-local-fixture"})
    if asr_deepseek:
        # 只要一条 DeepSeek 连接，DeepSeek 语音识别就“准备好了”；这一段不会真的去识别（指向本机的假服务）。
        call("connections.upsert", {"connection": {"id": "ds-fixture", "displayName": "DeepSeek（假）", "kind": "remote",
                                                    "provider": "deepseek", "baseUrl": f"http://127.0.0.1:{MOCK_PORT}/v1",
                                                    "model": "deepseek-chat"}, "apiKey": "sk-local-fixture"})
    call("settings.update", {"patch": {"assistant": {"connectionId": "mock", "defaultDelivery": ["danmaku"]}}})


def start_interpreter():
    r = invoke("agent_send", {"text": "帮我盯着队友"})
    if "ok" not in r:
        raise RuntimeError(f"agent_send: {r}")
    main_id = r["ok"]["conversationId"]

    def card():
        conv = invoke("agent_conversation", {"id": main_id}).get("ok") or {}
        return next((a for a in conv.get("subagents", []) if a["name"] == "interpreter"), None)
    c = wait_until(card, 60, "the interpreter background task")

    def listen_result():
        msgs = (invoke("agent_subagent_messages", {"conversationId": main_id, "name": "interpreter"}).get("ok")
                or {}).get("messages", [])
        return next((m for m in msgs if m.get("role") == "tool" and m.get("name") == "voice.listen.start"), None)
    return main_id, c, wait_until(listen_result, 60, "the voice.listen.start result")


def heard_sentences(main_id):
    """Every sentence 同传 has heard so far, in order. Sentences that arrive while the background task is busy are
    merged into the waiting message as more attachments (its atMs stays the first one's), so count attachments."""
    msgs = (invoke("agent_subagent_messages", {"conversationId": main_id, "name": "interpreter"}).get("ok")
            or {}).get("messages", [])
    return [a.get("text", "") for m in msgs if m.get("role") == "user" and m.get("source") == "heard"
            for a in m.get("attachments", [])]


def heard_after(main_id, seen, since_s):
    """(seconds since `since_s`, text) of the first new sentence (after the first `seen`) about the red house."""
    new = heard_sentences(main_id)[seen:]
    hit = next((t for t in new if "house" in t.lower() or "enem" in t.lower()), None)
    return (round(time.time() - since_s, 1), hit) if hit else None


def voice_message_after(conv_id, since_ms):
    conv = invoke("agent_conversation", {"id": conv_id}).get("ok") or {}
    for m in conv.get("messages", []):
        if m.get("role") == "user" and int(m.get("atMs", 0)) > since_ms and "打野" in m.get("text", ""):
            return int(m["atMs"]), m.get("text", "")[:60], m.get("source")
    return None


def vad_to_text(mark):
    m = wait_until(lambda: re.search(r"voice-timing: vad→text=(\d+)ms", log_text()[mark:]), 60)
    return int(m.group(1)) if m else None


# ---------- A. online ASR ----------

def phase_online():
    print("\n== A. online recognition (DeepSeek ASR), no SenseVoice on disk ==", flush=True)
    set_home("home-online")
    shutil.rmtree(MODELS_EMPTY, ignore_errors=True)
    os.makedirs(MODELS_EMPTY)
    write_config(MODELS_EMPTY)
    speaker = start_speaker()
    app = start_app()
    try:
        setup_connections(asr_deepseek=True)
        call("models.activate", {"category": "asr", "engineId": "deepseek-asr", "modelId": "deepseek-asr"})
        status = wait_until(lambda: (lambda s: s if s.get("phase") == "running" else None)(call("runtime.status")), 30)
        info(f"runtime: {json.dumps({k: (status or {}).get(k) for k in ('phase', 'engineId', 'modelId', 'asrLocal')})}")
        info(f"models dir: {sorted(os.listdir(MODELS_EMPTY))} (SenseVoice on disk: "
             f"{os.path.isdir(os.path.join(MODELS_EMPTY, SENSEVOICE_DIR))})")
        asr_log = os.path.join(WORK, "asr.jsonl")
        open(asr_log, "w").close()
        main_id, _, result = start_interpreter()
        content = str(result.get("content", ""))
        info(f"voice.listen.start → isError={result.get('isError')}: {content[:160]}")
        started = check("(a) online recognition ready → 同传 starts without downloading SenseVoice",
                        status and not result.get("isError") and '"online":true' in content.replace(" ", ""),
                        content[:160])
        if started:
            seen = len(heard_sentences(main_id))
            t = teammate_speaks()
            heard = wait_until(lambda: heard_after(main_id, seen, t), 40)
            uploads = sum(1 for _ in open(asr_log, encoding="utf-8"))
            asr = [r for r in ledger() if r.get("kind") == "asr" and r.get("atMs", 0) >= t * 1000 - 1000]
            info(f"heard {heard}; {uploads} uploads to the mock DeepSeek ASR; usage: "
                 f"{[(r.get('provider'), r.get('model'), r.get('audioMs'), r.get('cost')) for r in asr]}")
            check("(a) the teammate is recognised by the online recognition, the cost is in usage as ASR",
                  heard and uploads > 0 and any(r.get("provider") == "deepseek" for r in asr),
                  f"heard {heard and heard[1]}, {uploads} uploads, {len(asr)} usage rows")
            info(f"models dir afterwards: {sorted(os.listdir(MODELS_EMPTY))}")
    finally:
        stop_app(app)
        open(STOP, "w").close()
        speaker.wait(timeout=10)


# ---------- B. local SenseVoice ----------

def phase_local():
    print("\n== B. local SenseVoice selected (mic = CABLE Output) ==", flush=True)
    set_home("home-local")
    write_config(MODELS_SV)
    speaker = start_speaker()
    app = start_app()
    pid = app.pid
    try:
        setup_connections(asr_deepseek=False)
        cable = next(d["id"] for d in call("audio.devices")["devices"] if str(d["id"]).startswith("CABLE Output"))
        call("settings.update", {"patch": {"audioDeviceId": cable}})
        call("models.activate", {"category": "asr", "engineId": "sherpa-offline", "modelId": SENSEVOICE})
        if call("runtime.status").get("phase") != "stopped":
            call("runtime.stop")
        idle = settled_mb(pid)
        call("runtime.start", timeout_ms=30_000)
        wait_until(lambda: "runtime warmup ok: sherpa-offline" in log_text(), 60, "SenseVoice warmup")
        core = settled_mb(pid)
        info(f"private bytes: idle {idle:.0f} MB → after 启动本地语音识别 {core:.0f} MB (+{core - idle:.0f})")

        # A call alone (baseline vad→text).
        # 不 reload：2026-11-25 起的构建里 reload 回来是“asset not found: index.html”（窗口地址带 index.html）
        page_js('await page.getByTestId("sidebar-new-chat").evaluate((el) => el.click()); await page.waitForTimeout(1000);')
        mark = len(log_text())
        t0 = int(time.time() * 1000)
        click_call()
        play_into_cable(CHINESE)
        solo = vad_to_text(mark)
        conv_id = wait_until(lambda: (invoke("agent_state", {}).get("ok") or {}).get("current"), 10, "current conversation")
        said = wait_until(lambda: voice_message_after(conv_id, t0), 30)
        info(f"call alone: {said}; vad→text={solo} ms")
        time.sleep(5)

        main_id, card, result = start_interpreter()
        check("同传 started (voice.listen.start ok)", not result.get("isError"), str(result.get("content", ""))[:120])
        both = settled_mb(pid)
        info(f"private bytes: after 同传 starts {both:.0f} MB (+{both - core:.0f} over the core's SenseVoice)")
        check("(b) SenseVoice already loaded → 同传 does not load a second copy (< +50 MB)", both - core < 50,
              f"+{both - core:.0f} MB; the core's copy was +{core - idle:.0f} MB")

        # (d) the call and 同传 at the same time.
        mark = len(log_text())
        t1 = int(time.time() * 1000)
        seen = len(heard_sentences(main_id))
        start = teammate_speaks()
        time.sleep(0.3)
        click_call()
        play_into_cable(CHINESE)
        during = vad_to_text(mark)
        heard = wait_until(lambda: heard_after(main_id, seen, start), 40)
        said = wait_until(lambda: voice_message_after(main_id, t1), 30)
        info(f"same window: 同传 heard {heard} (s after the teammate started, seen by the poll); "
             f"call → {said and (round((said[0] - t1) / 1000, 1), said[1])} s; "
             f"vad→text {solo} ms alone vs {during} ms during 同传")
        check("(d) call + 同传 together: one recognizer in memory (both results came, private bytes as (b))",
              heard and said and both - core < 50,
              f"{both - idle:.0f} MB for recognition (one SenseVoice is about 285 MB)")
        time.sleep(5)

        # (c) 停止本地语音识别.
        call("runtime.stop", timeout_ms=30_000)
        wait_until(lambda: call("runtime.status").get("phase") == "stopped", 30, "runtime stopped")
        stopped = settled_mb(pid)
        listening = invoke("plugin_call", {"plugin": "interpreter", "tool": "interpreter.status", "arguments": {}})
        listening = ((listening.get("ok") or {}).get("content") or {}).get("listening")
        seen = len(heard_sentences(main_id))
        start = teammate_speaks()
        heard = wait_until(lambda: heard_after(main_id, seen, start), 30)
        info(f"after 停止本地语音识别: phase stopped, private {stopped:.0f} MB ({stopped - idle:+.0f} vs idle), "
             f"interpreter.status listening={listening}, {seen} sentences before, then heard={heard}")
        notes = [m.get("text", "") for m in (invoke("agent_subagent_messages",
                 {"conversationId": main_id, "name": "interpreter"}).get("ok") or {}).get("messages", [])
                 if m.get("role") == "user" and m.get("source") == "listen"]
        info(f"note in the background conversation: {notes[-1][:80] if notes else '-'}")
        check("(c) 停止本地语音识别 → 同传 stops too (nothing heard, memory back, not listening, a note)",
              not heard and stopped - idle < 50 and not listening and notes,
              f"heard {heard[1] if heard else '-'}; {stopped - idle:+.0f} MB; listening={listening}; "
              f"{len(notes)} notes")

        invoke("agent_send", {"text": "停止同传"})
        wait_until(lambda: "background task interpreter stopped" in log_text(), 60)
        end = settled_mb(pid)
        info(f"after 停止同传: private {end:.0f} MB ({end - idle:+.0f} vs idle)")
        m = re.findall(r"listen: \S+ heard [^\n]+", log_text())
        info(m[-1] if m else "no listen summary in the log")
    finally:
        stop_app(app)
        open(STOP, "w").close()
        try:
            speaker.wait(timeout=10)
        except Exception:  # noqa: BLE001
            speaker.kill()


def ledger():
    try:
        with open(os.path.join(HOME, "usage.jsonl"), encoding="utf-8") as f:
            return [json.loads(line) for line in f if line.strip()]
    except FileNotFoundError:
        return []


def take_lock():
    deadline = time.time() + 1800
    while True:
        try:
            os.makedirs(LOCK)
            return
        except FileExistsError:
            if time.time() > deadline:
                raise SystemExit(f"STOP: {LOCK} held for 30 minutes (another real-machine script is running)")
            print("waiting for real.lock …", flush=True)
            time.sleep(15)


def main():
    guard()
    take_lock()
    try:
        return run()
    finally:
        os.rmdir(LOCK)


def run():
    if not os.path.exists(SRC_EXE):
        raise SystemExit(f"no {SRC_EXE} (build the desktop app first)")
    os.makedirs(os.path.join(WORK, "bin"), exist_ok=True)
    shutil.copy2(SRC_EXE, EXE)
    info(f"exe: copy of target/debug/kotone.exe built {time.ctime(os.path.getmtime(SRC_EXE))}")
    if not os.path.isdir(os.path.join(MODELS_SV, SENSEVOICE_DIR)):
        src = os.path.join(REPO, "target", "interp-e2e", "home", "models")
        shutil.copytree(os.path.join(src, SENSEVOICE_DIR), os.path.join(MODELS_SV, SENSEVOICE_DIR))
        shutil.copy2(os.path.join(src, "silero_vad.onnx"), MODELS_SV)
    make_english()
    open(os.path.join(WORK, "requests.jsonl"), "w").close()
    mock = subprocess.Popen([sys.executable, "-X", "utf8", "-I", os.path.join(HERE, "mock_interpreter.py"),
                             str(MOCK_PORT), WORK], env=dict(os.environ, MOCK_LISTEN_APP=SPEAKER))
    try:
        phase_online()
        phase_local()
    finally:
        mock.terminate()
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
