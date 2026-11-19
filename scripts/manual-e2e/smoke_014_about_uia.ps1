param(
    [Parameter(Mandatory)][int]$AppProcessId,
    [Parameter(Mandatory)][string]$TargetDirectory
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or $env:GITHUB_REPOSITORY -ine 'l1veIn/kotone' -or
    $env:GITHUB_REF -cne 'refs/heads/codex/smoke-0.1.14-isolated' -or
    $env:GITHUB_EVENT_NAME -cne 'workflow_dispatch') {
    throw 'Use this check only on the disposable Windows guest.'
}
$temp = (Resolve-Path -LiteralPath $env:RUNNER_TEMP).ProviderPath.TrimEnd('\') + '\'
$target = (Resolve-Path -LiteralPath $TargetDirectory).ProviderPath
if (!$target.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Expected a test directory under RUNNER_TEMP.'
}
$process = Get-Process -Id $AppProcessId
$installedExe = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Kotone/kotone.exe'
if ($process.Path -ine $installedExe -or (Get-Item -LiteralPath $installedExe).VersionInfo.ProductVersion -notin @('0.1.14','0.1.14.0')) {
    throw 'The selected process is not the installed 0.1.14 in this guest.'
}

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$elementType = [System.Windows.Automation.AutomationElement]
$condition = [System.Windows.Automation.Condition]::TrueCondition
$children = [System.Windows.Automation.TreeScope]::Children
$descendants = [System.Windows.Automation.TreeScope]::Descendants
$invoke = [System.Windows.Automation.InvokePattern]::Pattern
$expand = [System.Windows.Automation.ExpandCollapsePattern]::Pattern
$settingsName = 'Kotone ' + (-join [char[]](0x8bbe, 0x7f6e))
$aboutName = -join [char[]](0x5173, 0x4e8e)
$rollbackName = -join [char[]](0x56de, 0x5230, 0x65e7, 0x7248, 0x672c)
$linkName = (-join [char[]](0x4e0b, 0x8f7d)) + ' 0.1.13 ' + (-join [char[]](0x5b89, 0x88c5, 0x5305))
$url = 'https://github.com/l1veIn/kotone/releases/download/v0.1.13/Kotone_0.1.13_x64-setup.exe'
$download = Join-Path ([Environment]::GetFolderPath('UserProfile')) 'Downloads/Kotone_0.1.13_x64-setup.exe'
$expectedHash = '1292c8ec5553ba2f7da08d700bda0ccbf8e1f47c322a9fec836096c7ad0ab17b'

function Wait-For([scriptblock]$Check, [string]$Description, [int]$Seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        $value = & $Check
        if ($value) { return $value }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Description."
}

function Find-Control([string]$Name, [System.Windows.Automation.ControlType]$Type = $null) {
    foreach ($element in $window.FindAll($descendants, $condition)) {
        if ($element.Current.Name -ceq $Name -and (!$Type -or $element.Current.ControlType -eq $Type)) {
            return $element
        }
    }
    return $null
}

$window = Wait-For {
    foreach ($item in $elementType::RootElement.FindAll($children, $condition)) {
        if ($item.Current.ProcessId -eq $AppProcessId -and $item.Current.Name -ceq $settingsName) { return $item }
    }
} 'the installed Kotone settings window' 15

$about = Wait-For { Find-Control $aboutName ([System.Windows.Automation.ControlType]::Button) } 'the About button' 18
$about.GetCurrentPattern($invoke).Invoke()
Write-Host 'Opened About using Windows UI Automation.'
$summary = Wait-For { Find-Control $rollbackName } 'the rollback disclosure' 12
$pattern = $null
if ($summary.TryGetCurrentPattern($expand, [ref]$pattern)) {
    $pattern.Expand()
} else {
    $summary.GetCurrentPattern($invoke).Invoke()
}
$link = Wait-For { Find-Control $linkName ([System.Windows.Automation.ControlType]::Hyperlink) } 'the old installer link' 12
if (Test-Path -LiteralPath $download) { throw 'The expected installer already exists in the guest download folder.' }
$browserFilter = "Name = 'msedge.exe' OR Name = 'chrome.exe' OR Name = 'firefox.exe'"
$link.GetCurrentPattern($invoke).Invoke()
$evidence = Wait-For {
    foreach ($browser in (Get-CimInstance Win32_Process -Filter $browserFilter)) {
        if ($browser.CommandLine -and $browser.CommandLine.Contains($url)) { return 'browser launched with rollback URL' }
    }
    if ((Test-Path -LiteralPath $download) -and (Get-FileHash -LiteralPath $download).Hash -ieq $expectedHash) {
        return 'browser downloaded the official old installer'
    }
} 'the external browser to open the actual rollback URL' 24

@{ ok = $true; url = $url; method = 'Windows UI Automation'; evidence = $evidence } |
    ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $target 'about-rollback.json') -Encoding utf8
Write-Host "PASS About rollback link: $evidence."
