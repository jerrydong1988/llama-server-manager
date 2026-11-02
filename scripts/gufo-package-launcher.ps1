param(
    [ValidateSet('Start','Stop','Status','Initialize')][string]$Action = 'Start',
    [ValidatePattern('^$|^[a-zA-Z0-9_-]{1,64}$')][string]$TestId = ''
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
$packageRoot = $PSScriptRoot
$manifest = Get-Content -LiteralPath (Join-Path $packageRoot 'BUILD.json') -Raw -Encoding UTF8 | ConvertFrom-Json
if ($manifest.package_id -notmatch '^[a-zA-Z0-9][a-zA-Z0-9_-]{0,100}$') { throw 'Invalid package identity.' }
$dataId = $manifest.package_id + $(if ($TestId) { '-' + $TestId } else { '' })
$data = Join-Path $env:LOCALAPPDATA ('LlamaServerManager-Gufo-Test\' + $dataId)
$app = Join-Path $packageRoot 'bin\llama-server-manager.exe'
$cli = Join-Path $packageRoot 'bin\lsm.exe'
$engine = Join-Path $packageRoot 'gufo\gufo.exe'
$utf8 = New-Object System.Text.UTF8Encoding($false)

function Invoke-Lsm([string]$Operation, [hashtable]$Arguments = @{}) {
    $requestPath = Join-Path $data ('request-' + [guid]::NewGuid().ToString('N') + '.json')
    [System.IO.File]::WriteAllText($requestPath, ($Arguments | ConvertTo-Json -Depth 30), $utf8)
    try {
        $parts = $Operation.Split('.')
        $text = & $cli --app $app --data-dir $data $parts[0] $parts[1] --args ('@' + $requestPath) --wait --timeout 180
        $exitCode = $LASTEXITCODE
        $response = ($text -join "`n") | ConvertFrom-Json
        if ($exitCode -ne 0 -or -not $response.ok) { throw ($response | ConvertTo-Json -Depth 15 -Compress) }
        if ($response.data.status -eq 'failed') { throw ($response.data | ConvertTo-Json -Depth 15 -Compress) }
        if ($response.data.status -eq 'succeeded') { return $response.data.result }
        return $response.data
    } finally { Remove-Item -LiteralPath $requestPath -Force -ErrorAction SilentlyContinue }
}

try {
    [void][System.IO.Directory]::CreateDirectory($data)
    if ($Action -eq 'Stop') { Invoke-Lsm 'service.stop' | ConvertTo-Json -Depth 8; exit 0 }
    if ($Action -eq 'Status') { Invoke-Lsm 'service.status' | ConvertTo-Json -Depth 8; exit 0 }
    if ((Get-FileHash -LiteralPath $engine -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.gufo_sha256) { throw 'Gufo executable checksum mismatch. Restore this test package.' }
    foreach ($binary in @('llama-server-manager.exe','lsm.exe')) {
        $expected = $manifest.manager_sha256.$binary
        if ((Get-FileHash -LiteralPath (Join-Path $packageRoot ('bin\' + $binary))).Hash.ToLowerInvariant() -ne $expected) { throw "Manager executable checksum mismatch: $binary" }
    }
    $profiles = Get-Content -LiteralPath (Join-Path $packageRoot 'profiles.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($trialProfile in $profiles) {
        foreach ($file in @($trialProfile.config.model_path, $trialProfile.config.mmproj_path, $trialProfile.config.gufo.mtp_model, $trialProfile.config.gufo.dflash_model)) {
            if ($file -and -not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Model file missing: $file. Update profiles.json before first initialization." }
        }
    }
    $null = Invoke-Lsm 'service.start'
    $marker = Join-Path $data 'package-initialized.json'
    if (-not (Test-Path -LiteralPath $marker)) {
        $directories = @(@{ purpose='engine'; path=(Split-Path -Parent $engine) })
        foreach ($trialProfile in $profiles) {
            foreach ($file in @($trialProfile.config.model_path, $trialProfile.config.mmproj_path, $trialProfile.config.gufo.mtp_model, $trialProfile.config.gufo.dflash_model)) {
                if ($file) { $directories += @{ purpose='model'; path=(Split-Path -Parent $file) } }
            }
        }
        $seen = @{}
        foreach ($directory in $directories) {
            $key = $directory.purpose + ':' + $directory.path
            if ($seen.ContainsKey($key)) { continue }; $seen[$key] = $true
            $null = Invoke-Lsm 'paths.authorize' $directory
            $null = Invoke-Lsm 'directories.add' @{ purpose=$directory.purpose; path=$directory.path; revision=(Invoke-Lsm 'service.status').revision }
        }
        $null = Invoke-Lsm 'engines.refresh'
        $null = Invoke-Lsm 'models.refresh'
        $selected = @((Invoke-Lsm 'engines.list').engines | Where-Object { $_.kind -eq 'gufo' })
        if ($selected.Count -ne 1) { throw 'Expected exactly one isolated Gufo engine.' }
        $null = Invoke-Lsm 'engines.probe' @{ id=$selected[0].id }
        foreach ($trialProfile in $profiles) {
            # Convert JSON to a hashtable on Windows PowerShell 5.1 too.
            $config = @{}; $trialProfile.config.psobject.Properties | ForEach-Object { $config[$_.Name] = $_.Value }
            $config.engine_id = $selected[0].id
            $existing = (Invoke-Lsm 'instances.list').instances.psobject.Properties[$trialProfile.id]
            if ($null -eq $existing) { $null = Invoke-Lsm 'instances.create' @{ id=$trialProfile.id; config=$config; revision=(Invoke-Lsm 'service.status').revision } }
        }
        $routes = @($profiles | ForEach-Object { @{ id=('route-' + $_.id); enabled=$true; model_alias=$_.config.alias; target_instance_id=$_.id; priority=0; weight=1; max_concurrent_requests=1 } })
        $null = Invoke-Lsm 'routes.update' @{ revision=(Invoke-Lsm 'routes.get').revision; patch=@{ host='127.0.0.1'; port=18580; strict_model_routing=$true; health_check_interval_ms=1000; routes=$routes; api_keys=@() } }
        [System.IO.File]::WriteAllText($marker, ($manifest | ConvertTo-Json -Depth 8), $utf8)
    }
    Write-Host ('Isolated data: ' + $data)
    Write-Host 'Start one model at a time.'
    if ($Action -eq 'Start') {
        $null = Invoke-Lsm 'service.show'
        try {
            $router = Invoke-Lsm 'routes.start'
            Write-Host ('API: http://' + $router.bound_addr + '/v1 (loopback only).')
        } catch { Write-Warning ('Router did not start. Choose a free port in the API page: ' + $_) }
    }
    exit 0
} catch { Write-Host $_.ScriptStackTrace; Write-Error $_; exit 1 }
