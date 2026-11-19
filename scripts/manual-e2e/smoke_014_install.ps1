[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$CandidateInstaller
)

$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_REPOSITORY -ine 'l1veIn/kotone' -or
    $env:GITHUB_REF -cne 'refs/heads/codex/smoke-0.1.14-isolated' -or
    $env:GITHUB_EVENT_NAME -cne 'workflow_dispatch') {
    throw 'Run only on the disposable Windows GitHub-hosted runner for this checkout.'
}
if (-not $env:RUNNER_TEMP -or -not $env:GITHUB_WORKSPACE -or $env:KOTONE_HOME) {
    throw 'Expected a clean runner with the normal Windows user-data path.'
}

$FixtureVersion = '0.1.13'
$CandidateVersion = '0.1.14'
$OldInstallerName = 'Kotone_0.1.13_x64-setup.exe'
$OldInstallerUrl = "https://github.com/l1veIn/kotone/releases/download/v0.1.13/$OldInstallerName"
$OldInstallerSha256 = '1292c8ec5553ba2f7da08d700bda0ccbf8e1f47c322a9fec836096c7ad0ab17b'
$Candidate = Get-Item -LiteralPath $CandidateInstaller
$Workspace = (Resolve-Path -LiteralPath $env:GITHUB_WORKSPACE).ProviderPath.TrimEnd('\') + '\'
$TempRoot = (Resolve-Path -LiteralPath $env:RUNNER_TEMP).ProviderPath.TrimEnd('\') + '\'
if ($Candidate.PSIsContainer -or $Candidate.Name -cne 'Kotone_0.1.14_x64-setup.exe' -or
    (-not $Candidate.FullName.StartsWith($Workspace, [StringComparison]::OrdinalIgnoreCase) -and
     -not $Candidate.FullName.StartsWith($TempRoot, [StringComparison]::OrdinalIgnoreCase))) {
    throw 'Expected the freshly built 0.1.14 NSIS installer in the runner workspace or temp directory.'
}

$InstallDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Kotone'
$DataDir = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.kotone'
$Exe = Join-Path $InstallDir 'kotone.exe'
$Config = Join-Path $DataDir 'config.json'
$UninstallRegistry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Kotone'
if ((Test-Path -LiteralPath $InstallDir) -or (Test-Path -LiteralPath $DataDir) -or
    (Test-Path -LiteralPath $UninstallRegistry) -or (Get-Process -Name kotone -ErrorAction SilentlyContinue)) {
    throw 'A clean runner account is required. Found an existing Kotone installation, data, or process.'
}

$Scratch = Join-Path $env:RUNNER_TEMP 'kotone-014-overwrite'
New-Item -ItemType Directory -Path $Scratch | Out-Null
$OldInstaller = Join-Path $Scratch $OldInstallerName
$AudioScript = Join-Path $PSScriptRoot 'smoke_014_audio.py'
$AboutScript = Join-Path $PSScriptRoot 'smoke_014_about.mjs'

function Assert-Sha256([string]$Path, [string]$Expected) {
    $Actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if ($Actual -ine $Expected) {
        throw "SHA-256 mismatch for $Path. Expected $Expected, got $Actual."
    }
}

function Save-TestInput([string]$Url, [string]$Path, [string]$Sha256) {
    curl.exe --fail --location --retry 3 --max-time 240 --silent --show-error --output $Path $Url
    if ($LASTEXITCODE -ne 0) { throw "Download failed for $Url (curl $LASTEXITCODE)." }
    Assert-Sha256 $Path $Sha256
}

function Invoke-Nsis([string]$Path, [string[]]$Options) {
    $Installer = Start-Process -FilePath $Path -ArgumentList $Options -PassThru
    if (-not $Installer.WaitForExit(180000)) {
        $Installer.Kill()
        throw "NSIS timed out for $Path."
    }
    if ($Installer.ExitCode -ne 0) { throw "NSIS exited $($Installer.ExitCode) for $Path." }
}

function Assert-InstalledVersion([string]$Version) {
    $ProductVersion = (Get-Item -LiteralPath $Exe).VersionInfo.ProductVersion
    $Registry = Get-ItemProperty -LiteralPath $UninstallRegistry
    if ($ProductVersion -notin @($Version, "$Version.0") -or $Registry.DisplayVersion -ne $Version -or
        $Registry.InstallLocation.TrimEnd('\') -ine $InstallDir) {
        throw "Installed version/path mismatch. PE $ProductVersion, registry $($Registry.DisplayVersion), path $($Registry.InstallLocation)."
    }
    Write-Host "PASS installed $Version in the same per-user directory: $Exe"
}

function Assert-VoiceConfig([string]$Phase) {
    $Saved = Get-Content -LiteralPath $Config -Raw | ConvertFrom-Json
    if ($Saved.hotkey.key -cne 'F8' -or $Saved.hotkey.mode -cne 'hold' -or
        $Saved.hotkeyBackend -cne 'register' -or $Saved.interactionMode -cne 'push-to-talk' -or
        $Saved.sttEngine -cne 'sherpa-streaming' -or $Saved.activeModelId -cne $ModelId -or
        $Saved.audioDeviceId -cne $CaptureName -or $Saved.activeProfileId -cne 'generic' -or
        $Saved.autoSend -ne $true -or $Saved.ui.firstRunCompleted -ne $true) {
        throw "Representative voice settings changed during $Phase."
    }
    if ($Phase -ne '0.1.13' -and $Saved.lowerSpeakerVolumeWhileListening -eq $true) {
        throw 'Speaker volume reduction was unexpectedly enabled.'
    }
    Write-Host "PASS recorded voice settings preserved during $Phase."
}

function Invoke-VoiceSmoke([string]$Version, [switch]$CheckAbout) {
    $AudioArguments = @('--exe', $Exe, '--target', $Scratch, '--version', $Version)
    if ($CheckAbout) { $AudioArguments += @('--after-audio-script', $AboutScript) }
    python $AudioScript @AudioArguments
    if ($LASTEXITCODE -ne 0) { throw "Installed $Version voice smoke or About check failed." }
    if (Get-Process -Name kotone -ErrorAction SilentlyContinue) {
        throw "The $Version voice smoke did not exit the test application."
    }
}

$CaptureEndpoints = @(Get-PnpDevice -Class AudioEndpoint -PresentOnly |
    Where-Object { $_.Status -eq 'OK' -and $_.FriendlyName -like 'CABLE Output*' })
if ($CaptureEndpoints.Count -ne 1) {
    throw 'Expected one working VB-CABLE recording endpoint before installing Kotone.'
}
$CaptureName = $CaptureEndpoints[0].FriendlyName

Save-TestInput $OldInstallerUrl $OldInstaller $OldInstallerSha256
Write-Host "0.1.14 candidate installer SHA-256: $((Get-FileHash -LiteralPath $Candidate.FullName).Hash)"
Invoke-Nsis $OldInstaller @('/S')
Assert-InstalledVersion $FixtureVersion

$ModelId = 'x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05'
$ModelDirName = 'sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05'
$ModelBase = Join-Path $DataDir 'models'
$Archive = Join-Path $Scratch "$ModelDirName.tar.bz2"
$Extracted = Join-Path $Scratch 'x-asr-archive'
$ModelUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/$ModelDirName.tar.bz2"
$ModelSha256 = 'fa5f63d618e5a01526e275a358bb7772e403f84808a4769fba52cffd8160bf74'
Save-TestInput $ModelUrl $Archive $ModelSha256
New-Item -ItemType Directory -Path $Extracted | Out-Null
tar.exe -xf $Archive -C $Extracted
if ($LASTEXITCODE -ne 0) { throw 'X-ASR archive extraction failed.' }
$Encoders = @(Get-ChildItem -LiteralPath $Extracted -Recurse -File -Filter 'encoder.int8.onnx')
if ($Encoders.Count -ne 1) { throw 'Expected one X-ASR model root in the official archive.' }
$ModelDir = Join-Path $ModelBase $ModelDirName
New-Item -ItemType Directory -Path $ModelDir -Force | Out-Null
$ModelFiles = @{
    'encoder.int8.onnx' = '908596dcc137a73b95be908ca55e88caa1b3dbbe8027c171615f4b0609c5eb1e'
    'decoder.onnx' = 'a1cbc9eac2d5e3fb6617a218c67ad6daaa7f4e0fd225f08b2c22ab0413c8c257'
    'joiner.int8.onnx' = 'aedb7fa697b2ab43f20499826fff7c997eea7d67db77be97769aeeeb726e63b3'
    'tokens.txt' = 'b818a60878b9aae978cbb8ad594acbd403d76d1af2e31ef4197c84e2dbdba27c'
    'bpe.model' = 'f87a38025a5fdd1e4e9591f6a44bb81295097ce0b80df6f4ab9f44e52c64ca5f'
}
foreach ($File in $ModelFiles.GetEnumerator()) {
    $ExtractedFile = Join-Path $Encoders[0].DirectoryName $File.Key
    Assert-Sha256 $ExtractedFile $File.Value
    Copy-Item -LiteralPath $ExtractedFile -Destination $ModelDir
}

@{
    hotkey = @{ key = 'F8'; mode = 'hold' }
    hotkeyBackend = 'register'
    interactionMode = 'push-to-talk'
    audioDeviceId = $CaptureName
    sttEngine = 'sherpa-streaming'
    activeModelId = $ModelId
    activeProfileId = 'generic'
    language = 'zh'
    autoSend = $true
    channelCycleHotkey = ''
    resendLastHotkey = ''
    togglePostProcessingHotkey = ''
    cyclePostProcessingHotkey = ''
    runtimeToggleHotkey = ''
    adminPromptDismissed = $true
    autoAdminPromptDismissed = $true
    runAsAdminOnStart = $false
    evalRecording = $false
    overlay = @{ visibility = 'never' }
    assistant = @{ hotkey = '' }
    postProcessing = @{ enabled = $false }
    soundFeedback = @{ record = @{ enabled = $false }; send = @{ enabled = $false } }
    ui = @{ firstRunCompleted = $true; autoStart = $true }
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Config -Encoding utf8NoBOM

Invoke-VoiceSmoke $FixtureVersion
Assert-VoiceConfig $FixtureVersion
$ConfigSha256 = (Get-FileHash -LiteralPath $Config).Hash
Invoke-Nsis $Candidate.FullName @('/S', '/UPDATE')
Assert-InstalledVersion $CandidateVersion
Assert-Sha256 $Config $ConfigSha256
Assert-VoiceConfig 'the installer overwrite'
Invoke-VoiceSmoke $CandidateVersion -CheckAbout
Assert-VoiceConfig $CandidateVersion
Write-Host 'PASS original 0.1.13 installation, 0.1.14 in-place overwrite, both real recognitions, and rollback link.'
