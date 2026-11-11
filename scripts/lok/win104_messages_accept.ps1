# win104_messages_accept.ps1 —— #104 LOK worker Windows 消息驱动实机验收
#
# 模式：C:\code\_reports\_ape2.ps1（匿名管道逐帧）。#69 实测回归点：
# 旧 main 的 worker 在 ready 后收到请求帧无应答、bye(run-loop-returned)
# 进程退出；本脚本验证 #104 修复面——message-only window 消息驱动下
# ping/version/load 三帧全应答且 worker 保持存活（含二次 ping 复核泵活），
# 最后关 stdin 走干净关机。
#
# 用法（Windows 实机，LO 26.8.2.2 例）：
#   powershell -ExecutionPolicy Bypass -File win104_messages_accept.ps1 `
#     -WorkerExe  C:\code\mdcore\target\x86_64-pc-windows-msvc\debug\mdc-lok-worker.exe `
#     -ProgramDir "C:\Program Files\LibreOffice\program" `
#     -Fixture    C:\code\mdcore\crates\lok\tests\fixtures\smoke-chars-img-table.docx
#
# 判定：末行 ACCEPT: PASS = 实机门通过；任何一步失败即 ACCEPT: FAIL。
# 证据：每步打印 [step] 帧摘要；stderr 尾部随失败输出。

param(
    [Parameter(Mandatory = $true)][string]$WorkerExe,
    [Parameter(Mandatory = $true)][string]$ProgramDir,
    [Parameter(Mandatory = $true)][string]$Fixture,
    [string]$SessionDir = (Join-Path $env:TEMP "mdc-lok-104-accept")
)

$ErrorActionPreference = 'Stop'

# ---------- 前置检查 ----------
$kernelLib = Join-Path $ProgramDir 'mergedlo.dll'
if (-not (Test-Path $WorkerExe)) { throw "worker 不存在：$WorkerExe（先 cargo build -p mdc-lok --target x86_64-pc-windows-msvc）" }
if (-not (Test-Path $kernelLib)) { throw "内核库不存在：$kernelLib（-ProgramDir 应为 LO 的 program 目录）" }
if (-not (Test-Path $Fixture)) { throw "夹具不存在：$Fixture" }
New-Item -ItemType Directory -Force -Path $SessionDir | Out-Null

$fixtureUrl = 'file:///' + ($Fixture -replace '\\', '/')

# ---------- 启动（匿名管道 + LOK 环境卫生，对齐 client spawn 侧） ----------
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $WorkerExe
$psi.Arguments = "--session-dir `"$SessionDir`" --kernel-lib `"$kernelLib`" --program-dir `"$ProgramDir`""
$psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.CreateNoWindow = $true
# 环境卫生（worker 侧 env_hygiene 幂等双保险；此处显式设便于排查）
$psi.EnvironmentVariables['SAL_LOK_OPTIONS'] = 'unipoll'
$psi.EnvironmentVariables['SAL_NOLOCK_PROFILE'] = '1'
$psi.EnvironmentVariables['SAL_DISABLE_OPENCL'] = '1'
$psi.EnvironmentVariables['SAL_SKIA'] = 'raster'

Write-Host "[boot] worker=$WorkerExe"
Write-Host "[boot] kernel=$kernelLib"
$proc = [System.Diagnostics.Process]::Start($psi)
$pass = $true

function Read-Frame([int]$TimeoutSec, [string]$What) {
    $task = $proc.StandardOutput.ReadLineAsync()
    if (-not $task.Wait($TimeoutSec * 1000)) {
        throw "读 $What 超时（${TimeoutSec}s 无帧；HasExited=$($proc.HasExited)）"
    }
    $line = $task.Result
    if ($null -eq $line) {
        $code = '(running)'
        if ($proc.HasExited) { $code = $proc.ExitCode }
        throw "stdout EOF（worker 已退出或关流；exit=$code；$What）"
    }
    return ($line | ConvertFrom-Json)
}

