param(
  [string]$Action = "names",   # names | invoke | setvalue | toggle
  [string]$Name = "",
  [string]$Value = "",
  [int]$Index = 0,
  [string]$NameB64 = "",
  [string]$ValueB64 = ""
)
if ($NameB64) { $Name = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($NameB64)) }
if ($ValueB64) { $Value = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($ValueB64)) }
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
# 用 UI Automation 操作隔离环境里的 Kotone 设置窗口（WebView2 会把无障碍树暴露给 UIA）。
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$C = [System.Windows.Automation.Condition]
$pids = @(Get-Process kotone -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "*kotone-automation-e2e*" } | ForEach-Object { $_.Id })
$root = $null
foreach ($w in $A::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
  if ($pids -contains $w.Current.ProcessId -and $w.Current.Name -like "Kotone*") { $root = $w; break }
}
if (-not $root) { "NO WINDOW"; exit 1 }
$all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
if ($Action -eq "names") {
  foreach ($e in $all) {
    $n = $e.Current.Name
    if ($n -and $n.Trim().Length -gt 0) { "{0}|{1}" -f $e.Current.ControlType.ProgrammaticName.Replace("ControlType.", ""), $n }
  }
  exit 0
}
$matches = @($all | Where-Object { $_.Current.Name -eq $Name })
if ($Action -eq "setvalue") { $matches = @($matches | Where-Object { $_.Current.ControlType.ProgrammaticName -eq "ControlType.Edit" -or $_.Current.ControlType.ProgrammaticName -eq "ControlType.Spinner" }) }
if ($Action -eq "invoke") { $matches = @($matches | Where-Object { $_.Current.ControlType.ProgrammaticName -eq "ControlType.Button" }) }
if ($matches.Count -le $Index) { "NOT FOUND: $Name"; exit 1 }
$el = $matches[$Index]
switch ($Action) {
  "invoke" { $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); "invoked $Name" }
  "toggle" { $el.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle(); "toggled $Name" }
  "setvalue" {
    $el.SetFocus()
    $el.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($Value)
    "set $Name = $Value"
  }
}
