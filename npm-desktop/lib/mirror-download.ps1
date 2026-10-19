# npm-desktop/lib/mirror-download.ps1
# 共享下载内核：镜像直连（curl --resolve 锁快节点）。
#
# 为什么存在（2026-10-15 实测事故）：
#   镜像走 Cloudflare，域名解析可能落到慢节点——本机 Invoke-WebRequest 走 gh-proxy.com
#   域名只有 32 KB/s（35 分钟下不完 73MB 的 win32 包），osx 包更是停在 99.6% 零增长；
#   同一资产 curl --resolve 锁快节点稳定 1.3-2.5 MB/s，171MB 约 1 分钟取完。
#   另：PS 5.1 的 IWR 对 >40MB 资产会无征兆停滞（TimeoutSec 对卡死的 body 流不生效）。
#
# 纪律（git-pr-protocol SKILL §4.3 / §4.4 / §4.5）：
#   - 节点从 DNS 现取 + 2MB Range 测速挑选，不硬编码 IP（R4；IP 池随 DNS 变化）；
#   - 镜像直连，绝不叠本地代理（叠了限速回 40 KB/s）；只有裸 GitHub 域名才走代理；
#   - 本文件只负责把字节搬下来；落 .tmp、权威 sha256 对账、转正由调用方完成。
#
# 用法：. (Join-Path $PSScriptRoot 'lib\mirror-download.ps1')
#       if (Invoke-MirrorDownload -Url $mirrorUrl -OutFile $tmp) { ... }

# 每主机缓存测速结果：测一次、全程复用；某次下载失败即清该主机缓存，重测换节点
$script:NuphusMirrorFastIp = @{}

function Get-GhProxyIpList {
    param([Parameter(Mandatory = $true)][string]$HostName)
    try {
        return @([System.Net.Dns]::GetHostAddresses($HostName) |
            Where-Object { $_.AddressFamily -eq 'InterNetwork' } |
            ForEach-Object { $_.IPAddressToString })
    } catch {
        return @()
    }
}

# 逐个节点 2MB Range 测速，返回最快的 IP；无节点或全失败返回 $null（调用方回退域名直连）
function Select-FastMirrorIp {
    param(
        [Parameter(Mandatory = $true)][string]$ProbeUrl,
        [int]$ProbeBytes = 2MB,
        [int]$TimeoutSec = 20
    )
    $mirrorHost = ([Uri]$ProbeUrl).Host
    $ips = @(Get-GhProxyIpList -HostName $mirrorHost)
    if ($ips.Count -eq 0) { return $null }
    $bestIp = $null
    $bestSpeed = 0
    foreach ($ip in $ips) {
        $speed = 0
        $raw = & curl.exe -k --http1.1 --resolve "$mirrorHost`:443:$ip" `
            -r "0-$($ProbeBytes - 1)" -s -o NUL -w '%{speed_download}' `
            --max-time $TimeoutSec $ProbeUrl 2>$null
        if ($raw) { try { $speed = [double]$raw } catch { $speed = 0 } }
        if ($speed -gt $bestSpeed) { $bestSpeed = $speed; $bestIp = $ip }
    }
    if ($bestIp) {
        Write-Host ("    [mirror] fastest node for {0}: {1} ({2:N0} B/s)" -f $mirrorHost, $bestIp, $bestSpeed) -ForegroundColor DarkGray
    }
    return $bestIp
}

# 锁快节点下载；成功返回 $true。失败自动换节点重测，Attempts 次后返回 $false。
# 测速不可用时回退域名直连（不带 --resolve），保持与旧行为等价的兜底路径。
function Invoke-MirrorDownload {
    param(
        [Parameter(Mandatory = $true)][string]$Url,
        [Parameter(Mandatory = $true)][string]$OutFile,
        [int]$MaxTimeSec = 1800,
        [int]$Attempts = 3
    )
    $mirrorHost = ([Uri]$Url).Host
    for ($i = 1; $i -le $Attempts; $i++) {
        $ip = $script:NuphusMirrorFastIp[$mirrorHost]
        if (-not $ip) {
            $ip = Select-FastMirrorIp -ProbeUrl $Url
            $script:NuphusMirrorFastIp[$mirrorHost] = $ip
        }
        $curlArgs = @('-k', '--http1.1', '--fail', '--location', '--continue-at', '-',
            '--max-time', "$MaxTimeSec", '-s', '-S', '-o', $OutFile)
        if ($ip) { $curlArgs += @('--resolve', "$mirrorHost`:443:$ip") }
        $curlArgs += $Url
        # Start-Process 调 curl：stderr 不进 PowerShell error stream，免疫 ErrorActionPreference=Stop
        # 下的 NativeCommandError（SKILL §7.4 / §7.7）
        $p = Start-Process -FilePath 'curl.exe' -ArgumentList $curlArgs -Wait -PassThru -NoNewWindow
        if ($p.ExitCode -eq 0 -and (Test-Path $OutFile) -and (Get-Item $OutFile).Length -gt 0) {
            return $true
        }
        Write-Host ("    [mirror] attempt {0} failed (curl exit {1}, node {2}) - re-probing" -f $i, $p.ExitCode, $ip) -ForegroundColor Yellow
        $script:NuphusMirrorFastIp[$mirrorHost] = $null
    }
    return $false
}
