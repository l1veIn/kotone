"""ADR 028 real-machine checks: the real desktop app (debug build), isolated KOTONE_HOME, real DeepSeek key in memory only.

Drives the settings window through WebView2's DevTools port (Playwright connectOverCDP) — no OS input, so it is safe
while the player is in a game. Three parts:
1. new user (acceptance 1, 5, 6, 7): onboarding → one DeepSeek key → five things on DeepSeek → call key → chat with
   “你能做什么？” prefilled → send it, print her answer; online recognition is running without anyone pressing start and
   no start button is visible; then switch to the local X-ASR model (copied from the player's models dir, read only):
   it starts at once (D9); stop it, the chat shows “本地语音识别 · 未启动”, the chat's call button starts it before
   listening; switch back online: the local status disappears and the start shortcut keeps its value.
2. 0.1.x config → 继续使用语音输入法 (acceptance 3): only ui.legacyUpgrade / ui.startPage change; next launch lands on
   voice input without the onboarding.
3. 0.1.x config → add a key (acceptance 4): opted in, call key in companionBindings, speech recognition untouched.
The DeepSeek connections created in the isolated instances are deleted at the end (their keys live in the isolated
credential space `kotone@<KOTONE_HOME>`). Hotkeys are off or F24, speech volume 0, input tools denied.
usage: python -X utf8 -I scripts/manual-e2e/copilot_flow_real.py [new] [keep] [key]
"""
import json
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import real_deepseek  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "copilot-real")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
PORT = 9334
NO_INPUT = {"game_chat.send": "deny", "game.act": "deny", "automation.run": "deny", "companion.type": "deny"}
X_ASR = "sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05"
X_ASR_ID = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f" — {detail}" if detail else ""))
    return bool(ok)


def env_for(home):
    env = dict(os.environ, KOTONE_HOME=home, WEBVIEW2_USER_DATA_FOLDER=os.path.join(os.path.dirname(home), "webview"),
               WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}")
    env.pop("KOTONE_TOOL_SERVICE_DIR", None)
    return env


