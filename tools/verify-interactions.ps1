# 端到端验证：贴边自动隐藏 + 快速隐藏热键
# 用 SetWindowPos 从外部移动窗口（等价于用户拖窗），再观察状态机是否收起
param(
    [ValidateSet('dock','hotkey','align')][string]$Case = 'dock',
    [string]$Edge = 'left'
)

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class DockProbe {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr p, IntPtr after, string c, string n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);

    public struct RECT { public int L,T,R,B; }
    public struct Win { public IntPtr H; public string Title; public int L,T,R,B; public bool Vis; }

    const int SM_CXSCREEN = 0, SM_CYSCREEN = 1;
    public static int ScreenW() { return GetSystemMetrics(SM_CXSCREEN); }
    public static int ScreenH() { return GetSystemMetrics(SM_CYSCREEN); }

    // 找该进程所有可见的够大的窗口（不按标题过滤：中文字面量经 Add-Type
    // 编译易被编码问题破坏，交给 PowerShell 侧筛选）
    public static List<Win> FindAll(uint pid) {
        var res = new List<Win>();
        var parents = new List<IntPtr> { IntPtr.Zero, FindWindowW("Progman", null) };
        IntPtr ww = IntPtr.Zero;
        while (true) { ww = FindWindowExW(IntPtr.Zero, ww, "WorkerW", null); if (ww == IntPtr.Zero) break; parents.Add(ww); }
        foreach (var par in parents) {
            IntPtr ch = IntPtr.Zero; int guard = 0;
            // 上限要大：par=0 时枚举的是全部顶层窗口（含大量不可见的系统/隐藏窗口），
            // 200 个根本轮不到目标进程，之前"找不到窗口"就是这个原因
            while (guard++ < 5000) {
                ch = FindWindowExW(par, ch, null, null);
                if (ch == IntPtr.Zero) break;
                uint p; GetWindowThreadProcessId(ch, out p);
                if (p != pid) continue;
                RECT r; GetWindowRect(ch, out r);
                if (!IsWindowVisible(ch) || (r.R - r.L) < 40) continue;
                var tt = new StringBuilder(128); GetWindowTextW(ch, tt, 128);
                res.Add(new Win { H = ch, Title = tt.ToString(), L = r.L, T = r.T, R = r.R, B = r.B, Vis = true });
            }
        }
        return res;
    }

    public static bool Move(IntPtr h, int x, int y) {
        // SWP_NOSIZE|NOACTIVATE|NOZORDER = 0x1|0x10|0x4
        return SetWindowPos(h, IntPtr.Zero, x, y, 0, 0, 0x1 | 0x10 | 0x4);
    }
}
'@

$p = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) {
    # 应用没在跑也要能测：先起起来（进程检查与前置重启必须在一个流程里，
    # 否则前置重启后这里的 $p 就成了过期 PID）
    & "$PSScriptRoot\ensure-widgets.ps1" 2>&1 | Out-Null
    $p = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue | Select-Object -First 1
}
if (-not $p) { "xhy 未运行且无法启动"; exit 1 }
$pid32 = [uint32]$p.Id
$scale = 1.0   # 本脚本进程是 DPI-unaware，Windows 已把坐标虚拟化回逻辑像素，
              # 所以这里读到的就是逻辑坐标，绝不能再除一次缩放（除一次会得到假数字）

# 只返回对象，显示走 Write-Host：否则格式化字符串会混进管道，
# 调用方拿到的就不是窗口对象列表了（之前"找不到便签窗口"就是这个原因）
function Show($tag) {
  $ws = [DockProbe]::FindAll($pid32)
  foreach ($w in $ws) { Write-Host ("  [{0}] {1} 逻辑=({2},{3})-({4},{5}) vis={6}" -f `
      $tag, $w.Title, $w.L, $w.T, $w.R, $w.B, $w.Vis) }
  return $ws
}

switch ($Case) {
  'dock' {
    "=== 贴边自动隐藏 ==="
    # 前置：确保便签挂件**可见**（可能被上一位使用者或上次测试收起，
    # 否则窗口不存在，脚本会报"找不到便签窗口"—— 实测踩过）。
    # 用 -Kind 只确保可见；若因此重启了应用，重新取 PID（顶层那个已过期）。
    # 这里不能用 Return：PowerShell 函数里 Return 会退出整个脚本作用域，
    # 改用输出判断。
    $before = $pid32
    $out = & "$PSScriptRoot\ensure-widgets.ps1" -Kind note 2>&1
    $out | ForEach-Object { Write-Host "  $_" }
    $p2 = Get-Process -Name 'xhy' -EA SilentlyContinue | Select-Object -First 1
    if (-not $p2) { "重启后仍无 xhy 进程"; exit 1 }
    if ([uint32]$p2.Id -ne $before) {
        $pid32 = [uint32]$p2.Id
        Write-Host "  前置重启换过进程，PID 已刷新为 $pid32"
    }
    $ws = Show '初始'
    # 标题在 PowerShell 侧匹配（脚本文件是 UTF-8，中文正常；C# 侧易被编码破坏）
    $target = $ws | Where-Object { $_.Title -match '便签' } | Select-Object -First 1
    if (-not $target) {
      "找不到便签窗口。实际标题: " + (($ws | ForEach-Object { "'$($_.Title)'" }) -join ', ')
      exit 1
    }
    "→ 把便签移到屏幕左缘（物理 x=0，等价逻辑 x=0）"
    [void][DockProbe]::Move($target.H, 0, $target.T)
    Start-Sleep -Milliseconds 400
    [void](Show '移动后')
    "等待状态机（SETTLE 320ms + 轮询 120ms）…"
    Start-Sleep -Milliseconds 1500
    $after = Show '停靠后'
    $t2 = $after | Where-Object { $_.Title -match '便签' } | Select-Object -First 1
    $w = $t2.R - $t2.L
    "便签左缘=$($t2.L) 右缘=$($t2.R) 宽=$w（停靠应只留 16 逻辑px≈20 物理px 在屏内）"
    if ($t2.R -le 40) { "✅ 已停靠：屏内只剩 $([Math]::Max(0,$t2.R)) px 边条" } else { "❌ 未停靠（屏内 $([Math]::Max(0,$t2.R)) px）" }
  }
  'hotkey' {
    "=== 快速隐藏热键 Alt+Shift+H ==="
    $before = Show '前'
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.SendKeys]::SendWait('%+h')
    Start-Sleep -Milliseconds 1200
    $a = Show '按热键后'
    $visCount = ($a | Where-Object { $_.Vis }).Count
    "隐藏后可见窗口数: $visCount（之前 $($before.Count)）"
    if ($visCount -lt $before.Count) { "✅ 热键生效，挂件被收起" } else { "❌ 未生效" }
    [System.Windows.Forms.SendKeys]::SendWait('%+h')
    Start-Sleep -Milliseconds 1200
    $b = Show '再按一次'
    "恢复后可见窗口数: $(($b | Where-Object { $_.Vis }).Count)"
    if (($b | Where-Object { $_.Vis }).Count -ge $before.Count) { "✅ 恢复显示" } else { "❌ 未恢复" }
  }
}
