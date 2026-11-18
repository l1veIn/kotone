"""Reproduce 「试听语音」 with the real DeepSeek voice (player report: no sound).

Result 2026-11-18: not reproduced — 3/3 previews audible (loopback peak ≈ 0.66) with default and with the
player's own voice settings (PLAYER_LIKE=1). Since then an empty synthesis is an error and every playback logs
its device and length (`speech: playing …` in kotone.log).

Isolated Kotone (KOTONE_HOME=target/preview-e2e/home, hotkeys off) + the player's real DeepSeek key
(real_deepseek.py: memory only, deleted afterwards). Sets 琴音's connection to DeepSeek so the voice is "auto" =
DeepSeek TTS, presses 试听语音 through the settings window's IPC, and prints the `speech preview:` log lines and
the default output device's peak level while it plays (WASAPI loopback via soundcard if installed).
usage: python -X utf8 -I scripts/manual-e2e/speech_preview_real.py
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
import real_deepseek  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
WORK = os.path.join(REPO, "target", "preview-e2e")
HOME = os.path.join(WORK, "home")
EXE = os.path.join(REPO, "target", "debug", "kotone.exe")
ENV = dict(os.environ, KOTONE_HOME=HOME, WEBVIEW2_USER_DATA_FOLDER=os.path.join(WORK, "webview"),
           WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333")
ENV.pop("KOTONE_TOOL_SERVICE_DIR", None)


def ui(script):
    code = """
import { chromium } from "@playwright/test";
const browser = await chromium.connectOverCDP("http://127.0.0.1:9333");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("#/settings"));
try {
%s
} finally { await browser.close(); }
""" % script
    out = subprocess.run(["node", "--input-type=module"], input=code, capture_output=True, text=True,
                         encoding="utf-8", cwd=os.path.join(REPO, "apps", "desktop"))
    return (out.stdout + out.stderr).strip()


def loopback_peak(seconds, result):
    try:
        import soundcard as sc
        import numpy as np
        mic = sc.get_microphone(sc.default_speaker().name, include_loopback=True)
        data = mic.record(samplerate=48000, numframes=int(48000 * seconds))
        result["peak"] = float(np.abs(data).max())
    except Exception as e:  # noqa: BLE001
        result["peak"] = f"unavailable: {e}"


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(HOME)
    with open(os.path.join(HOME, "config.json"), "w", encoding="utf-8") as f:
        json.dump({"ui": {"firstRunCompleted": True}, "adminPromptDismissed": True, "toolService": {"enabled": True},
                   "hotkey": {"key": "F24", "mode": "toggle"}, "runtimeToggleHotkey": "", "automationStopHotkey": "",
                   "assistant": {"hotkey": "", "callHotkeyDefaulted": True, "wake": {"enabled": False}}}, f)
    app = subprocess.Popen([EXE], env=ENV)
    try:
        time.sleep(10)
        cid = real_deepseek.add_connection(EXE, ENV)
        patch = {"assistant": {"connectionId": cid}}
        if os.environ.get("PLAYER_LIKE"):
            # The player's own voice settings (2026-11-18): DeepSeek voice picked explicitly, a system voice left in voiceId.
            patch = {"assistant": {"connectionId": cid, "ttsModelId": "deepseek-tts", "speechEngine": "system",
                                   "voiceId": r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Speech\Voices\Tokens\TTS_MS_ZH-CN_HUIHUI_11.0",
                                   "speechVolume": 100, "speechRate": 0},
                     "modelConfigs": {"deepseek-tts": {"connection": cid}}}
        print("settings:", json.dumps(real_deepseek.tools_call(EXE, ENV, "settings.update", {"patch": patch}), ensure_ascii=False)[:300])
        for attempt in range(3):
            peak, said = {}, {}
            t = threading.Thread(target=lambda: said.setdefault("r", ui("""
const r = await page.evaluate(async () => {
  const t = Date.now();
  try { await window.__TAURI_INTERNALS__.invoke("preview_reminder", { kind: "voice" }); return "ok " + (Date.now() - t) + "ms"; }
  catch (e) { return "error: " + e; }
});
console.log(r);
""")))
            t.start()
            loopback_peak(8, peak)
            t.join()
            print(f"preview #{attempt + 1}:", said.get("r"), "| output peak while previewing:", peak.get("peak"))
            time.sleep(1)
        with open(os.path.join(HOME, "kotone.log"), encoding="utf-8", errors="replace") as f:
            for line in f:
                if "speech" in line.lower() or "tts" in line.lower():
                    print("  log:", line.rstrip()[:300])
    finally:
        real_deepseek.delete_connection(EXE, ENV)
        try:
            real_deepseek.tools_call(EXE, ENV, "app.quit", {})
        except Exception as e:  # noqa: BLE001
            print("quit:", e)
        time.sleep(3)
        if app.poll() is None:
            app.terminate()


if __name__ == "__main__":
    main()