def ui(script, stdin_secret=None):
    """Run Playwright code against the settings window. `KEY` is defined from stdin when a secret is given."""
    prelude = 'const KEY = (await new Promise((r) => { let d = ""; process.stdin.on("data", (c) => d += c); process.stdin.on("end", () => r(d)); })).trim();' if stdin_secret else ""
    code = """
import { chromium } from "@playwright/test";
%s
const browser = await chromium.connectOverCDP("http://127.0.0.1:%d");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
const out = {};
try {
%s
} finally { console.log(JSON.stringify(out)); await browser.close(); }
""" % (prelude, PORT, script)
    # 代码走命令行（不含 Key），Key 只走标准输入；在 apps/desktop 下运行才找得到 @playwright/test。
    proc = subprocess.run(["node", "--input-type=module", "-e", code], input=stdin_secret or "", capture_output=True,
                          text=True, encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    if proc.returncode != 0:
        raise RuntimeError((proc.stdout + proc.stderr)[-3000:])
    return json.loads(proc.stdout.strip().splitlines()[-1])


def wait_ui(home, timeout=60):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            ui('await page.waitForSelector("[data-testid=app-frame]", { timeout: 2000 });')
            return
        except Exception:  # noqa: BLE001
            time.sleep(1)
    raise RuntimeError("settings window did not come up")


def read_config(home):
    with open(os.path.join(home, "config.json"), encoding="utf-8") as f:
        return json.load(f)


def start(home):
    app = subprocess.Popen([EXE], env=env_for(home))
    wait_ui(home)
    return app


def stop(app, home, tools=True):
    if tools:
        try:
            real_deepseek.tools_call(EXE, env_for(home), "app.quit", {})
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
    if app.poll() is None:
        app.terminate()
        app.wait(10)


def call(home, tool, args=None, timeout_ms=30_000):
    return real_deepseek.tools_call(EXE, env_for(home), tool, args or {}, timeout_ms)


def runtime(home):
    status = call(home, "runtime.status")
    return status.get("status", status)


def wait_runtime(home, predicate, timeout=40):
    deadline = time.time() + timeout
    status = None
    while time.time() < deadline:
        status = runtime(home)
        if predicate(status):
            return status
        time.sleep(1)
    return status


def delete_deepseek_connections(home, tools=True):
    for c in read_config(home).get("connections", []):
        if c.get("provider") == "deepseek":
            try:
                if tools:
                    call(home, "connections.delete", {"id": c["id"]})
                else:
                    ui('await page.evaluate((id) => window.__TAURI_INTERNALS__.invoke("delete_connection", { id }), %s);' % json.dumps(c["id"]))
            except Exception as e:  # noqa: BLE001
                print(f"WARN could not delete {c['id']}: {e}")


def fresh(name, config, with_model=False):
    home = os.path.join(WORK, name, "home")
    shutil.rmtree(os.path.dirname(home), ignore_errors=True)
    os.makedirs(home)
    with open(os.path.join(home, "config.json"), "w", encoding="utf-8") as f:
        json.dump(config, f, ensure_ascii=False)
    if with_model:
        src = os.path.join(os.path.expanduser("~"), ".kotone", "models", X_ASR)
        if os.path.isdir(src):
            shutil.copytree(src, os.path.join(home, "models", X_ASR))
        else:
            print("WARN the player has no local X-ASR model; local checks will fail")
    return home


ONBOARD_KEY = """
await page.getByTestId("onboarding-key-input").fill(KEY);
await page.getByTestId("onboarding-key-submit").click();
await page.getByTestId("onboarding-connected").waitFor({ timeout: 40000 });
out.switched = await page.getByTestId("onboarding-switched").innerText();
out.callKey = await page.getByTestId("onboarding-call-key-value").innerText();
"""


def part_new_user():
    print("\n== 1. new user, online, then local ==")
    home = fresh("new", {
        "adminPromptDismissed": True, "toolService": {"enabled": True}, "hotkey": {"key": "F24", "mode": "toggle"},
        "automationStopHotkey": "", "assistant": {"speechVolume": 0, "wake": {"enabled": False}},
        "agent": {"permissions": {"mode": "allow-all", "rules": NO_INPUT}, "progressSpeech": False},
    }, with_model=True)
    app = start(home)
    try:
        r = ui('await page.getByTestId("onboarding-welcome").waitFor({ timeout: 15000 }); out.welcome = await page.getByTestId("onboarding-welcome").innerText();')
        check("new user sees the welcome", "添加 DeepSeek Key" in r["welcome"] and "只想用语音输入？" in r["welcome"])
        key = real_deepseek.read_key()
        if not key:
            raise RuntimeError("no DeepSeek key")
        r = ui('await page.getByTestId("onboarding-add-key").click();' + ONBOARD_KEY, stdin_secret=key)
        del key
        check("five things switched to DeepSeek", all(w in r["switched"] for w in ["对话", "语音识别", "文字识别", "语音合成", "联网搜索"]), r["switched"].replace("\n", " "))
        check("call key defaults to ~", r["callKey"].strip() == "~")
        r = ui("""
await page.getByTestId("onboarding-enter-chat").click();
await page.getByTestId("assistant-request").waitFor();
out.draft = await page.getByTestId("assistant-request").inputValue();
""")
        check("chat opens with “你能做什么？” prefilled", r["draft"] == "你能做什么？")
        cfg = read_config(home)
        check("settings: deepseek ASR / OCR / TTS / search", cfg.get("activeModelId") == "deepseek-asr" and cfg["ocr"]["activeModelId"] == "deepseek-ocr"
              and cfg["assistant"]["ttsModelId"] == "deepseek-tts" and cfg["search"]["activeModelId"] == "deepseek-search")
        status = wait_runtime(home, lambda s: s.get("phase") == "running")
        check("online recognition is running without pressing start", status.get("phase") == "running" and status.get("asrLocal") is False, json.dumps(status, ensure_ascii=False))
        r = ui("""
out.titlebar = await page.getByTestId("titlebar-asr").count();
out.chatStart = await page.getByRole("button", { name: /启动/ }).filter({ visible: true }).count();
await page.getByTestId("settings-nav-general").click();
await page.getByTestId("asr-card").waitFor();
out.card = await page.getByTestId("asr-card").getAttribute("data-asr-state");
out.voiceStart = await page.getByRole("button", { name: /启动/ }).filter({ visible: true }).count();
await page.getByTestId("sidebar-new-chat").click();
const box = page.getByTestId("assistant-request");
await box.fill("你能做什么？");
await box.press("Enter");
""")
        check("no start button anywhere (online)", r["titlebar"] == 0 and r["chatStart"] == 0 and r["voiceStart"] == 0 and r["card"] == "online-ready", json.dumps(r, ensure_ascii=False))
        deadline = time.time() + 180
        while time.time() < deadline:
            state = ui('out.state = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("agent_state"));')["state"]
            if not state.get("running") and state.get("current"):
                break
            time.sleep(3)
        conv = ui('out.c = await page.evaluate((id) => window.__TAURI_INTERNALS__.invoke("agent_conversation", { id }), %s);' % json.dumps(state.get("current")))["c"]
        answer = next((m.get("text", "") for m in reversed(conv.get("messages", [])) if m.get("role") == "assistant" and m.get("text", "").strip()), "")
        print("---- her answer to 你能做什么？ ----\n" + answer + "\n----")
        check("she answered “你能做什么？”", len(answer) > 30)
        check("her answer ends with how to call her (~)", "~" in answer or "呼叫" in answer)

        # 换成本地识别：立即启动一次（D9）
        call(home, "settings.update", {"patch": {"runtimeToggleHotkey": "Ctrl+Alt+F11"}})
        call(home, "models.activate", {"category": "asr", "engineId": "sherpa-streaming", "modelId": X_ASR_ID}, timeout_ms=30_000)
        status = wait_runtime(home, lambda s: s.get("phase") == "running" and s.get("asrLocal") is True, timeout=60)
        check("switching to local starts it at once", status.get("phase") == "running" and status.get("asrLocal") is True, json.dumps(status, ensure_ascii=False))
        r = ui('await page.getByTestId("titlebar-asr").waitFor(); out.t = await page.getByTestId("titlebar-asr").getAttribute("data-asr");')
        check("titlebar shows the local status", r["t"] == "local-running")
        call(home, "runtime.stop")
        wait_runtime(home, lambda s: s.get("phase") == "stopped")
        r = ui("""
await page.getByTestId("sidebar-new-chat").click();
await page.getByTestId("chat-local-asr").waitFor({ timeout: 10000 });
out.row = await page.getByTestId("chat-local-asr").innerText();
await page.getByTestId("chat-call").click();
""")
        check("chat shows 本地语音识别 · 未启动", "未启动" in r["row"], r["row"])
        status = wait_runtime(home, lambda s: s.get("phase") == "running", timeout=40)
        check("the chat's call button started local recognition", status.get("phase") == "running")
        time.sleep(7)  # 气泡在听，没开口 5 秒后安静结束
        call(home, "models.activate", {"category": "asr", "engineId": "deepseek-asr", "modelId": "deepseek-asr"}, timeout_ms=30_000)
        status = wait_runtime(home, lambda s: s.get("phase") == "running" and s.get("asrLocal") is False, timeout=60)
        check("switching back online: ready, not local", status.get("asrLocal") is False and status.get("phase") == "running")
        r = ui('await page.waitForTimeout(1000); out.t = await page.getByTestId("titlebar-asr").count();')
        check("titlebar status gone", r["t"] == 0)
        check("start shortcut kept", read_config(home).get("runtimeToggleHotkey") == "Ctrl+Alt+F11")
        log = open(os.path.join(home, "kotone.log"), encoding="utf-8", errors="replace").read()
        check("log: auto start after switching models", "识别模型已切换" in log)
    finally:
        delete_deepseek_connections(home)
        stop(app, home)


LEGACY = {
    "hotkey": {"key": "CapsLock", "mode": "hold"}, "interactionMode": "push-to-talk", "activeProfileId": "lol",
    "sttEngine": "sherpa-streaming", "ui": {"firstRunCompleted": True, "autoStart": False}, "adminPromptDismissed": True,
    "assistant": {"hotkey": "Alt+F9", "speechVolume": 0},
}


def part_legacy_keep():
    print("\n== 2. 0.1.x config → 继续使用语音输入法 ==")
    home = fresh("legacy-keep", LEGACY, with_model=True)
    app = start(home)
    try:
        before = read_config(home)
        r = ui("""
await page.getByTestId("onboarding-upgrade").waitFor({ timeout: 15000 });
out.summary = await page.getByTestId("onboarding-upgrade-summary").innerText();
out.first = await page.getByTestId("onboarding-upgrade").getByRole("button").first().innerText();
await page.getByTestId("onboarding-keep-voice").click();
await page.getByTestId("voice-input").waitFor();
await page.waitForTimeout(1000);
out.card = await page.getByTestId("asr-card").getAttribute("data-asr-state");
""")
        check("upgrade welcome lists the old settings", "CapsLock" in r["summary"], r["summary"].replace("\n", " "))
        check("继续使用语音输入法 is the first button", r["first"].strip() == "继续使用语音输入法")
        check("voice input keeps local recognition", (r["card"] or "").startswith("local-"), r["card"])
        after = read_config(home)
        # 保存时会把缺省字段按默认值写出来（0.1.x 认识这些字段）；比的是原来就有的值
        changed = sorted(k for k in before["ui"] if before["ui"].get(k) != after["ui"].get(k))
        check("old ui values unchanged; only legacyUpgrade / startPage added", not changed and set(after["ui"]) - set(before["ui"]) <= {"legacyUpgrade", "startPage", "updatePromptDisabled"}, str(changed))
        same = all(before.get(k) == after.get(k) for k in ["hotkey", "interactionMode", "activeProfileId", "sttEngine", "activeModelId", "connections"] if k in before)
        check("voice input settings unchanged", same and after["assistant"]["hotkey"] == "Alt+F9" and "companionBindings" not in after["assistant"])
        check("written: dismissed + voiceInput", after["ui"].get("legacyUpgrade") == "dismissed" and after["ui"].get("startPage") == "voiceInput")
    finally:
        stop(app, home, tools=False)
    app = start(home)
    try:
        r = ui('await page.waitForTimeout(2500); out.voice = await page.getByTestId("voice-input").count(); out.onboarding = await page.getByTestId("onboarding").count();')
        check("next launch lands on voice input, no onboarding", r["voice"] == 1 and r["onboarding"] == 0)
    finally:
        stop(app, home, tools=False)


def part_legacy_key():
    print("\n== 3. 0.1.x config → add a DeepSeek key ==")
    home = fresh("legacy-key", LEGACY, with_model=True)
    app = start(home)
    try:
        before = read_config(home)
        key = real_deepseek.read_key()
        r = ui('await page.getByTestId("onboarding-upgrade").waitFor({ timeout: 15000 }); await page.getByTestId("onboarding-meet-companion").click();' + ONBOARD_KEY, stdin_secret=key)
        del key
        check("speech recognition kept", "保持原来的设置" in r["switched"], r["switched"].replace("\n", " "))
        check("call key empty by default", "未设置" in r["callKey"])
        ui("""
await page.getByTestId("onboarding-call-key-recommended").click();
await page.getByTestId("onboarding-enter-chat").click();
await page.getByTestId("assistant-request").waitFor();
""")
        after = read_config(home)
        status = ui('out.s = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("get_runtime_status"));')["s"]
        check("opted in, chat landing", after["ui"].get("legacyUpgrade") == "optedIn" and after["ui"].get("startPage") == "chat")
        check("call key and connection in companionBindings", (after["assistant"].get("companionBindings") or {}).get("hotkey") == "Backquote")
        check("old reminder key and ASR untouched", after["assistant"]["hotkey"] == "Alt+F9" and after.get("sttEngine") == before.get("sttEngine")
              and after.get("activeModelId", "") == before.get("activeModelId", ""))
        check("recognition is still local", status.get("asrLocal") is True, json.dumps(status, ensure_ascii=False))
    finally:
        delete_deepseek_connections(home, tools=False)
        stop(app, home, tools=False)


def sweep_credentials():
    """兜底：测试实例卡住时连接删不掉，Key 的副本还留在 `kotone@<隔离目录>` 下——按目标名删掉。"""
    listing = subprocess.run(["cmdkey", "/list"], capture_output=True, text=True, encoding="mbcs", errors="replace").stdout
    for line in listing.splitlines():
        if "target=" in line and WORK in line:
            target = line.split("target=", 1)[1].strip()
            subprocess.run(["cmdkey", f"/delete:{target}"], capture_output=True)
            print("removed leftover test credential")


def main():
    os.makedirs(WORK, exist_ok=True)
    parts = {"new": part_new_user, "keep": part_legacy_keep, "key": part_legacy_key}
    for part in [parts[name] for name in (sys.argv[1:] or parts)]:
        try:
            part()
        except Exception as e:  # noqa: BLE001
            check(f"{part.__name__} ran to the end", False, str(e)[-1500:])
    sweep_credentials()
    failed = [name for name, ok in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} passed" + (f"; failed: {failed}" if failed else ""))
    return not failed


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
