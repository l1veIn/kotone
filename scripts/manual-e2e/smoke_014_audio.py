"""Exercise an installed Kotone against real audio on a fresh hosted Windows runner."""

import argparse
from array import array
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unicodedata
import wave


MODEL = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
EXPECTED_TEXT = "对面打野在下路"
VK_F8 = 0x77
KEYEVENTF_KEYUP = 0x0002
WM_CLOSE = 0x0010

TEST_WINDOW = r"""
param([Parameter(Mandatory = $true)][string]$TestDirectory)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Kotone maintenance smoke test'
$form.Width = 520
$form.Height = 180
$form.TopMost = $true
$inputBox = New-Object System.Windows.Forms.TextBox
$inputBox.Multiline = $true
$inputBox.AcceptsReturn = $true
$inputBox.ImeMode = [System.Windows.Forms.ImeMode]::Disable
$inputBox.Dock = 'Fill'
$form.Controls.Add($inputBox)
$utf8 = [System.Text.UTF8Encoding]::new($false)
$inputBox.add_TextChanged({
    [System.IO.File]::WriteAllText((Join-Path $TestDirectory 'received.txt'), $inputBox.Text, $utf8)
})
$form.add_Shown({
    $form.Activate()
    $inputBox.Focus() | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $TestDirectory 'hwnd.txt'), $form.Handle.ToInt64().ToString())
})
[System.Windows.Forms.Application]::Run($form)
"""


def require_hosted_windows():
    if (
        os.name != "nt"
        or os.environ.get("GITHUB_ACTIONS") != "true"
        or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"
        or os.environ.get("RUNNER_OS") != "Windows"
        or Path.home().name.casefold() != "runneradmin"
        or not os.environ.get("RUNNER_TEMP")
        or not os.environ.get("GITHUB_WORKSPACE")
    ):
        raise RuntimeError("This smoke test requires a fresh GitHub-hosted Windows runner.")


def verify_inputs(args):
    validate_under = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    target = args.target.resolve()
    if target == validate_under or not target.is_relative_to(validate_under):
        raise RuntimeError("The smoke-test directory must be below RUNNER_TEMP.")

    exe = args.exe.resolve(strict=True)
    installed = Path(os.environ["LOCALAPPDATA"]).resolve(strict=True) / "Kotone" / "kotone.exe"
    if exe != installed:
        raise RuntimeError("Expected Kotone installed at the hosted runner's per-user path.")
    repo = Path(os.environ["GITHUB_WORKSPACE"]).resolve(strict=True)
    if args.after_audio_script:
        script = args.after_audio_script.resolve(strict=True)
        if not script.is_relative_to(repo) or script.suffix != ".mjs":
            raise RuntimeError("The post-recording check must be a repository .mjs script.")
        args.after_audio_script = script

    version = subprocess.check_output(
        [
            "powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
            "if (Get-Process -Name kotone -ErrorAction SilentlyContinue) { exit 1 }; "
            "[Diagnostics.FileVersionInfo]::GetVersionInfo($env:KOTONE_SMOKE_EXE).ProductVersion",
        ],
        env={**os.environ, "KOTONE_SMOKE_EXE": str(exe)},
        text=True,
        timeout=10,
        creationflags=subprocess.CREATE_NO_WINDOW,
    ).strip()
    if version not in {args.version, f"{args.version}.0"}:
        raise RuntimeError(f"Unexpected installed PE version: {version!r}")

    home = Path.home().resolve(strict=True) / ".kotone"
    settings = home / "config.json"
    if settings.resolve(strict=True) != settings:
        raise RuntimeError("Expected a regular settings file in the runner's test profile.")
    config = json.loads(settings.read_text(encoding="utf-8-sig"))
    if (
        config.get("hotkey") != {"key": "F8", "mode": "hold"}
        or config.get("hotkeyBackend") != "register"
        or config.get("interactionMode") != "push-to-talk"
        or config.get("activeProfileId") != "generic"
        or config.get("sttEngine") != "sherpa-streaming"
        or config.get("activeModelId") != MODEL
        or not config.get("ui", {}).get("autoStart")
        or not config.get("ui", {}).get("firstRunCompleted")
        or not config.get("audioDeviceId", "").startswith("CABLE Output")
        or config.get("connections")
    ):
        raise RuntimeError("The fixture must use F8, the real X-ASR model and the guest virtual mic.")
    target.mkdir(parents=True, exist_ok=True)
    return exe, home, repo, config["audioDeviceId"], target


def wait_for(check, description, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.08)
    raise TimeoutError(f"Timed out waiting for {description}")


def runtime_log(log, pid):
    try:
        text = log.read_text(encoding="utf-8", errors="replace")
    except (FileNotFoundError, PermissionError):
        return ""
    _, _, since_start = text.rpartition(f"===== session start (pid {pid}) =====")
    return since_start


