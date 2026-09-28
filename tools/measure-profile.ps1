#requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$InputPath,
    [string]$Executable = "$PSScriptRoot/../src-tauri/target/release/examples/profile_benchmark.exe",
    [ValidateRange(1,10)][int]$Repeats = 3,
    [string]$OutputPath = "$PSScriptRoot/../.cache/benchmarks/latest.json"
)
$ErrorActionPreference = 'Stop'
$capturePath = (Resolve-Path -LiteralPath $InputPath).Path
$binaryPath = (Resolve-Path -LiteralPath $Executable).Path
$outputFullPath = [IO.Path]::GetFullPath($OutputPath)
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($outputFullPath)) | Out-Null
$startInfo = [Diagnostics.ProcessStartInfo]::new($binaryPath)
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.ArgumentList.Add($capturePath)
$startInfo.ArgumentList.Add([string]$Repeats)
$process = [Diagnostics.Process]::new()
$process.StartInfo = $startInfo
$samples = [Collections.Generic.List[object]]::new()
$watch = [Diagnostics.Stopwatch]::StartNew()
try {
    if (!$process.Start()) { throw 'Benchmark process did not start' }
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $peakWorking = 0L
    $peakPrivateSampled = 0L
    while (!$process.WaitForExit(50)) {
        $process.Refresh()
        $peakWorking = [Math]::Max($peakWorking, $process.PeakWorkingSet64)
        $peakPrivateSampled = [Math]::Max($peakPrivateSampled, $process.PrivateMemorySize64)
        $samples.Add(@{ elapsedMs=$watch.Elapsed.TotalMilliseconds; workingBytes=$process.WorkingSet64; privateBytes=$process.PrivateMemorySize64 })
    }
    $watch.Stop()
    $exitCode=$process.ExitCode
    $outText=$stdout.GetAwaiter().GetResult()
    $errText=$stderr.GetAwaiter().GetResult()
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1 Name,NumberOfLogicalProcessors
    $machine = Get-CimInstance Win32_ComputerSystem
    $report = @{
        dateUtc=[DateTime]::UtcNow.ToString('o'); inputBytes=(Get-Item -LiteralPath $capturePath).Length
        inputSha256=(Get-FileHash -LiteralPath $capturePath -Algorithm SHA256).Hash
        binarySha256=(Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash
        cpu=$cpu; physicalMemoryBytes=$machine.TotalPhysicalMemory; os=[Environment]::OSVersion.VersionString
        elapsedMs=$watch.Elapsed.TotalMilliseconds; peakWorkingSetBytes=$peakWorking
        sampledPeakPrivateBytes=$peakPrivateSampled; sampleIntervalMs=50; exitCode=$exitCode
        events=@($outText -split '\r?\n' | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json })
        samples=$samples; stderr=$errText
        scope='Standalone release production parser/extractor/query; excludes WebView and Agent; sampled private peak may miss spikes.'
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $outputFullPath -Encoding utf8
    $report | Select-Object inputBytes,elapsedMs,peakWorkingSetBytes,sampledPeakPrivateBytes,exitCode | ConvertTo-Json
    if ($exitCode -ne 0) { throw "Benchmark failed ($exitCode): $errText" }
} finally {
    if ($process.Id -and !$process.HasExited) { $process.Kill($true); $process.WaitForExit() }
    $process.Dispose()
}
