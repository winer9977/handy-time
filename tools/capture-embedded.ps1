# 抓取"嵌入桌面层后"的挂件窗口（它们是 Progman 的子窗口，不再是顶层）
param([string]$OutDir = (Join-Path (Split-Path -Parent $PSScriptRoot) 'shots-embedded'))

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class EmbeddedCap {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr p, IntPtr after, string c, string n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    public struct RECT { public int L,T,R,B; }
    public struct Info { public IntPtr Hwnd; public int X,Y,W,H; public string Title, Parent; }

    public static List<Info> Find(uint targetPid) {
        var res = new List<Info>();
        var parents = new List<KeyValuePair<IntPtr,string>>();
        IntPtr pm = FindWindowW("Progman", null);
        if (pm != IntPtr.Zero) parents.Add(new KeyValuePair<IntPtr,string>(pm, "Progman"));
        IntPtr ww = IntPtr.Zero;
        while (true) {
            ww = FindWindowExW(IntPtr.Zero, ww, "WorkerW", null);
            if (ww == IntPtr.Zero) break;
            parents.Add(new KeyValuePair<IntPtr,string>(ww, "WorkerW"));
        }
        foreach (var kv in parents) {
            IntPtr child = IntPtr.Zero;
            int guard = 0;
            while (true) {
                child = FindWindowExW(kv.Key, child, null, null);
                if (child == IntPtr.Zero) break;
                uint pid; GetWindowThreadProcessId(child, out pid);
                if (pid == targetPid) {
                    RECT r; GetWindowRect(child, out r);
                    var tt = new StringBuilder(128); GetWindowTextW(child, tt, 128);
                    if (IsWindowVisible(child) && (r.R-r.L) > 40) {
                        res.Add(new Info { Hwnd = child, X = r.L, Y = r.T, W = r.R-r.L, H = r.B-r.T,
                                           Title = tt.ToString(), Parent = kv.Value });
                    }
                }
                if (++guard > 200) break;
            }
        }
        return res;
    }
    public static bool Grab(IntPtr h, IntPtr hdc) { return PrintWindow(h, hdc, 0x2); }
}
'@

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Remove-Item "$OutDir\*.png" -Force -ErrorAction SilentlyContinue
$p = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未运行"; exit 1 }
$items = [EmbeddedCap]::Find([uint32]$p.Id)
"pid=$($p.Id)  桌面层子窗口数: $($items.Count)"
$i = 0
foreach ($it in $items) {
  $bmp = New-Object System.Drawing.Bitmap $it.W, $it.H
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $g.GetHdc(); $ok = [EmbeddedCap]::Grab($it.Hwnd, $hdc); $g.ReleaseHdc($hdc); $g.Dispose()
  $name = "{0}-{1}.png" -f $i, ($it.Title -replace '[^\w\u4e00-\u9fff]','')
  $bmp.Save((Join-Path $OutDir $name), [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  "  [{0}] '{1}' parent={2} {3}x{4} @({5},{6}) print={7}" -f $i,$it.Title,$it.Parent,$it.W,$it.H,$it.X,$it.Y,$ok
  $i++
}
