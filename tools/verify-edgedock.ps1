# 贴边自动隐藏端到端验证（真实驱动轮询状态机）
#
# 状态机是 120ms 轮询读真实窗口位置的，所以用 SetWindowPos 分步移动窗口
# 就能真实复现"用户拖动"的每一步；每步之间留 >SETTLE(320ms) 让状态机反应。
#
# 用法:  powershell -File tools\verify-edgedock.ps1
# 退出码 0 = 全部通过
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class ED {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x,int y,int cx,int cy,uint flags);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
    delegate bool E(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }
    public const uint SWP_NOSIZE=0x1, SWP_NOZORDER=0x4, SWP_NOACTIVATE=0x10;
    public static void Aware(){SetProcessDPIAware();}
    public static IntPtr Find(uint pid, string want){ IntPtr hit=IntPtr.Zero;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            var tt=new StringBuilder(64); GetWindowTextW(h,tt,64);
            if(tt.ToString().Contains(want)){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static int[] Rect(IntPtr h){ RC r; GetWindowRect(h,out r); return new[]{r.L,r.T,r.R-r.L,r.B-r.T}; }
    public static void Move(IntPtr h,int x,int y){ SetWindowPos(h,IntPtr.Zero,x,y,0,0,SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE); }
}
'@
[ED]::Aware()

$fail = 0
function Check($name, $ok, $detail) {
    if ($ok) { "  ✅ $name" } else { "  ❌ $name  $detail"; $script:fail++ }
}
function StoreWidget($kind) {
    $sp = "$env:APPDATA\dev.xhy.desktop\store.json"
    ((Get-Content $sp -Raw | ConvertFrom-Json).widgets | Where-Object { $_.kind -eq $kind })
}

$vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
$exe = "$env:LOCALAPPDATA\xhy\xhy.exe"
"屏幕(物理): $($vs.Width)x$($vs.Height)"

# ── 准备：干净启动 ───────────────────────────────────────
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep 2
$s = "$env:APPDATA\dev.xhy.desktop\store.json"
$j = Get-Content $s -Raw | ConvertFrom-Json
# 必须先把日历挂件**设为可见** —— 它可能被上一位使用者（或上次测试）收起了，
# 那样窗口根本不存在，后续所有几何断言都会失败（实测踩过：4 项 FAIL 全因此）。
# 测试脚本要自己建立前置状态，不依赖调用方把应用留成什么样。
($j.widgets | Where-Object { $_.kind -eq 'calendar' }).visible = $true
($j.widgets | Where-Object { $_.kind -eq 'calendar' }).autoDock = $true
($j.widgets | Where-Object { $_.kind -eq 'calendar' }).edgeHide = $null
($j.widgets | Where-Object { $_.kind -eq 'calendar' }).free = $true
($j.widgets | Where-Object { $_.kind -eq 'calendar' }).rect.x = 800
($j.widgets | Where-Object { $_.kind -eq 'calendar' }).rect.y = 300
$j | ConvertTo-Json -Depth 12 | Set-Content $s -Encoding utf8
Remove-Item "$env:APPDATA\dev.xhy.desktop\xhy.log" -Force -EA SilentlyContinue
Start-Process $exe
Start-Sleep 14

$p = Get-Process -Name 'xhy' | Select-Object -First 1
$cal = [ED]::Find([uint32]$p.Id, '日历')
if (-not $cal) { "找不到日历窗口"; exit 1 }
$r = [ED]::Rect($cal)
"日历初始 @($($r[0]),$($r[1])) $($r[2])x$($r[3])"

# ── 1. 拖到右边缘 → 应停靠收起 ───────────────────────────
"`n[1] 拖到右边缘 → 停靠"
$targetX = $vs.Width - $r[2] - 5
$curX = $r[0]
while ($curX -lt $targetX) {
    $curX = [Math]::Min($curX + 60, $targetX)
    [ED]::Move($cal, $curX, $r[1]); Start-Sleep -Milliseconds 220
}
Start-Sleep -Milliseconds 1200
$d = [ED]::Rect($cal)
Check "已收起（大部分出界）" ($d[0] -gt $vs.Width - 40) "@x=$($d[0])"
Check "edgeHide 已落库=right" ((StoreWidget 'calendar').edgeHide -eq 'right') "实际=$((StoreWidget 'calendar').edgeHide)"

# ── 2. 悬停唤出（回归：必须仍然可用）────────────────────
"`n[2] 光标移到边条 → 唤出"
[void][ED]::SetCursorPos($d[0] + 8, $d[1] + 100)
Start-Sleep -Milliseconds 1400
$rv = [ED]::Rect($cal)
Check "唤出后完整在屏内" ($rv[0] + $rv[2] -le $vs.Width + 2) "@x=$($rv[0]) 右缘=$($rv[0]+$rv[2])"

# ── 3. 光标离开 → 收回 ──────────────────────────────────
"`n[3] 光标移开 → 收回"
[void][ED]::SetCursorPos(400, 400)
Start-Sleep -Milliseconds 1400
$rb = [ED]::Rect($cal)
Check "已收回边条" ($rb[0] -gt $vs.Width - 40) "@x=$($rb[0])"

# ── 4. 核心：从收起态拖出来（用户报的 bug）──────────────
"`n[4] 从收起态往屏内拖 → 必须拖得出来"
$x2 = $rb[0]
for ($i = 0; $i -lt 10; $i++) {
    $x2 -= 40
    [ED]::Move($cal, $x2, $rb[1]); Start-Sleep -Milliseconds 240
}
Start-Sleep -Milliseconds 1500
$out = [ED]::Rect($cal)
$sw = StoreWidget 'calendar'
Check "窗口已回到屏内" ($out[0] + $out[2] -le $vs.Width - 10) "@x=$($out[0]) 右缘=$($out[0]+$out[2]) 屏宽=$($vs.Width)"
Check "edgeHide 已清除" (-not $sw.edgeHide) "实际=$($sw.edgeHide)"
Check "位置已落库（重启不丢）" ([Math]::Abs($sw.rect.x * 1.25 - $out[0]) -lt 40) "store x=$($sw.rect.x) 实际物理=$($out[0])"

# ── 5. 重启后不该自己吸回边上 ───────────────────────────
"`n[5] 重启后保持自由"
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep 3
Start-Process $exe
Start-Sleep 14
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$cal2 = [ED]::Find([uint32]$p.Id, '日历')
$r5 = [ED]::Rect($cal2)
$sw5 = StoreWidget 'calendar'
Check "重启后仍在屏内" ($r5[0] + $r5[2] -le $vs.Width - 10) "@x=$($r5[0]) 右缘=$($r5[0]+$r5[2])"
Check "重启后未重新停靠" (-not $sw5.edgeHide) "实际=$($sw5.edgeHide)"

"`n=== DOCK 日志 ==="
Get-Content "$env:APPDATA\dev.xhy.desktop\xhy.log" | Select-String 'DOCK' | ForEach-Object { "  " + $_.Line }

"`n" + $(if ($fail -eq 0) { "✅ 全部通过" } else { "❌ $fail 项失败" })
exit $fail
