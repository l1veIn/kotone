param(
  [Parameter(Mandatory=$true)][string]$InjectCli,
  [Parameter(Mandatory=$true)][string]$Base,
  [int]$Runs = 10,
  [string]$Tag = "case"
)
# Kotone notepad injection integration test:
# open notepad -> force foreground -> inject_cli (real SendInput) -> read text via
# UI Automation -> literal compare. A unique per-run tag "(<Tag><run>)" is appended
# to defeat notepad session-restore false positives.
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FG {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern IntPtr SetActiveWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, IntPtr pid);
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool fAttach);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
  // bypass the foreground lock: send a harmless F24 key first (grants our process
  // foreground rights as the last input sender), then AttachThreadInput + SetForegroundWindow
  public static bool ForceForeground(IntPtr hwnd) {
    IntPtr fg = GetForegroundWindow();
    if (fg == hwnd) return true;
    keybd_event(0x87, 0, 0, UIntPtr.Zero);          // VK_F24 down
    keybd_event(0x87, 0, 2, UIntPtr.Zero);          // VK_F24 up (KEYEVENTF_KEYUP)
    uint fgThread = GetWindowThreadProcessId(fg, IntPtr.Zero);
    uint targetThread = GetWindowThreadProcessId(hwnd, IntPtr.Zero);
    uint cur = GetCurrentThreadId();
    ShowWindowAsync(hwnd, 9); // SW_RESTORE
    if (fgThread != cur) AttachThreadInput(cur, fgThread, true);
    if (targetThread != cur) AttachThreadInput(cur, targetThread, true);
    bool ok = SetForegroundWindow(hwnd);
    SetActiveWindow(hwnd);
    BringWindowToTop(hwnd);
    if (targetThread != cur) AttachThreadInput(cur, targetThread, false);
    if (fgThread != cur) AttachThreadInput(cur, fgThread, false);
    return ok;
  }
}
"@

function Start-Notepad {
  Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 400
  cmd.exe /c start notepad.exe | Out-Null
  for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Milliseconds 500
    $proc = Get-Process notepad -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if ($proc) { return $proc }
  }
  return $null
}

function Ensure-Foreground($hwnd) {
  for ($i = 0; $i -lt 10; $i++) {
    if ([FG]::GetForegroundWindow() -eq $hwnd) { return $true }
    [void][FG]::ForceForeground($hwnd)
    Start-Sleep -Milliseconds 300
  }
  return ([FG]::GetForegroundWindow() -eq $hwnd)
}

function Read-NotepadText($hwnd) {
  $el = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
  $condDoc = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Document)
  $condEdit = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)
  $cond = New-Object System.Windows.Automation.OrCondition($condDoc, $condEdit)
  $doc = $el.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
  if (-not $doc) { return $null }
  $tp = $doc.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
  if (-not $tp) { return $null }
  return $tp.DocumentRange.GetText(-1)
}

$pass = 0; $fail = 0
for ($run = 1; $run -le $Runs; $run++) {
  $expected = "$Base ($Tag$run)"
  $proc = Start-Notepad
  if (-not $proc) { Write-Output "RUN $run FAIL: notepad window not found"; $fail++; continue }
  $fg = Ensure-Foreground $proc.MainWindowHandle
  if (-not $fg) { Write-Output "RUN $run FAIL: cannot foreground notepad"; $fail++; continue }
  Start-Sleep -Milliseconds 300

  $out = & $InjectCli $expected 2>&1 | Out-String
  if ($out -notmatch 'INJECT_OK') {
    Write-Output "RUN $run FAIL: injector error: $out"
    $fail++
    Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force
    continue
  }
  Start-Sleep -Milliseconds 500

  $actual = Read-NotepadText $proc.MainWindowHandle
  if ($null -eq $actual) {
    Write-Output "RUN $run FAIL: UIA read failed"
    $fail++
  } elseif ($actual.Contains($expected)) {
    Write-Output "RUN $run PASS"
    $pass++
  } else {
    $snippet = if ($actual.Length -gt 120) { $actual.Substring(0, 120) } else { $actual }
    Write-Output "RUN $run FAIL: content mismatch. expected=[$expected] actual=[$snippet]"
    $fail++
  }
  Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force
}
Write-Output "RESULT ${Tag}: $pass/$Runs PASS"
if ($fail -gt 0) { exit 1 }
