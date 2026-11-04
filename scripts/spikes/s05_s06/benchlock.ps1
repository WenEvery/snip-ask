#!/usr/bin/env pwsh
# S-05/S-06 基准 CPU 独占锁（.verify/bench.lock）
#
# 用法：
#   pwsh scripts/spikes/s05_s06/benchlock.ps1 acquire      # 获取锁（已被他人持有则等待退出码 1）
#   pwsh scripts/spikes/s05_s06/benchlock.ps1 status      # 查看锁
#   pwsh scripts/spikes/s05_s06/benchlock.ps1 release      # 释放自己的锁
#   pwsh scripts/spikes/s05_s06/benchlock.ps1 acquire -Force  # 强制接管（仅限确认持有者已死）
#
# 规则（与项目 P0 团队约定一致）：
#   - 锁文件 .verify/bench.lock 内容为 JSON：{owner,pid,startedAt,note}
#   - 已存在且 age < 40 分钟且 pid 存活 → 视为有人占用，退出码 2；
#   - age >= 40 分钟或 pid 不存在 → 陈旧锁，可接管；
#   - 正式时延测量期间必须持锁；下载/写脚本等阶段不持锁。
param(
    [Parameter(Position = 0)]
    [ValidateSet('acquire', 'release', 'status')]
    [string]$Action = 'status',
    [string]$Owner = 'ext-bench',
    [string]$Note = 'S-05/S-06 正式时延测量',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
$LockPath = Join-Path $Root '.verify\bench.lock'
$StaleMinutes = 40

function Read-Lock {
    if (-not (Test-Path $LockPath)) { return $null }
    try { return Get-Content $LockPath -Raw -Encoding utf8 | ConvertFrom-Json } catch { return $null }
}

function Test-PidAlive([int]$ProcId) {
    if (-not $ProcId) { return $false }
    return [bool](Get-Process -Id $ProcId -ErrorAction SilentlyContinue)
}

switch ($Action) {
    'status' {
        $lock = Read-Lock
        if (-not $lock) { Write-Output 'no lock'; exit 0 }
        $ageMin = ((Get-Date) - [datetime]$lock.startedAt).TotalMinutes
        $alive = Test-PidAlive $lock.pid
        Write-Output ("owner={0} pid={1} alive={2} age_min={3:N1} note={4}" -f $lock.owner, $lock.pid, $alive, $ageMin, $lock.note)
        exit 0
    }
    'acquire' {
        $lock = Read-Lock
        if ($lock) {
            $ageMin = ((Get-Date) - [datetime]$lock.startedAt).TotalMinutes
            $alive = Test-PidAlive $lock.pid
            $sameOwner = $lock.owner -eq $Owner
            if (-not $Force -and -not $sameOwner -and $ageMin -lt $StaleMinutes -and $alive) {
                Write-Warning ("bench.lock 被占用：owner={0} pid={1} alive={2} age={3:N1}min（<{4}min）" -f $lock.owner, $lock.pid, $alive, $ageMin, $StaleMinutes)
                exit 2
            }
            if (-not $Force -and -not $sameOwner -and $ageMin -ge $StaleMinutes) {
                Write-Warning ("接管陈旧锁：owner={0} age={1:N1}min >= {2}min" -f $lock.owner, $ageMin, $StaleMinutes)
            }
            if (-not $Force -and -not $sameOwner -and -not $alive) {
                Write-Warning ("接管死进程锁：owner={0} pid={1} 已不存在" -f $lock.owner, $lock.pid)
            }
        }
        $payload = [ordered]@{
            owner     = $Owner
            pid       = $PID
            startedAt = (Get-Date).ToString('o')
            note      = $Note
        }
        New-Item -ItemType Directory -Force -Path (Split-Path $LockPath) | Out-Null
        $payload | ConvertTo-Json | Out-File -FilePath $LockPath -Encoding utf8 -Force
        Write-Output ("lock acquired: owner={0} pid={1}" -f $Owner, $PID)
        exit 0
    }
    'release' {
        $lock = Read-Lock
        if (-not $lock) { Write-Output 'no lock'; exit 0 }
        if ($lock.owner -ne $Owner -and -not $Force) {
            Write-Warning ("拒绝释放：锁属于 owner={0}" -f $lock.owner)
            exit 2
        }
        Remove-Item $LockPath -Force
        Write-Output ("lock released: owner={0}" -f $lock.owner)
        exit 0
    }
}
