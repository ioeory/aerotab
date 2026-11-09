param([Parameter(Mandatory=$true)][string]$Version, [switch]$InsideSandbox)
$ErrorActionPreference = 'Stop'
$installer = Join-Path $PSScriptRoot "AeroTab_${Version}_x64-setup.exe"
$progressPath = Join-Path $PSScriptRoot 'smoke-progress.txt'
function Write-SmokeProgress([string]$Stage) {
    "$(Get-Date -Format o) $Stage" | Add-Content -Encoding UTF8 $progressPath
    Write-Host "Smoke stage: $Stage"
}
function Get-SandboxInitializationError {
    Add-Type -AssemblyName UIAutomationClient
    foreach ($session in @(Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue)) {
        if ($session.MainWindowHandle -eq 0) { continue }
        $window = [System.Windows.Automation.AutomationElement]::FromHandle($session.MainWindowHandle)
        $nodes = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)
        foreach ($node in $nodes) {
            if ($node.Current.Name -match 'Windows Sandbox failed to initialize') { return $node.Current.Name }
        }
    }
    return $null
}
$running = @(Get-Process aerotab-app -ErrorAction SilentlyContinue)
if ($running.Count -gt 0 -and -not $InsideSandbox) {
    # NSIS silently kills every same-name process even with /D=. Never do that on the host.
    if (-not (Get-Command WindowsSandbox.exe -ErrorAction SilentlyContinue)) { throw 'Existing user app running; Windows Sandbox unavailable. No user process was stopped.' }
    $initializationError = Get-SandboxInitializationError
    if ($initializationError) { throw "Existing Sandbox initialization failure; refusing blind retry: $initializationError" }
    $report = Join-Path $PSScriptRoot 'smoke-result.txt'
    foreach ($old in @($report, $progressPath, (Join-Path $PSScriptRoot 'smoke-launcher-error.txt'))) {
        if (Test-Path $old) { Remove-Item $old }
    }
    Write-SmokeProgress 'host-preflight-ok'
    $folder = [Security.SecurityElement]::Escape($PSScriptRoot)
    # cmd captures parser/path/bootstrap errors even before the helper can write a report.
    $command = [Security.SecurityElement]::Escape("cmd.exe /d /c powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\Smoke\windows-isolated-smoke.ps1 -Version $Version -InsideSandbox > C:\Smoke\smoke-launcher-error.txt 2>&1")
    $wsb = Join-Path $PSScriptRoot 'isolated-smoke.wsb'
    @"
<Configuration><Networking>Disable</Networking><ClipboardRedirection>Disable</ClipboardRedirection><MappedFolders><MappedFolder><HostFolder>$folder</HostFolder><SandboxFolder>C:\Smoke</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder></MappedFolders><LogonCommand><Command>$command</Command></LogonCommand></Configuration>
"@ | Set-Content -Encoding UTF8 $wsb
    Write-Host 'Existing user app preserved. Starting network-disabled Windows Sandbox.'
    $sandbox = Start-Process WindowsSandbox.exe -ArgumentList "`"$wsb`"" -PassThru
    $deadline = (Get-Date).AddMinutes(3)
    while ((Get-Date) -lt $deadline -and -not (Test-Path $report)) {
        Start-Sleep -Seconds 2
        $initializationError = Get-SandboxInitializationError
        if ($initializationError) { throw "Sandbox failed before guest LogonCommand: $initializationError" }
        if ($sandbox.HasExited -and $sandbox.ExitCode -ne 0) { throw "Windows Sandbox exited: $($sandbox.ExitCode)" }
    }
    if (-not (Test-Path $report)) {
        if (Test-Path $progressPath) { Get-Content $progressPath | Write-Host }
        $launcherError = Join-Path $PSScriptRoot 'smoke-launcher-error.txt'
        if (Test-Path $launcherError) { Get-Content $launcherError | Write-Host }
        throw 'Windows Sandbox produced no smoke report within 3 minutes; host application was not touched.'
    }
    $result = Get-Content -Raw $report
    Write-Host $result
    if ($result -notmatch 'Smoke OK:') { throw 'Sandbox smoke did not pass' }
    return
}
$reportPath = Join-Path $PSScriptRoot 'smoke-result.txt'
try {
    Write-SmokeProgress 'guest-started (shared directory writable)'
    $installDir = Join-Path $env:TEMP ("AeroTab-Smoke-" + [guid]::NewGuid())
    Write-Host "Installing isolated copy: $installDir"
    Write-SmokeProgress 'installer-started'
    $install = Start-Process -FilePath $installer -ArgumentList @('/S','/NS',"/D=$installDir") -Wait -PassThru
    Write-SmokeProgress "installer-finished exit=$($install.ExitCode)"
    if ($install.ExitCode -ne 0) { throw "Installer exit code $($install.ExitCode)" }
    $exe = Join-Path $installDir 'aerotab-app.exe'
    if (-not (Test-Path $exe)) { throw 'Isolated installed exe missing' }
    $app = Start-Process -FilePath $exe -ArgumentList '--smoke-test' -PassThru
    $expected = "AeroTab $Version [SMOKE TEST - ISOLATED]"
    $deadline = (Get-Date).AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 500
        $proc = Get-Process -Id $app.Id -ErrorAction SilentlyContinue
        if (-not $proc) { throw 'Smoke process exited before window verification' }
    } while ($proc.MainWindowTitle -ne $expected -and (Get-Date) -lt $deadline)
    if ($proc.MainWindowTitle -ne $expected) { throw "Unexpected title: $($proc.MainWindowTitle)" }
    Write-SmokeProgress "window-verified pid=$($app.Id) title=$expected"
    Start-Sleep -Seconds 4
    $proc = Get-Process -Id $app.Id -ErrorAction Stop
    $result = "Smoke OK: PID $($proc.Id) still running; title=$($proc.MainWindowTitle); exe=$exe; isolated=True; version=$Version"
    Write-Host $result
    $result | Set-Content -Encoding UTF8 $reportPath
    # Close only the fresh isolated test window, never any pre-existing user process.
    $null = $proc.CloseMainWindow()
} catch {
    "Smoke FAILED: $($_.Exception.Message)" | Set-Content -Encoding UTF8 $reportPath
    throw
} finally {
    if ($InsideSandbox) { Start-Process shutdown.exe -ArgumentList '/s /t 5' }
}