def reserve_loopback_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.bind(("127.0.0.1", 0))
        return server.getsockname()[1]


def find_playback_config(fixture):
    import sounddevice as sd

    with wave.open(str(fixture), "rb") as source:
        if source.getnchannels() != 1 or source.getsampwidth() != 2 or source.getcomptype() != "NONE":
            raise RuntimeError("The speech fixture must be mono, uncompressed 16-bit PCM.")
        frames = array("h", source.readframes(source.getnframes()))
        source_rate = source.getframerate()

    host_apis = sd.query_hostapis()
    for api_name in ("Windows WASAPI", "MME"):
        for index, device in enumerate(sd.query_devices()):
            if (
                host_apis[device["hostapi"]]["name"] != api_name
                or not device["name"].startswith(("CABLE Input (VB-Audio", "Speakers (VB-Audio Virtual Cab"))
                or not device["max_output_channels"]
            ):
                continue
            for rate in dict.fromkeys((source_rate, int(device["default_samplerate"]))):
                for channels in (1, 2):
                    if channels > device["max_output_channels"]:
                        continue
                    try:
                        sd.check_output_settings(device=index, channels=channels, dtype="int16", samplerate=rate)
                    except sd.PortAudioError:
                        continue
                    # The endpoint may accept only its native rate and two channels.
                    output = array("h")
                    for sample in range(len(frames) * rate // source_rate):
                        pos = sample * source_rate
                        left, remainder = divmod(pos, rate)
                        right = min(left + 1, len(frames) - 1)
                        value = (frames[left] * (rate - remainder) + frames[right] * remainder) // rate
                        output.extend((value,) * channels)
                    return sd, index, channels, rate, output.tobytes(), device["name"]
    raise RuntimeError("The hosted runner has no compatible VB-CABLE playback endpoint.")


class Desktop:
    def __init__(self):
        self.api = ctypes.WinDLL("user32", use_last_error=True)
        self.api.GetForegroundWindow.restype = wintypes.HWND
        self.api.GetWindowThreadProcessId.argtypes = (wintypes.HWND, ctypes.POINTER(wintypes.DWORD))
        self.api.GetWindowThreadProcessId.restype = wintypes.DWORD
        self.api.AttachThreadInput.argtypes = (wintypes.DWORD, wintypes.DWORD, wintypes.BOOL)
        self.api.AttachThreadInput.restype = wintypes.BOOL
        self.api.SetForegroundWindow.argtypes = (wintypes.HWND,)
        self.api.SetForegroundWindow.restype = wintypes.BOOL
        self.api.PostMessageW.argtypes = (wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM)
        self.api.keybd_event.argtypes = (wintypes.BYTE, wintypes.BYTE, wintypes.DWORD, ctypes.c_size_t)
        self.api.keybd_event.restype = None
        self.api.MapVirtualKeyW.argtypes = (wintypes.UINT, wintypes.UINT)
        self.api.MapVirtualKeyW.restype = wintypes.UINT
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetCurrentThreadId.restype = wintypes.DWORD
        self.current_thread = kernel.GetCurrentThreadId()

    def owner(self, window):
        pid = wintypes.DWORD()
        self.api.GetWindowThreadProcessId(window, ctypes.byref(pid))
        return pid.value

    def focus_owned_window(self, window, pid):
        if self.owner(window) != pid:
            raise RuntimeError("Receiver HWND does not belong to the disposable test process.")
        foreground = self.api.GetForegroundWindow()
        if foreground == window:
            return True
        foreground_thread = self.api.GetWindowThreadProcessId(foreground, None) if foreground else 0
        attached = False
        if foreground_thread and foreground_thread != self.current_thread:
            attached = self.api.AttachThreadInput(self.current_thread, foreground_thread, True)
        try:
            self.api.SetForegroundWindow(window)
        finally:
            if attached:
                self.api.AttachThreadInput(self.current_thread, foreground_thread, False)
        return self.is_focused(window, pid)

    def is_focused(self, window, pid):
        return self.api.GetForegroundWindow() == window and self.owner(window) == pid

    def assert_focus(self, window, pid):
        if not self.is_focused(window, pid):
            raise RuntimeError("The disposable textbox lost focus; stopping the input test.")

    def send_f8(self, keyup):
        scan = self.api.MapVirtualKeyW(VK_F8, 0)
        self.api.keybd_event(VK_F8, scan, KEYEVENTF_KEYUP if keyup else 0, 0)

    def close_owned_window(self, window, pid):
        if window and self.owner(window) == pid:
            self.api.PostMessageW(window, WM_CLOSE, 0, 0)


def test_recording(desktop, hwnd, pid, playback, session):
    sd, output_id, channels, rate, pcm, output_name = playback
    desktop.assert_focus(hwnd, pid)
    desktop.send_f8(keyup=False)
    try:
        wait_for(lambda: "hotkey fired: mode=Hold state=Pressed" in session(), "F8 down", 3)
        time.sleep(0.65)
        with sd.RawOutputStream(device=output_id, channels=channels, samplerate=rate, dtype="int16") as stream:
            block_bytes = rate // 10 * channels * 2
            for offset in range(0, len(pcm), block_bytes):
                desktop.assert_focus(hwnd, pid)
                stream.write(pcm[offset:offset + block_bytes])
        time.sleep(0.8)
        desktop.assert_focus(hwnd, pid)
    finally:
        desktop.send_f8(keyup=True)
    wait_for(lambda: "hotkey fired: mode=Hold state=Released" in session(), "F8 up", 3)
    return output_name


def receive_text(received):
    try:
        text = received.read_text(encoding="utf-8")
    except (FileNotFoundError, PermissionError, UnicodeDecodeError):
        return ""
    normalized = "".join(
        char for char in text if not char.isspace() and not unicodedata.category(char).startswith("P")
    )
    return text.strip() if normalized == EXPECTED_TEXT else ""


def stop_receiver(desktop, receiver, hwnd):
    if receiver is None or receiver.poll() is not None:
        return
    desktop.close_owned_window(hwnd, receiver.pid)
    try:
        receiver.wait(timeout=3)
    except subprocess.TimeoutExpired:
        receiver.terminate()
        receiver.wait(timeout=5)


def run_smoke(args):
    require_hosted_windows()
    exe, home, repo, capture_name, target = verify_inputs(args)
    run_dir = Path(tempfile.mkdtemp(prefix=f"audio-{args.version}-", dir=target))
    target_script = run_dir / "receiver.ps1"
    target_script.write_text(TEST_WINDOW, encoding="utf-8")
    playback = find_playback_config(repo / "crates/kotone-stt/tests/fixtures/zh-game-3s.wav")
    desktop = Desktop()
    port = reserve_loopback_port()
    app_env = {
        **os.environ,
        "WEBVIEW2_USER_DATA_FOLDER": str(run_dir / "webview"),
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": (
            f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
        ),
    }
    app_env.pop("KOTONE_HOME", None)
    app = subprocess.Popen([str(exe)], env=app_env)
    receiver = None
    hwnd = None
    log = home / "kotone.log"

    def app_session():
        if app.poll() is not None:
            raise RuntimeError(f"Kotone exited early with code {app.returncode}")
        return runtime_log(log, app.pid)

    def is_ready():
        session = app_session()
        if "runtime start failed" in session:
            raise RuntimeError("The installed app failed to start its recording runtime.")
        return all(mark in session for mark in (
            "runtime started",
            f"runtime warmup ok: sherpa-streaming ({MODEL})",
            "hotkey backend=register registered ok: F8 (Hold)",
        ))

    try:
        wait_for(is_ready, "Kotone's X-ASR model and hotkey registration", 120)
        receiver = subprocess.Popen(
            [
                "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-STA", "-File",
                str(target_script), "-TestDirectory", str(run_dir),
            ],
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        handle_file = run_dir / "hwnd.txt"
        wait_for(handle_file.is_file, "the disposable textbox", 20)
        hwnd = int(handle_file.read_text(encoding="ascii"))
        wait_for(
            lambda: desktop.focus_owned_window(hwnd, receiver.pid),
            "the disposable textbox to receive keyboard focus",
            10,
        )
        playback_name = test_recording(desktop, hwnd, receiver.pid, playback, app_session)
        transcript = wait_for(lambda: receive_text(run_dir / "received.txt"), "known X-ASR transcript", 15)
        data = {
            "version": args.version,
            "hotkey": "F8",
            "hotkeyBackend": "RegisterHotKey",
            "model": MODEL,
            "capture": capture_name,
            "playback": playback_name,
            "transcript": transcript,
        }
        (run_dir / "audio-result.json").write_text(
            json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        print(json.dumps(data, ensure_ascii=True), flush=True)
        stop_receiver(desktop, receiver, hwnd)
        if args.after_audio_script:
            subprocess.run(
                ["node", str(args.after_audio_script), str(port), str(target)],
                cwd=repo,
                check=True,
                timeout=60,
            )
    finally:
        try:
            (run_dir / "kotone-session.log").write_text(runtime_log(log, app.pid), encoding="utf-8")
        finally:
            try:
                stop_receiver(desktop, receiver, hwnd)
            finally:
                if app.poll() is None:
                    app.terminate()
                    app.wait(timeout=8)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--target", required=True, type=Path)
    parser.add_argument("--version", required=True, choices=("0.1.13", "0.1.14"))
    parser.add_argument("--after-audio-script", type=Path)
    run_smoke(parser.parse_args())


if __name__ == "__main__":
    main()
