"""Run the maintenance candidate only in its disposable hosted Windows guest."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import smoke_014_audio as guest


XASR = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
SENSEVOICE = "sense-voice-zh-en-ja-ko-yue-2024-07-17"

XASR_DIRECTORY = "sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05"
SENSEVOICE_DIRECTORY = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"


def download(url, target, expected):
    subprocess.run(
        ["curl.exe", "--fail", "--location", "--retry", "3", "--max-time", "240",
         "--silent", "--show-error", "--output", str(target), url],
        check=True,
    )
    with target.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    if digest != expected:
        raise RuntimeError(f"Official model checksum mismatch: {target.name}")


def install_models(root):
    models = root / "models"
    models.mkdir()
    archive = root / (XASR_DIRECTORY + ".tar.bz2")
    download(
        f"https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/{archive.name}",
        archive, "fa5f63d618e5a01526e275a358bb7772e403f84808a4769fba52cffd8160bf74",
    )
    subprocess.run(["tar.exe", "-xf", str(archive), "-C", str(models)], check=True)
    sensevoice = models / SENSEVOICE_DIRECTORY
    sensevoice.mkdir()
    hf = ("https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
          "/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07")
    for name, checksum in (
        ("model.int8.onnx", "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51"),
        ("tokens.txt", "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc"),
    ):
        download(f"{hf}/{name}", sensevoice / name, checksum)
    return models


def seed(home, models, capture, model, engine):
    # Match a 0.1.14 configuration that used the old default autoStart=false.
    settings = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "updatePromptDisabled": True},
        "hotkey": {"key": "F8", "mode": "hold"},
        "hotkeyBackend": "register",
        "interactionMode": "push-to-talk",
        "audioDeviceId": capture,
        "sttEngine": engine,
        "activeModelId": model,
        "activeProfileId": "generic",
        "models": {"dir": str(models)},
        "language": "zh",
        "autoSend": True,
        "channelCycleHotkey": "",
        "resendLastHotkey": "",
        "togglePostProcessingHotkey": "",
        "cyclePostProcessingHotkey": "",
        "runtimeToggleHotkey": "",
        "adminPromptDismissed": True,
        "runAsAdminOnStart": False,
        "overlay": {"visibility": "never"},
        "assistant": {"hotkey": ""},
        "postProcessing": {"enabled": False},
        "soundFeedback": {"record": {"enabled": False}, "send": {"enabled": False}},
    }
    (home / "config.json").write_text(json.dumps(settings), encoding="utf-8")


def spawn_receiver(folder, desktop):
    folder.mkdir()
    script = folder / "receiver.ps1"
    script.write_text(guest.TEST_WINDOW, encoding="ascii")
    receiver = subprocess.Popen(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-STA", "-File",
         str(script), "-TestDirectory", str(folder)],
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    try:
        handle = folder / "hwnd.txt"
        guest.wait_for(handle.is_file, "the owned textbox", 45)
        hwnd = int(handle.read_text(encoding="ascii"))
        guest.wait_for(lambda: desktop.focus_owned_window(hwnd, receiver.pid), "textbox focus", 12)
        return receiver, hwnd
    except BaseException:
        receiver.terminate()
        receiver.wait(timeout=10)
        raise


def hear_phrase(label, folder, desktop, receiver, hwnd, playback, session, cold_marker):
    guest.wait_for(lambda: desktop.focus_owned_window(hwnd, receiver.pid), "textbox focus", 10)
    before = session()
    if cold_marker in before:
        raise RuntimeError(f"{label}: model was ready before this first press.")
    audio = guest.test_recording(desktop, hwnd, receiver.pid, playback, lambda: session()[len(before):])
    try:
        text = guest.wait_for(
            lambda: guest.receive_text(folder / "received.txt"),
            "the entire X-ASR/SenseVoice transcript",
            25,
        )
    except TimeoutError:
        print("Textbox received " + json.dumps(guest.read_received(folder / "received.txt"), ensure_ascii=True), flush=True)
        raise
    recorded = session()[len(before):]
    if "state -> listening" not in recorded:
        raise RuntimeError(f"{label}: no capture entered the listening state.")
    print(json.dumps({"case": label, "playback": audio, "transcript": text}, ensure_ascii=True), flush=True)


def run(exe, scratch, models, capture, playback, model, engine, expect_idle):
    home = scratch / model
    home.mkdir()
    seed(home, models, capture, model, engine)
    desktop = guest.Desktop()
    first_folder = home / "first"
    receiver = hwnd = None
    app = None
    log = home / "kotone.log"
    try:
        if not expect_idle:
            # Focus preparation must finish before X-ASR begins prewarming.
            receiver, hwnd = spawn_receiver(first_folder, desktop)
        app = subprocess.Popen(
            [str(exe)],
            env={**os.environ, "KOTONE_HOME": str(home), "WEBVIEW2_USER_DATA_FOLDER": str(home / "webview")},
        )

        def session():
            if app.poll() is not None:
                raise RuntimeError(f"Test build exited before the check: code {app.returncode}")
            return guest.runtime_log(log, app.pid)

        guest.wait_for(lambda: "runtime started" in session(), "automatic hotkey registration", 90)
        if expect_idle:
            # An idle heavy model does not start loading. Open our textbox after
            # the app's window has taken its initial focus on this VM.
            receiver, hwnd = spawn_receiver(first_folder, desktop)
        completed = f"runtime model load completed: {engine}" if expect_idle else f"runtime warmup ok: {engine} ({model})"
        hear_phrase(f"{model} first key", first_folder, desktop, receiver, hwnd, playback, session, completed)
        if not expect_idle:
            return
        if f"runtime model load started: {engine}" not in session():
            raise RuntimeError("The first heavy-model session did not trigger a cold load.")
        guest.stop_receiver(desktop, receiver, hwnd)
        idle_since = time.monotonic()
        unload = f"runtime model idle unloaded: {engine}"
        guest.wait_for(lambda: unload in session(), "the real five-minute model eviction", 340)
        idle_seconds = time.monotonic() - idle_since
        if idle_seconds < 295:
            raise RuntimeError(f"The model was unexpectedly released after {idle_seconds:.1f} seconds.")
        print(f"PASS model released after {idle_seconds:.1f} idle seconds.", flush=True)
        second_folder = home / "after-idle"
        receiver, hwnd = spawn_receiver(second_folder, desktop)
        pos = session().rfind(unload)
        if pos < 0:
            raise RuntimeError("The idle release proof was lost.")
        hear_phrase(f"{model} after idle release", second_folder, desktop, receiver, hwnd, playback,
                    lambda: session()[pos:], completed)
        if f"runtime model load started: {engine}" not in session()[pos:]:
            raise RuntimeError("The second heavy-model session did not reload the released model.")
    except BaseException:
        if log.is_file():
            print(log.read_text(encoding="utf-8", errors="replace"), flush=True)
        raise
    finally:
        if receiver is not None:
            guest.stop_receiver(desktop, receiver, hwnd)
        if app is not None and app.poll() is None:
            app.terminate()
            app.wait(timeout=10)


def main():
    guest.require_hosted_windows()
    if (os.environ.get("GITHUB_REPOSITORY", "").lower() != "l1vein/kotone"
            or os.environ.get("GITHUB_REF") != "refs/heads/codex/smoke-0.1.15-isolated"
            or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"):
        raise RuntimeError("Expected the isolated branch and manual guest workflow.")
    candidate = Path(os.environ["GITHUB_WORKSPACE"]).resolve(strict=True) / "candidate"
    exe = (candidate / "target/release/kotone.exe").resolve(strict=True)
    if not exe.is_relative_to(candidate / "target"):
        raise RuntimeError("The candidate must be a guest build.")
    capture = "CABLE Output (VB-Audio Virtual Cable)"
    scratch = Path(tempfile.mkdtemp(prefix="kotone-015-", dir=os.environ["RUNNER_TEMP"]))
    fixture = candidate / "crates/kotone-stt/tests/fixtures/zh-game-3s.wav"
    playback = guest.find_playback_config(fixture, capture, scratch / "routing.json")
    models = install_models(scratch)
    run(exe, scratch, models, capture, playback, XASR, "sherpa-streaming", False)
    run(exe, scratch, models, capture, playback, SENSEVOICE, "sherpa-offline", True)
    print("PASS first speech at startup and after the five-minute release.", flush=True)


if __name__ == "__main__":
    main()
