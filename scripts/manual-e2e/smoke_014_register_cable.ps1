param([Parameter(Mandatory)][string]$DriverRoot)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_REF -cne 'refs/heads/codex/smoke-0.1.14-isolated' -or !$env:RUNNER_TEMP) {
    throw 'Audio device registration is restricted to the disposable GitHub-hosted smoke guest.'
}

$root = (Get-Item -LiteralPath $DriverRoot).FullName.TrimEnd('\', '/')
$guestTemp = (Get-Item -LiteralPath $env:RUNNER_TEMP).FullName.TrimEnd('\', '/')
if (!$root.StartsWith("$guestTemp\", [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The verified driver package must be in the hosted guest temporary directory.'
}
$archive = Join-Path $root 'VBCABLE_Driver_Pack46.zip'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne
    'bc44e8974191a5a2427d6c979e2dab08dbb09925f3f435dfa2f0ba3e45b6eb90') {
    throw 'The official VB-CABLE archive does not match its pinned SHA-256.'
}
$inf = Join-Path $root 'vbMmeCable64_win10.inf'
if (!(Test-Path -LiteralPath $inf -PathType Leaf)) {
    throw 'The verified archive did not contain the current AMD64-compatible INF.'
}

function Show-CableDiagnostics {
    Get-Service -Name AudioEndpointBuilder, Audiosrv, VBAudioVACMME -ErrorAction SilentlyContinue |
        Format-Table Name, Status, StartType | Out-Host
    & "$env:windir\System32\pnputil.exe" /enum-devices /deviceid VBAudioVACWDM /deviceids /drivers /services
    & "$env:windir\System32\pnputil.exe" /enum-devices /class AudioEndpoint
    $log = Join-Path $env:windir 'inf\setupapi.dev.log'
    if (Test-Path -LiteralPath $log) {
        Select-String -LiteralPath $log -Pattern 'VBAudioVAC|vbmmeCable|vbaudio_cable' -Context 1, 3 |
            Select-Object -Last 18 | ForEach-Object { Write-Host $_.ToString() }
    }
    Get-WinEvent -FilterHashtable @{
        LogName = 'Microsoft-Windows-CodeIntegrity/Operational'
        StartTime = (Get-Date).AddMinutes(-10)
    } -ErrorAction SilentlyContinue | Where-Object Message -Match 'vbaudio|vbcable' |
        Select-Object -First 10 | ForEach-Object { Write-Host ('CodeIntegrity {0}: {1}' -f $_.Id, $_.Message) }
}

try {
    $existing = @(Get-CimInstance Win32_PnPEntity | Where-Object { $_.HardwareID -contains 'VBAudioVACWDM' })
    if ($existing.Count -eq 0) {
        # The INF serves a software root device. PnPUtil can bind the signed package
        # after the device node exists, but it does not create the root node itself.
        Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public static class KotoneSmokeCableDevice
{
    private const uint GenerateDeviceId = 1;
    private const uint HardwareIdProperty = 1;
    private const uint RegisterDevice = 0x19;

    [StructLayout(LayoutKind.Sequential)]
    private struct DeviceInfo
    {
        internal uint Size;
        internal Guid ClassGuid;
        internal uint DeviceInstance;
        internal UIntPtr Reserved;
    }

    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern IntPtr SetupDiCreateDeviceInfoList(ref Guid deviceClass, IntPtr parent);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    private static extern bool SetupDiCreateDeviceInfoW(IntPtr set, string name, ref Guid deviceClass,
        string description, IntPtr parent, uint flags, ref DeviceInfo device);

    [DllImport("setupapi.dll", ExactSpelling = true, SetLastError = true)]
    private static extern bool SetupDiSetDeviceRegistryPropertyW(IntPtr set, ref DeviceInfo device,
        uint property, byte[] value, uint byteCount);

    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiCallClassInstaller(uint action, IntPtr set, ref DeviceInfo device);

    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    private static extern bool SetupDiGetDeviceInstanceIdW(IntPtr set, ref DeviceInfo device,
        StringBuilder id, uint length, out uint required);

    [DllImport("setupapi.dll")]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);

    private static void Require(bool ok, string api)
    {
        if (!ok)
        {
            int code = Marshal.GetLastWin32Error();
            throw new Win32Exception(code, api + " failed: " + new Win32Exception(code).Message);
        }
    }

    public static string Register()
    {
        Guid media = new Guid("4d36e96c-e325-11ce-bfc1-08002be10318");
        IntPtr set = SetupDiCreateDeviceInfoList(ref media, IntPtr.Zero);
        Require(set != new IntPtr(-1), nameof(SetupDiCreateDeviceInfoList));
        try
        {
            var device = new DeviceInfo { Size = (uint)Marshal.SizeOf<DeviceInfo>() };
            Require(SetupDiCreateDeviceInfoW(set, "MEDIA", ref media, null, IntPtr.Zero,
                GenerateDeviceId, ref device), nameof(SetupDiCreateDeviceInfoW));
            byte[] hardwareId = Encoding.Unicode.GetBytes("VBAudioVACWDM\0\0");
            Require(SetupDiSetDeviceRegistryPropertyW(set, ref device, HardwareIdProperty,
                hardwareId, (uint)hardwareId.Length), nameof(SetupDiSetDeviceRegistryPropertyW));
            Require(SetupDiCallClassInstaller(RegisterDevice, set, ref device), nameof(SetupDiCallClassInstaller));
            var id = new StringBuilder(512);
            uint required;
            Require(SetupDiGetDeviceInstanceIdW(set, ref device, id, (uint)id.Capacity, out required),
                nameof(SetupDiGetDeviceInstanceIdW));
            return id.ToString();
        }
        finally
        {
            SetupDiDestroyDeviceInfoList(set);
        }
    }
}
'@
        Write-Host ('Registered the guest VB-CABLE root device {0}.' -f [KotoneSmokeCableDevice]::Register())
    } else {
        Write-Host "Keeping the existing guest device(s): $($existing.DeviceID -join ', ')"
    }

    # The OS verifies the original vendor catalog and driver. No reboot flag or
    # changes to code-integrity policy are used.
    & "$env:windir\System32\pnputil.exe" /add-driver $inf /install
    $result = $LASTEXITCODE
    if ($result -eq 3010 -or $result -eq 1641) {
        throw "The guest reported that audio driver installation requires a reboot ($result)."
    }
    if ($result -ne 0 -and $result -ne 259) {
        throw "PnPUtil could not install the guest audio driver (exit $result)."
    }
    Show-CableDiagnostics
} catch {
    Show-CableDiagnostics
    throw
}
