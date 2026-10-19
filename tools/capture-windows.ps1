# 逐窗口 PrintWindow 抓取（不受遮挡影响），验证挂件真实渲染
param(
  [string]$OutDir = (Join-Path (Split-Path -Parent $PSScriptRoot) 'shots'),
  [string]$ProcessName = 'xhy'
)

Add-Type -AssemblyName System.Drawing

# 只放 P/Invoke：位图在 PowerShell 层用已加载的 System.Drawing 处理
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class WinEnum {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    // 关键：本进程必须 DPI-aware。否则 GetWindowRect 返回的是被虚拟化的
    // 逻辑尺寸（310x400），而窗口真实表面是物理尺寸（387x500），
    // PrintWindow 到按逻辑尺寸建的位图里 = 右侧和底部各裁掉 ~20%。
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    public struct RECT { public int L, T, R, B; }
    public struct Info { public IntPtr Hwnd; public int X, Y, W, H; public string Cls, Title; }

    public static List<Info> List(uint targetPid) {
        var res = new List<Info>();
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;
            if (!IsWindowVisible(h)) return true;
            RECT r; GetWindowRect(h, out r);
            int w = r.R - r.L, ht = r.B - r.T;
            if (w < 40 || ht < 40) return true;
            var cls = new StringBuilder(64); GetClassNameW(h, cls, 64);
            var tt = new StringBuilder(256); GetWindowTextW(h, tt, 256);
            res.Add(new Info { Hwnd = h, X = r.L, Y = r.T, W = w, H = ht,
                               Cls = cls.ToString(), Title = tt.ToString() });
            return true;
        }, IntPtr.Zero);
        return res;
    }

    public static bool Grab(IntPtr h, IntPtr hdc) {
        return PrintWindow(h, hdc, 0x2 /* PW_RENDERFULLCONTENT */);
    }
}
'@

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
[void][WinEnum]::SetProcessDPIAware()
Remove-Item "$OutDir\*.png" -Force -ErrorAction SilentlyContinue

$p = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "$ProcessName 未运行"; exit 1 }
"pid=$($p.Id)"

$wins = [WinEnum]::List([uint32]$p.Id)
"可见窗口数: $($wins.Count)"
$i = 0
foreach ($w in $wins) {
  $bmp = New-Object System.Drawing.Bitmap $w.W, $w.H
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $g.GetHdc()
  $ok = [WinEnum]::Grab($w.Hwnd, $hdc)
  $g.ReleaseHdc($hdc)
  $g.Dispose()
  $name = "win{0}-{1}.png" -f $i, ($w.Title -replace '[^\w\u4e00-\u9fff]', '')
  $path = Join-Path $OutDir $name
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  "  [{0}] hwnd=0x{1:X} '{2}' cls={3} {4}x{5} @({6},{7}) print={8} -> {9}" -f `
    $i, $w.Hwnd.ToInt64(), $w.Title, $w.Cls, $w.W, $w.H, $w.X, $w.Y, $ok, $name
  $i++
}
