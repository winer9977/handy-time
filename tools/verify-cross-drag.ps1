# 端到端验证「拖待办到日历生成日程」：用真实鼠标事件跨窗口拖拽
#
# 关键点：拖拽由 Rust 侧接管（幽灵窗 + 全局抬起检测），所以必须用
# 真实鼠标 down/move/up，前端合成事件测不到跨窗口路径。
# 本进程显式声明 DPI-aware，让 GetWindowRect / SetCursorPos 都用物理像素，
# 避免坐标虚拟化带来的偏差。
param(
    [string]$PickTitle = '写季度总结',   # 要拖的待办
    [string]$TargetDate = '2026-12-25'   # 拖到日历上的哪一天
)

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class Mouse {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint d, uint e, uint extra, IntPtr ptr);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr p, IntPtr after, string c, string n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);

    public struct RECT { public int L,T,R,B; }
    public struct Win { public IntPtr H; public string Title; public int L,T,R,B; }

    const uint DOWN = 0x0002, UP = 0x0004, MOVE = 0x0001;

    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    delegate bool EnumProc(IntPtr h, IntPtr l);

    // 挂件在 BottomZ 模式下位于所有普通窗口之下：不先最小化其它窗口，
    // 合成的鼠标点击会落在盖住它的那个窗口上，测不到拖拽。
    public static int MinimizeOthers(uint myPid) {
        int n = 0;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid == myPid) return true;
            if (!IsWindowVisible(h)) return true;
            RECT r; GetWindowRect(h, out r);
            if ((r.R - r.L) < 80 || (r.B - r.T) < 80) return true;   // 工具条/托盘等
            var cl = new StringBuilder(64); GetClassNameW(h, cl, 64);
            string c = cl.ToString();
            if (c == "Progman" || c == "WorkerW" || c == "Shell_TrayWnd"
                || c == "Shell_SecondaryTrayWnd" || c == "Windows.UI.Core.CoreWindow"
                || c == "ApplicationFrameWindow" || c == "NotifyIconOverflowWindow"
                || c == "TaskbarCreated" || c == "DV2ControlHost") return true;
            ShowWindow(h, 6); // SW_MINIMIZE
            n++;
            return true;
        }, IntPtr.Zero);
        return n;
    }

    public static void Aware() { SetProcessDPIAware(); }
    public static void Down() { mouse_event(DOWN, 0,0,0,IntPtr.Zero); }
    public static void Up()   { mouse_event(UP, 0,0,0,IntPtr.Zero); }
    public static void Go(int x, int y) { SetCursorPos(x, y); }
    public static void Bring(IntPtr h) { SetForegroundWindow(h); }

    public static List<Win> Find(uint pid) {
        var res = new List<Win>();
        var parents = new List<IntPtr> { IntPtr.Zero, FindWindowW("Progman", null) };
        IntPtr ww = IntPtr.Zero;
        while (true) { ww = FindWindowExW(IntPtr.Zero, ww, "WorkerW", null); if (ww == IntPtr.Zero) break; parents.Add(ww); }
        foreach (var par in parents) {
            IntPtr ch = IntPtr.Zero; int guard = 0;
            while (guard++ < 5000) {
                ch = FindWindowExW(par, ch, null, null);
                if (ch == IntPtr.Zero) break;
                uint p; GetWindowThreadProcessId(ch, out p);
                if (p != pid) continue;
                RECT r; GetWindowRect(ch, out r);
                if (!IsWindowVisible(ch) || (r.R-r.L) < 40) continue;
                var tt = new StringBuilder(128); GetWindowTextW(ch, tt, 128);
                res.Add(new Win { H = ch, Title = tt.ToString(), L = r.L, T = r.T, R = r.R, B = r.B });
            }
        }
        return res;
    }
}
'@

# 用 WebView2 的 CDP 端口不方便；这里改为直接读 store.json 判定结果
$ErrorActionPreference = 'Stop'
[Mouse]::Aware()

$p = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未运行"; exit 1 }
$id = [uint32]$p.Id

$wins = [Mouse]::Find($id)
"窗口:"
$wins | ForEach-Object { "  '{0}' 物理=({1},{2})-({3},{4})" -f $_.Title, $_.L, $_.T, $_.R, $_.B }

$todo = $wins | Where-Object { $_.Title -match '待办' } | Select-Object -First 1
$cal  = $wins | Where-Object { $_.Title -match '日历' } | Select-Object -First 1
if (-not $todo -or -not $cal) { "找不到待办/日历窗口"; exit 1 }

