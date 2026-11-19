$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_REF -cne 'refs/heads/codex/smoke-0.1.14-isolated' -or !$env:RUNNER_TEMP) {
    throw 'This script is only for the disposable GitHub-hosted smoke test guest.'
}

@'
import ctypes as c
from ctypes import wintypes as w

user = c.WinDLL("user32", use_last_error=True)
kernel = c.WinDLL("kernel32", use_last_error=True)
session = w.DWORD()
if not kernel.ProcessIdToSessionId(kernel.GetCurrentProcessId(), c.byref(session)) or session.value == 0:
    raise SystemExit("The runner process is not in an interactive Windows session.")
user.OpenInputDesktop.argtypes = (w.DWORD, w.BOOL, w.DWORD)
user.OpenInputDesktop.restype = w.HANDLE
user.CloseDesktop.argtypes = (w.HANDLE,)
user.GetUserObjectInformationW.argtypes = (w.HANDLE, c.c_int, w.LPVOID, w.DWORD, c.POINTER(w.DWORD))
desktop = user.OpenInputDesktop(0, False, 1)
if not desktop:
    raise c.WinError(c.get_last_error())
try:
    name = c.create_unicode_buffer(128)
    length = w.DWORD()
    if not user.GetUserObjectInformationW(desktop, 2, name, c.sizeof(name), c.byref(length)):
        raise c.WinError(c.get_last_error())
    if name.value.lower() != "default":
        raise SystemExit("The runner does not have the unlocked Default input desktop.")
    print(f"Interactive guest: session={session.value}, desktop={name.value}")
finally:
    user.CloseDesktop(desktop)
'@ | python -
if ($LASTEXITCODE -ne 0) { throw 'Guest desktop preflight failed.' }

foreach ($serviceName in @('AudioEndpointBuilder', 'Audiosrv')) {
    $service = Get-Service -Name $serviceName
    if ($service.StartType -eq 'Disabled') { Set-Service -Name $serviceName -StartupType Manual }
    Start-Service -Name $serviceName
}

$driverRoot = Join-Path $env:RUNNER_TEMP "kotone-smoke-cable-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $driverRoot | Out-Null
$archive = Join-Path $driverRoot 'VBCABLE_Driver_Pack46.zip'
Invoke-WebRequest -Uri 'https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip' -OutFile $archive -TimeoutSec 90
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
if ($hash -ne 'bc44e8974191a5a2427d6c979e2dab08dbb09925f3f435dfa2f0ba3e45b6eb90') {
    throw 'The official virtual audio driver archive does not match its pinned SHA-256.'
}
Expand-Archive -LiteralPath $archive -DestinationPath $driverRoot
$setup = Get-ChildItem -LiteralPath $driverRoot -Recurse -File -Filter 'VBCABLE_Setup_x64.exe'
if (@($setup).Count -ne 1) { throw 'Expected one x64 virtual audio installer.' }
$installer = Start-Process -FilePath $setup.FullName -ArgumentList @('-h', '-i') -WorkingDirectory $setup.DirectoryName -WindowStyle Hidden -PassThru
if (!$installer.WaitForExit(90000)) { throw 'The virtual audio installer did not finish in the guest.' }
Write-Host "Guest driver installer exited $($installer.ExitCode). Verifying endpoints."

function Wait-CableAudio([int] $Seconds) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        $endpoints = Get-PnpDevice -Class AudioEndpoint -PresentOnly -ErrorAction SilentlyContinue
        $recording = $endpoints | Where-Object { $_.Status -eq 'OK' -and $_.FriendlyName -like 'CABLE Output*' }
        $playback = $endpoints | Where-Object { $_.Status -eq 'OK' -and $_.FriendlyName -like 'CABLE Input (*' }
        if ($recording -and $playback) {
            Write-Host "Verified guest recording and playback endpoints: $($recording.FriendlyName); $($playback.FriendlyName)"
            return $true
        }
        Start-Sleep -Seconds 2
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Show-CableDevices {
    Get-PnpDevice -Class Media,AudioEndpoint -ErrorAction SilentlyContinue |
        Format-Table Status, Class, FriendlyName, InstanceId -AutoSize | Out-String | Write-Host
    pnputil.exe /enum-devices /class Media /problem /deviceids | Out-String | Write-Host
}

if (Wait-CableAudio 8) { return }
Show-CableDevices
& "$PSScriptRoot/smoke_014_register_cable.ps1" -DriverRoot $driverRoot
if (Wait-CableAudio 45) { return }
Show-CableDevices
throw 'The guest has no working virtual recording and playback endpoints.'
