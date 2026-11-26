# Run only in an isolated Windows environment without an active render endpoint.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [switch] $LocalOnly
)
$ErrorActionPreference = 'Stop'
if (@(Get-Service Audiosrv,AudioEndpointBuilder | Where-Object Status -ne Running).Count) {
    throw 'NOT RUN: start the Windows audio services in the isolated test environment first'
}

# The first method of each IUnknown interface is sufficient for a read-only
# endpoint count. COM failures propagate as exceptions instead of counting as zero.
if (!('EchoRenderEndpoints' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class EchoRenderEndpoints {
    [ComImport, Guid("a95664d2-9614-4f35-a746-de8db63617e6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDeviceEnumerator {
        void EnumAudioEndpoints(int flow, uint stateMask, out IDeviceCollection devices);
    }
    [ComImport, Guid("0bd7a1be-7a1a-44db-8397-cc5392387b5e"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IDeviceCollection {
        void GetCount(out uint count);
    }

    public static uint ActiveCount() {
        var clsid = new Guid("bcde0395-e52f-467c-8e3d-c4579291692e");
        var enumerator = (IDeviceEnumerator)Activator.CreateInstance(Type.GetTypeFromCLSID(clsid, true));
        IDeviceCollection devices = null;
        try {
            const int render = 0;
            const uint active = 1;
            enumerator.EnumAudioEndpoints(render, active, out devices);
            devices.GetCount(out uint count);
            return count;
        } finally {
            if (devices != null) Marshal.ReleaseComObject(devices);
            Marshal.ReleaseComObject(enumerator);
        }
    }
}
'@
}
$count = [EchoRenderEndpoints]::ActiveCount()
if ($count -ne 0) { throw "NOT RUN: Windows reports $count active render endpoint(s)" }
Write-Host 'Windows Core Audio reports zero active render endpoints'

$previousRoot = $env:ECHO_INSTALLED_NATIVE_ROOT
$previousManifest = $env:ECHO_INSTALLED_NATIVE_MANIFEST
$previousFlag = $env:ECHO_TEST_NO_OUTPUT
$env:ECHO_INSTALLED_NATIVE_ROOT = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$env:ECHO_INSTALLED_NATIVE_MANIFEST = Join-Path $env:ECHO_INSTALLED_NATIVE_ROOT 'dlls.json'
$env:ECHO_TEST_NO_OUTPUT = '1'
Push-Location (Resolve-Path "$PSScriptRoot/../..")
try {
    if ($LocalOnly) {
        & "$PSScriptRoot/test-local.ps1" -ResourceRoot $ResourceRoot -NoOutputEndpoint
    } else {
        bun run fixtures:audio
        if ($LASTEXITCODE -ne 0) { throw 'Audio fixture generation failed' }
        cargo test --locked --manifest-path src-tauri/Cargo.toml --test native -- --ignored --exact no_output_device
        if ($LASTEXITCODE -ne 0) { throw 'WASAPI device absence acceptance failed' }
    }
    if ([EchoRenderEndpoints]::ActiveCount() -ne 0) { throw 'Render endpoint appeared during the test' }
} finally {
    Pop-Location
    $env:ECHO_INSTALLED_NATIVE_ROOT = $previousRoot
    $env:ECHO_INSTALLED_NATIVE_MANIFEST = $previousManifest
    $env:ECHO_TEST_NO_OUTPUT = $previousFlag
}