function Send-Frame([object]$obj) {
    $proc.StandardInput.WriteLine(($obj | ConvertTo-Json -Compress -Depth 6))
    $proc.StandardInput.Flush()
}

function Check([bool]$Cond, [string]$Name, [string]$Evidence) {
    if ($Cond) {
        Write-Host "[ok]   $Name :: $Evidence"
    } else {
        Write-Host "[FAIL] $Name :: $Evidence"
        $script:pass = $false
    }
}

try {
    # ---------- 1. hello / ready（hook_2 一次性数秒级，宽限 120s） ----------
    $hello = Read-Frame 10 'hello'
    Check ($hello.ev -eq 'hello') 'life.hello' ($hello | ConvertTo-Json -Compress)

    $ready = Read-Frame 120 'ready'
    $ver = [string]$ready.detail.version_info
    Check ($ready.ev -eq 'ready' -and $ver.Length -gt 0) 'life.ready(version_info)' $ver

    # ---------- 2. ping / version / load 三帧全应答（#69 回归面） ----------
    Send-Frame @{ id = 1; op = 'ping' }
    $pong = Read-Frame 30 'ping 应答'
    Check ($pong.id -eq 1 -and $pong.ok -eq $true -and $pong.pong -eq $true) 'ring.ping' ($pong | ConvertTo-Json -Compress)

    Send-Frame @{ id = 2; op = 'version' }
    $version = Read-Frame 30 'version 应答'
    Check ($version.id -eq 2 -and $version.ok -eq $true -and ([string]$version.version_info).Length -gt 0) 'ring.version' ([string]$version.version_info)

    Send-Frame @{ id = 3; op = 'load'; url = $fixtureUrl }
    $load = Read-Frame 60 'load 应答'
    Check ($load.id -eq 3 -and $load.ok -eq $true -and $load.parts -ge 1) 'ring.load' ($load | ConvertTo-Json -Compress)

    # ---------- 3. 存活门（核心回归断言：run_loop 不得返回） ----------
    Start-Sleep -Seconds 2
    $alive1 = -not $proc.HasExited
    Check $alive1 'alive.after_ring(2s)' "HasExited=$($proc.HasExited)"

    Send-Frame @{ id = 4; op = 'ping' }
    $pong2 = Read-Frame 30 '二次 ping 应答'
    Check ($pong2.id -eq 4 -and $pong2.ok -eq $true) 'alive.second_ping_pumped' ($pong2 | ConvertTo-Json -Compress)

    # ---------- 4. 干净关机（关 stdin → bye(stdin-eof) → exit 0） ----------
    $proc.StandardInput.Close()
    $bye = Read-Frame 15 'bye'
    Check ($bye.ev -eq 'bye') 'shutdown.bye' ($bye | ConvertTo-Json -Compress)

    $exited = $proc.WaitForExit(15000)
    Check ($exited -and $proc.ExitCode -eq 0) 'shutdown.exit0' "exited=$exited code=$($proc.ExitCode)"
} catch {
    Write-Host "[FAIL] 异常：$($_.Exception.Message)"
    $pass = $false
} finally {
    if (-not $proc.HasExited) {
        Write-Host "[teardown] worker 仍存活，强制清理（pid=$($proc.Id)）"
        try { $proc.Kill() } catch { }
    }
    if ($pass -eq $false) {
        $err = ''
        try { $err = $proc.StandardError.ReadToEnd() } catch { }
        $tail = ($err -split "`r?`n" | Select-Object -Last 15) -join "`n"
        if ($tail.Trim().Length -gt 0) { Write-Host "[stderr tail]`n$tail" }
    }
    try { $proc.Dispose() } catch { }
}

Write-Host ""
if ($pass) { Write-Host 'ACCEPT: PASS（#104 实机门：三帧全应答 + 存活 + 干净关机）' }
else { Write-Host 'ACCEPT: FAIL（证据见上方 [FAIL] 行与 stderr tail）' }
if ($pass) { exit 0 } else { exit 1 }
