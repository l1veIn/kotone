param([string]$Title = "KotoneScriptTarget")
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class F {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
"@
$h = [F]::FindWindow([NullString]::Value, $Title)
if ($h -eq [IntPtr]::Zero) { "not found"; exit 1 }
[F]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [F]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
[void][F]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 200
"foreground ok: $([F]::GetForegroundWindow() -eq $h)"
