param(
    [string]$Executable = "$PSScriptRoot\target\x86_64-pc-windows-msvc\debug\native-terminal-spike.exe",
    [ValidateSet('--pty-smoke', '--smoke')]
    [string]$Mode = '--pty-smoke'
)
$ErrorActionPreference = 'Stop'
$info = New-Object System.Diagnostics.ProcessStartInfo
$info.FileName = $Executable
$info.Arguments = $Mode
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$process = New-Object System.Diagnostics.Process
$process.StartInfo = $info
try {
    if (-not $process.Start()) { throw 'Native prototype process did not start' }
    $processId = $process.Id
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(45000)) {
        & taskkill.exe /PID $process.Id /T /F | Out-Null
        throw 'Native prototype exceeded 45 second external timeout; process tree terminated'
    }
    $process.WaitForExit()
    [Console]::Out.Write($stdout.Result)
    [Console]::Error.Write($stderr.Result)
    exit $process.ExitCode
} finally {
    if (-not $process.HasExited) { & taskkill.exe /PID $process.Id /T /F | Out-Null }
    $process.Dispose()
    # Only this prototype's isolated temporary profile; no persistent browser/user profile.
    $profile = Join-Path ([IO.Path]::GetTempPath()) ("aerotab-spike-webview-" + $processId)
    # WebView2 can outlive the host briefly. Only terminate processes explicitly
    # using this exact per-process prototype user-data-dir, never a user's browser.
    Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" | Where-Object {
        $_.CommandLine -and $_.CommandLine.Contains($profile)
    } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    if (Test-Path $profile) {
        try { Remove-Item -LiteralPath $profile -Recurse -Force -ErrorAction Stop }
        catch { [Console]::Error.WriteLine("Temporary prototype profile remains locked: " + $profile) }
    }
}
