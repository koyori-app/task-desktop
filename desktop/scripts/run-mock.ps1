param(
    [int]$Port = 4209,
    [switch]$NoBuild,
    [string]$SettingsPath
)

$ErrorActionPreference = 'Stop'
$desktopRoot = Split-Path $PSScriptRoot -Parent
$sessionRoot = Join-Path $desktopRoot 'target/mock-session'
New-Item -ItemType Directory -Force -Path $sessionRoot | Out-Null
if (-not $SettingsPath) { $SettingsPath = Join-Path $sessionRoot 'settings.json' }
$SettingsPath = [IO.Path]::GetFullPath($SettingsPath)

if (-not $NoBuild) {
    Push-Location $desktopRoot
    try {
        cargo build -p app -p mock-api
        if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
    } finally { Pop-Location }
}

# Running copies do not lock Cargo's output binaries during further development.
$runRoot = Join-Path $sessionRoot ([guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runRoot | Out-Null
Copy-Item -LiteralPath (Join-Path $desktopRoot 'target/debug/koyori.exe') -Destination $runRoot
Copy-Item -LiteralPath (Join-Path $desktopRoot 'target/debug/koyori-mock.exe') -Destination $runRoot

if (-not (Test-Path -LiteralPath $SettingsPath)) {
    New-Item -ItemType Directory -Force -Path (Split-Path $SettingsPath -Parent) | Out-Null
    $initialSettings = @{ keep_running_in_background = $false; appearance = 'dark' } | ConvertTo-Json
    [IO.File]::WriteAllText($SettingsPath, $initialSettings, [Text.UTF8Encoding]::new($false))
}
# The in-memory fixture starts fresh on every run; an older cursor must not
# exclude fixture notifications, while appearance and panel sizes stay saved.
$mockSettings = Get-Content -LiteralPath $SettingsPath -Raw | ConvertFrom-Json
$mockSettings | Add-Member -NotePropertyName notification_cursor -NotePropertyValue $null -Force
[IO.File]::WriteAllText($SettingsPath, ($mockSettings | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))

$previousEnv = @{}
foreach ($name in @('KOYORI_API_BASE', 'KOYORI_DEV_TOKEN', 'KOYORI_MOCK_PORT', 'KOYORI_SETTINGS_PATH')) {
    $previousEnv[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$mockProcess = $null
try {
    # Only the in-memory loopback service is used. Credentials are never read.
    $env:KOYORI_API_BASE = "http://127.0.0.1:$Port/api"
    $env:KOYORI_DEV_TOKEN = 'dev'
    $env:KOYORI_MOCK_PORT = "$Port"
    $env:KOYORI_SETTINGS_PATH = [IO.Path]::GetFullPath($SettingsPath)
    $mockProcess = Start-Process -FilePath (Join-Path $runRoot 'koyori-mock.exe') -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $sessionRoot 'mock.stdout.log') `
        -RedirectStandardError (Join-Path $sessionRoot 'mock.stderr.log')
    $ready = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        Start-Sleep -Milliseconds 100
        if ($mockProcess.HasExited) { throw "Mock API failed to start. Check $sessionRoot/mock.stderr.log (port $Port may be busy)." }
        try {
            Invoke-RestMethod -Uri "$env:KOYORI_API_BASE/v1/tenants" -Headers @{ Authorization = 'Bearer dev' } | Out-Null
            $ready = $true
            break
        } catch { }
    }
    if (-not $ready) { throw 'Mock API did not become ready.' }
    Write-Output "Mock API: $env:KOYORI_API_BASE"
    Write-Output "Settings: $env:KOYORI_SETTINGS_PATH"
    Write-Output "Logs: $sessionRoot"
    $appProcess = Start-Process -FilePath (Join-Path $runRoot 'koyori.exe') -PassThru `
        -RedirectStandardOutput (Join-Path $sessionRoot 'app.stdout.log') `
        -RedirectStandardError (Join-Path $sessionRoot 'app.stderr.log')
    $appProcess.WaitForExit()
} finally {
    if ($mockProcess -and -not $mockProcess.HasExited) { Stop-Process -Id $mockProcess.Id }
    foreach ($name in $previousEnv.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnv[$name], 'Process')
    }
}
