#requires -Version 7.0
# Run on an isolated elevated Windows runner. Never prompts for elevation.
param(
    [string]$Installer = "$PSScriptRoot/../src-tauri/target/release/bundle/msi/Unity Profiler Analysis Agent_0.1.0_x64_en-US.msi",
    [switch]$InspectOnly
)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath("$PSScriptRoot/..")
$work = Join-Path $repo '.cache/msi-smoke'
$destination = Join-Path $work 'app'
$package = (Resolve-Path -LiteralPath $Installer).Path
$engine = New-Object -ComObject WindowsInstaller.Installer
$database = $engine.OpenDatabase($package, 0)
function Read-MsiProperty([string]$name) {
    $view = $database.OpenView("SELECT Value FROM Property WHERE Property = '$name'")
    try { [void]$view.Execute(); $row = $view.Fetch(); if (!$row) { throw "Missing MSI property: $name" }; return $row.StringData(1) }
    finally { [void]$view.Close() }
}
$product = Read-MsiProperty 'ProductCode'
$upgrade = Read-MsiProperty 'UpgradeCode'
if ($product -notmatch '^\{[0-9A-Fa-f-]{36}\}$') { throw ('Invalid MSI ProductCode: '+($product | ConvertTo-Json -Compress)) }
$result = [ordered]@{dateUtc=[DateTime]::UtcNow.ToString('o'); installerSha256=(Get-FileHash -LiteralPath $package).Hash; productCode=$product; upgradeCode=$upgrade; scope='MSI per-machine silent installation, installed stdio regression and uninstall; excludes GUI and version upgrades'; installed=$false; protocolPassed=$false; removed=$false}
if ($InspectOnly) { $result | ConvertTo-Json; exit 0 }
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'MSI verification requires an already elevated isolated runner; no UAC prompt will be launched' }
$registry = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
$vendor = 'HKCU:\Software\ray\Unity Profiler Analysis Agent'
if ($engine.RelatedProducts($upgrade).Count -gt 0) { throw 'Refusing to replace an installed related MSI product' }
foreach ($path in @($registry,$vendor,$destination)) { if (Test-Path -LiteralPath $path) { throw "Refusing existing installation or directory: $path" } }
if (Get-Process -Name 'unity-profiler-analysis-agent' -ErrorAction SilentlyContinue) { throw 'Close the app before MSI verification' }
[IO.Directory]::CreateDirectory($work) | Out-Null
function Run-Msi([string]$arguments) {
    $process = Start-Process -FilePath "$env:SystemRoot/System32/msiexec.exe" -ArgumentList $arguments -WindowStyle Hidden -PassThru
    try {
        if (!$process.WaitForExit(60000)) { throw "MSI timeout (PID $($process.Id)); inspect before retrying" }
        if ($process.ExitCode -ne 0) { throw "MSI exit code $($process.ExitCode); see .cache/msi-smoke logs" }
    } finally { $process.Dispose() }
}
$oldTestExe = $env:UPAA_TEST_APP_EXE
try {
    Run-Msi ('/i "{0}" /qn /norestart ALLUSERS=1 INSTALLDIR="{1}" /L*v "{2}"' -f $package,$destination,(Join-Path $work 'install.log'))
    $registration = Get-ItemProperty -LiteralPath $registry
    if ([IO.Path]::GetFullPath($registration.InstallLocation).TrimEnd('\') -ne $destination.TrimEnd('\')) { throw 'Unexpected MSI registration path' }
    $installedExe = Join-Path $destination 'unity-profiler-analysis-agent.exe'
    $result.installedBinarySha256 = (Get-FileHash -LiteralPath $installedExe).Hash
    $result.installed = $true
    $env:UPAA_TEST_APP_EXE = $installedExe
    Push-Location $repo
    try {
        & cargo test --manifest-path src-tauri/Cargo.toml --locked --offline --test mcp_wire --test acp_stdio_roundtrip *> (Join-Path $work 'protocol.log')
        if ($LASTEXITCODE -ne 0) { throw 'Installed MSI binary protocol regression failed' }
        $result.protocolPassed = $true
    } finally { Pop-Location }
} finally {
    $env:UPAA_TEST_APP_EXE = $oldTestExe
    # Only remove the previously absent exact ProductCode from this test package.
    if (Test-Path -LiteralPath $registry) {
        Run-Msi ('/x {0} /qn /norestart /L*v "{1}"' -f $product,(Join-Path $work 'uninstall.log'))
    }
    $result.removed = !(Test-Path -LiteralPath $registry) -and !(Test-Path -LiteralPath $destination)
    $result.vendorRegistryRetained = Test-Path -LiteralPath $vendor
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $work 'result.json') -Encoding utf8
    $result | ConvertTo-Json
    if (!$result.removed -or $result.vendorRegistryRetained) { throw 'MSI uninstall left registration, install files or vendor values; inspect logs' }
}