# 挂件内部布局（逻辑像素，与前端 CSS 对齐）：
#   头部 ~30px + 标签行 ~28px，之后每行约 52px；行内文本区在 x≈40..250
# 物理 = 逻辑 * 1.25（本机缩放）
$s = 1.25
$rowIndex = 0
$scale = $s
$grabX = $todo.L + [int](60 * $scale)                       # 行的文字区
$grabY = $todo.T + [int]((30 + 28 + 26 + $rowIndex * 52) * $scale)

"抓取点(物理) = ($grabX, $grabY)  待办窗口原点=($($todo.L),$($todo.T))"

# 目标：日历里 TargetDate 那一天。先算它在月历网格里的位置。
# 日历窗口：头部 30 + 星期行 18，之后 6 行 × 每行 (H-48)/6，7 列
$calW = ($cal.R - $cal.L) / $scale
$calH = ($cal.B - $cal.T) / $scale
$cellW = $calW / 7.0
$gridTop = 30 + 18
$cellH = ($calH - $gridTop) / 6.0

# 用 .NET 算出该日期在 2026-12 月历网格中的行列（周一起始）
$d = [datetime]::ParseExact($TargetDate, 'yyyy-MM-dd', $null)
$first = Get-Date -Year $d.Year -Month $d.Month -Day 1
$lead = ([int]$first.DayOfWeek + 6) % 7          # 周一起算的前置空格
$dayIdx = $lead + ($d.Day - 1)
$col = [int][math]::Floor($dayIdx / 7)
$row = $dayIdx % 7

$cellCX = $cal.L + [int](($col * $cellW + $cellW / 2) * $scale)
$cellCY = $cal.T + [int](($gridTop + $row * $cellH + $cellH / 2) * $scale)
"落点(物理) = ($cellCX, $cellCY)  目标 $TargetDate → 第 $col 行第 $row 列"

# 记录拖拽前的事件数
$sf = "$env:APPDATA\dev.xhy.desktop\store.json"
$before = ((Get-Content $sf -Raw | ConvertFrom-Json).events | Measure-Object).Count
"拖拽前事件数: $before"

# 关键：先最小化其它窗口，否则 BottomZ 挂件被盖住，点击落不到它身上
$min = [Mouse]::MinimizeOthers($id)
"已最小化其它窗口: $min 个"
Start-Sleep -Milliseconds 900

# 真实拖拽：移到抓取点 → 按下 → 分步移动 → 松开
[Mouse]::Go($grabX, $grabY); Start-Sleep -Milliseconds 250
[Mouse]::Down()
# 先小幅移动触发阈值（>6 逻辑像素），再分多步移向目标
[Mouse]::Go($grabX + 14, $grabY + 6); Start-Sleep -Milliseconds 120
[Mouse]::Go($grabX + 30, $grabY + 20); Start-Sleep -Milliseconds 150

# 诊断：拖拽应已起，Rust 会建出 label=drag-ghost 的幽灵窗
$mid = [Mouse]::Find($id) | ForEach-Object { $_.Title }
"诊断-拖拽中窗口标题: " + (($mid | Sort-Object -Unique) -join ' | ')
$ghost = $mid | Where-Object { $_ -match 'xhy' }
"诊断-幽灵窗存在: " + [bool]$ghost
$steps = 26
for ($i = 1; $i -le $steps; $i++) {
    $x = $grabX + 14 + [int](($cellCX - $grabX - 14) * $i / $steps)
    $y = $grabY + 6  + [int](($cellCY - $grabY - 6)  * $i / $steps)
    [Mouse]::Go($x, $y)
    Start-Sleep -Milliseconds 45
}
Start-Sleep -Milliseconds 200
[Mouse]::Up()
Start-Sleep -Milliseconds 1500

$after = ((Get-Content $sf -Raw | ConvertFrom-Json).events | Measure-Object).Count
"拖拽后事件数: $after"
$ev = (Get-Content $sf -Raw | ConvertFrom-Json).events | Where-Object { $_.date -eq $TargetDate }
if ($ev) {
  "✅ 跨窗口拖拽成功：$TargetDate 上生成了日程"
  $ev | ForEach-Object { "   - title='{0}' source={1} todoId={2}" -f $_.title, $_.source, $_.todoId }
} else {
  "❌ 未在 $TargetDate 生成日程"
}
