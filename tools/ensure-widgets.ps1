# 公共前置：确保指定挂件处于"可见 + 已建窗"状态
#
# 为什么需要它：多个验证脚本都要操作某个挂件窗口，而该挂件可能被用户或
# 上一次测试**收起**（store.visible=false）。此时窗口根本不存在，脚本会
# 报"找不到 XX 窗口"或几何断言全灭 —— 实测在 verify-edgedock（4 项 FAIL）
# 与 verify-interactions（找不到便签窗口）上各踩过一次。
#
# 测试脚本必须自建前置状态，不能依赖调用方把应用留成什么样。
# 这个前置也保证脚本**可重复运行**（跑第二遍不会因上次留下的状态而失败）。
#
# 用法：  .\ensure-widgets.ps1 -Kind note        # 便签
#         .\ensure-widgets.ps1 -Kind calendar    # 日历
#         .\ensure-widgets.ps1                   # 全部三个
# 参数：
#   -Kind      要确保可见的挂件类型（todo/calendar/note），省略=全部
#   -Restart   即使都已可见也强制重启（默认只在需要时重启）
param(
  [string[]]$Kind = @('todo', 'calendar', 'note'),
  [switch]$Restart
)

$exe = "$env:LOCALAPPDATA\xhy\xhy.exe"
$store = "$env:APPDATA\dev.xhy.desktop\store.json"

if (-not (Test-Path $exe)) { Write-Error "找不到 xhy.exe：$exe"; exit 1 }
if (-not (Test-Path $store)) { Write-Error "找不到 store.json：$store"; exit 1 }

# 应用运行时会持有 store 的内存副本，直接改文件不生效（退出时还会把副本
# 写回、覆盖改动）—— 所以必须先停进程再改，改完再启动。
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$changed = $false
$j = Get-Content $store -Raw | ConvertFrom-Json
foreach ($w in $j.widgets) {
    if ($Kind -contains $w.kind -and -not $w.visible) {
        $w.visible = $true
        $changed = $true
    }
}
if ($changed) {
    $j | ConvertTo-Json -Depth 14 | Set-Content $store -Encoding utf8
}

# 需要重启的三种情况：
#   ① 改了可见性（store 改动要重启才生效）
#   ② 调用方要求强制重启（几何被上次测试改过时用）
#   ③ **进程根本没在跑** —— 光看 store 会误判成"本就可见、无需重启"，
#      结果调用方随后就撞上"xhy 未运行"（实测踩过）
$running = [bool](Get-Process -Name 'xhy' -EA SilentlyContinue)
if ($changed -or $Restart -or -not $running) {
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
    Start-Process $exe
    Start-Sleep -Seconds 14
    Write-Host ("前置：{0} 已确保可见，应用已启动/重启" -f ($Kind -join '/'))
} else {
    Write-Host ("前置：{0} 本就可见且进程在跑，未重启" -f ($Kind -join '/'))
}
