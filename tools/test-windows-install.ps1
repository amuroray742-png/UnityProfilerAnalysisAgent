#requires -Version 7.0
# Exercises an installed NSIS binary, then removes only that test installation.
# Optional rendered UI checks substitute only the native picker response; excludes MSI.
param(
    [string]$Installer = "$PSScriptRoot/../src-tauri/target/release/bundle/nsis/Unity Profiler Analysis Agent_0.1.0_x64-setup.exe",
    [switch]$DesktopSmoke,
    [switch]$DesktopUi
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath("$PSScriptRoot/..")
$work = Join-Path $repo '.cache/installer-smoke'
$destination = Join-Path $work 'app'
$registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Unity Profiler Analysis Agent'
$vendorRegistry = 'HKCU:\Software\ray\Unity Profiler Analysis Agent'
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
if (!(Test-Path -LiteralPath (Join-Path $repo 'src-tauri/Cargo.toml'))) { throw 'Repository manifest missing' }
foreach ($path in @($registry,$vendorRegistry,$destination)) {
    if (Test-Path -LiteralPath $path) { throw "Refusing to touch an existing installation or test directory: $path" }
}
if (Get-Process -Name 'unity-profiler-analysis-agent' -ErrorAction SilentlyContinue) { throw 'Close the existing app before installer verification' }
[IO.Directory]::CreateDirectory($work) | Out-Null
$result = [ordered]@{
    dateUtc=[DateTime]::UtcNow.ToString('o'); installerSha256=(Get-FileHash -LiteralPath $installerPath).Hash
    scope='NSIS current-user silent installation, installed MCP bridge regression, uninstall; excludes MSI; UI only when explicitly requested'
    installed=$false; protocolPassed=$false; desktopIpcPassed=$null; desktopUiPassed=$null; removed=$false; retainedInstallPreference=$false; preferenceCleanedByHarness=$false
}
function Run-Installer([string]$binary, [string]$arguments) {
    $process = Start-Process -FilePath $binary -ArgumentList $arguments -WindowStyle Hidden -PassThru
    try {
        if (!$process.WaitForExit(60000)) { throw "Installer did not finish within 60 seconds (PID $($process.Id)); inspect it before retrying" }
        if ($process.ExitCode -ne 0) { throw "Installer exit code: $($process.ExitCode)" }
    } finally { $process.Dispose() }
}
$oldTestExe = $env:UPAA_TEST_APP_EXE
try {
    # NSIS /D consumes the remainder of the command line, including spaces.
    Run-Installer $installerPath "/S /NS /D=$destination"
    $registration = Get-ItemProperty -LiteralPath $registry
    if ($registration.InstallLocation.Trim('"') -ne $destination) { throw 'Unexpected registered installation path' }
    $installedExe = Join-Path $destination 'unity-profiler-analysis-agent.exe'
    $result.installedBinarySha256 = (Get-FileHash -LiteralPath $installedExe).Hash
    $result.installed = $true
    $env:UPAA_TEST_APP_EXE = $installedExe
    Push-Location $repo
    try {
        & cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test mcp_wire --test acp_stdio_roundtrip --test reports_source --test optimization *> (Join-Path $work 'protocol.log')
        if ($LASTEXITCODE -ne 0) { throw 'Installed binary protocol regression failed; see .cache/installer-smoke/protocol.log' }
        $result.protocolPassed = $true
        if ($DesktopSmoke -or $DesktopUi) {
            $desktopArguments = @((Join-Path $PSScriptRoot 'desktop-smoke.py'), '--application', $installedExe)
            if ($DesktopUi) { $desktopArguments += '--ui' }
            & python @desktopArguments *> (Join-Path $work 'desktop.log')
            if ($LASTEXITCODE -ne 0) { throw 'Installed WebView/IPC smoke failed; see .cache/installer-smoke/desktop.log' }
            $result.desktopIpcPassed = $true
            if ($DesktopUi) { $result.desktopUiPassed = $true; $result.scope += '; rendered UI checked with native picker response substituted' }
        }
    } finally { Pop-Location }
} finally {
    $env:UPAA_TEST_APP_EXE = $oldTestExe
    $uninstaller = Join-Path $destination 'uninstall.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        # Only invoke the uninstaller created inside the verified test location.
        Run-Installer $uninstaller '/S'
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        while (((Test-Path -LiteralPath $destination) -or (Test-Path -LiteralPath $registry)) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 200 }
    }
    $result.removed = !(Test-Path -LiteralPath $destination) -and !(Test-Path -LiteralPath $registry)
    $result.retainedInstallPreference = Test-Path -LiteralPath $vendorRegistry
    if ($result.removed -and $result.retainedInstallPreference) {
        # NSIS intentionally retains install preferences unless app data is
        # selected for deletion. This key did not exist before our test. Clean
        # only the exact default value we created, never other app/user data.
        $key = Get-Item -LiteralPath $vendorRegistry
        if ($key.GetValue('') -eq $destination -and $key.GetValueNames().Count -eq 1 -and $key.GetSubKeyNames().Count -eq 0) {
            $key.Close()
            Remove-Item -LiteralPath $vendorRegistry
            $result.preferenceCleanedByHarness = $true
        } else { $key.Close() }
    }
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $work 'result.json') -Encoding utf8
    $result | ConvertTo-Json
    if (!$result.removed -or (Test-Path -LiteralPath $vendorRegistry)) { throw 'Unexpected installation files or registry entries remain; inspect result before retrying' }
}
