"""Windows isolated input-chain smoke test for the 0.2.0 legacy mode.

Uses VB-CABLE as a silent virtual input and the built-in mock ASR, so no real
microphone or game is needed. Keyboard injection is restricted to its own textbox.
For the actual LL-hook/physical-key and production installer tests see ADR 027.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import psutil

TARGET = r"""
param([string]$TestDirectory)
Add-Type -AssemblyName System.Windows.Forms
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Kotone Upgrade Test Target'
$form.Width = 540
$form.Height = 220
$form.KeyPreview = $true
$form.TopMost = $true
$inputBox = New-Object System.Windows.Forms.TextBox
$inputBox.Multiline = $true
$inputBox.ImeMode = [System.Windows.Forms.ImeMode]::Disable
$inputBox.Dock = 'Fill'
$form.Controls.Add($inputBox)
$inputBox.add_TextChanged({
  [IO.File]::WriteAllText((Join-Path $TestDirectory 'received.txt'), $inputBox.Text)
})
$form.add_KeyDown({
  if ($_.KeyCode -eq [System.Windows.Forms.Keys]::Oemtilde) {
    [IO.File]::WriteAllText((Join-Path $TestDirectory 'oem-key.txt'), 'received')
  }
})
$form.add_Shown({
  $form.Activate()
  $inputBox.Focus() | Out-Null
  [IO.File]::WriteAllText((Join-Path $TestDirectory 'hwnd.txt'), $form.Handle.ToInt64().ToString())
})
[System.Windows.Forms.Application]::Run($form)
"""

def wait_for(check, what, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.05)
    raise RuntimeError(f"timed out: {what}")

def check_parent_foreground(user32):
    pid = ctypes.c_ulong()
    window = user32.GetForegroundWindow()
    user32.GetWindowThreadProcessId(window, ctypes.byref(pid))
    safe_processes = {"explorer.exe", "windowsterminal.exe", "pwsh.exe",
                      "powershell.exe", "codex.exe", "openai.codex.exe",
                      "conhost.exe", "openai.codexcommand.exe"}
    process = psutil.Process(pid.value).name().lower() if pid.value else ""
    if process not in safe_processes:
        raise RuntimeError("Use the desktop or terminal as the foreground window, then run again.")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True, help="locally built 0.2.0 kotone.exe")
    parser.add_argument("--virtual-device", default="CABLE Output", help="prefix of the installed VB-CABLE output ID")
    args = parser.parse_args()
    if os.name != "nt":
        raise SystemExit("Windows required.")
    exe = args.exe.resolve(strict=True)
    if exe.name.lower() != "kotone.exe" or "target" not in [part.lower() for part in exe.parts]:
        raise RuntimeError("Provide a local build under a target directory, not an installed executable.")
    version = subprocess.check_output(
        ["pwsh", "-NoProfile", "-Command",
         "[Diagnostics.FileVersionInfo]::GetVersionInfo($env:KOTONE_TEST_PE).ProductVersion"],
        env={**os.environ, "KOTONE_TEST_PE": str(exe)}, text=True,
        creationflags=subprocess.CREATE_NO_WINDOW,
    ).strip()
    if version not in {"0.2.0", "0.2.0.0"}:
        raise RuntimeError("Expected a local 0.2.0 PE version before launching an isolated executable.")
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.GetForegroundWindow.restype = ctypes.c_void_p
    user32.GetWindowThreadProcessId.argtypes = (ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong))
    user32.PostMessageW.argtypes = (ctypes.c_void_p, ctypes.c_uint, ctypes.c_size_t, ctypes.c_ssize_t)
    check_parent_foreground(user32)
    parent = Path(__file__).resolve().parents[2] / "target/upgrade-e2e"
    parent.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="native-", dir=parent))
    home = root / "home"
    home.mkdir()
    target_ps = root / "target.ps1"
    target_ps.write_text(TARGET, encoding="utf-8")
    config = {
        "ui": {"firstRunCompleted": True, "autoStart": False, "updatePromptDisabled": True,
               "legacyUpgrade": "dismissed"},
        "toolService": {"enabled": True},
        "hotkey": {"key": "F24", "mode": "hold"},
        "hotkeyBackend": "register",
        "assistant": {"hotkey": "", "wake": {"enabled": False}},
        "channelCycleHotkey": "", "runtimeToggleHotkey": "", "automationStopHotkey": "",
        "interactionMode": "push-to-talk", "activeProfileId": "generic",
        "sttEngine": "mock-stream", "audioDeviceId": "",
        "adminPromptDismissed": True, "runAsAdminOnStart": False,
        "soundFeedback": {"record": {"enabled": False}, "send": {"enabled": False}},
    }
    (home / "config.json").write_text(json.dumps(config), encoding="utf-8")
    env = {**os.environ, "KOTONE_HOME": str(home), "WEBVIEW2_USER_DATA_FOLDER": str(root / "webview")}
    env.pop("KOTONE_TOOL_SERVICE_DIR", None)
    app = subprocess.Popen([exe, "--background"], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    target = None
    hwnd = None
    def call(tool, arguments=None):
        output = subprocess.run([exe, "tools", "call", tool, "--args-file", "-", "--timeout-ms", "6000"],
                                env=env, text=True, encoding="utf-8", input=json.dumps(arguments or {}),
                                capture_output=True,
                                timeout=8, creationflags=subprocess.CREATE_NO_WINDOW)
        response = json.loads(output.stdout)
        if not response.get("ok"):
            raise RuntimeError(f"{tool}: isolated tool call did not succeed")
        return response["result"]
    def runtime_ready():
        if app.poll() is not None:
            raise RuntimeError("isolated Kotone exited")
        try:
            return call("runtime.status")
        except (RuntimeError, ValueError, subprocess.TimeoutExpired):
            return None
    def key(vk, released=False):
        if user32.GetForegroundWindow() != hwnd:
            raise RuntimeError("The isolated input window is no longer foreground. Input stopped.")
        user32.keybd_event(vk, 0, 2 if released else 0, 0)
    def owned_test_window():
        owner = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        return owner.value == target.pid
    try:
        wait_for(runtime_ready, "isolated Kotone service", 20)
        inputs = [device["id"] for device in call("audio.devices")["devices"]
                  if device["id"].startswith(args.virtual_device) and device["id"].startswith("CABLE Output")]
        if len(inputs) != 1:
            raise RuntimeError("exactly one installed VB-CABLE recording input is required")
        call("settings.update", {"patch": {"audioDeviceId": inputs[0]}})
        call("runtime.start")
        assert call("runtime.status")["phase"] == "running"
        target = subprocess.Popen(["pwsh", "-NoProfile", "-STA", "-File", target_ps,
                                   "-TestDirectory", root], creationflags=subprocess.CREATE_NO_WINDOW)
        handle_file = root / "hwnd.txt"
        wait_for(handle_file.is_file, "isolated textbox")
        hwnd = int(handle_file.read_text())
        if not owned_test_window():
            raise RuntimeError("The input window does not belong to the isolated test process.")
        wait_for(lambda: user32.GetForegroundWindow() == hwnd, "isolated textbox foreground")
        key(0x10)  # Shift + OEM3 on the standard layout
        try:
            key(0xC0)
            key(0xC0, True)
        finally:
            user32.keybd_event(0x10, 0, 2, 0)
        wait_for((root / "oem-key.txt").is_file, "frontmost receiver to get OEM3")
        key(0x87)  # F24
        try:
            time.sleep(1)
        finally:
            user32.keybd_event(0x87, 0, 2, 0)
        text_file = root / "received.txt"
        wait_for(lambda: text_file.is_file() and "对面打野在下路" in text_file.read_text("utf-8"),
                 "recognized mock text in the test chat box")
        print("PASS: OEM3 reached the foreground and hold-to-talk sent the mock transcription.")
    finally:
        if target and target.poll() is None:
            if hwnd and owned_test_window():
                user32.PostMessageW(hwnd, 0x0010, 0, 0)
            try:
                target.wait(timeout=3)
            except subprocess.TimeoutExpired:
                target.terminate()
                target.wait(timeout=5)
        if app.poll() is None:
            try:
                call("app.quit")
                app.wait(timeout=8)
            except Exception:
                app.terminate()
                app.wait(timeout=5)

if __name__ == "__main__":
    main()
