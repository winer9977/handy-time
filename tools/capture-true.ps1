# 真分辨率截取 xhy 挂件（DPI-aware，避免 PrintWindow 裁边）
# 用法: capture-true.ps1 [-OutDir 目录] [-Raise]
#   -Raise 会把挂件临时抬到最前，便于在有其它窗口遮挡时截图；
#   注意抬到最前后，半透明挂件会透出下方窗口 —— 那是截图副作用，不是产品缺陷。
param(
    [string]$OutDir = (Join-Path (Split-Path -Parent $PSScriptRoot) 'screenshots\live'),
    [switch]$Raise
)
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class True {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr a, int x,int y,int w,int hh, uint f);
    delegate bool E(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }
    public struct W { public IntPtr H; public string Title; public int L,T,Wd,Ht; }
    public static void Aware(){ SetProcessDPIAware(); }
    public static List<W> Of(uint pid){
        var res=new List<W>();
        EnumWindows((h,l)=>{
            uint p; GetWindowThreadProcessId(h,out p);
            if(p!=pid||!IsWindowVisible(h)) return true;
            RC r; GetWindowRect(h,out r);
            if((r.R-r.L)<40) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            res.Add(new W{H=h,Title=tt.ToString(),L=r.L,T=r.T,Wd=r.R-r.L,Ht=r.B-r.T});
            return true;
        }, IntPtr.Zero);
        return res;
    }
    // 抬到最顶再取消置顶：临时可见，且不留下 TOPMOST 状态
    public static void Peek(IntPtr h){
        SetWindowPos(h,(IntPtr)(-1),0,0,0,0,0x1|0x2|0x10);   // TOPMOST, NOACTIVATE|NOMOVE|NOSIZE
        SetWindowPos(h,(IntPtr)(-2),0,0,0,0,0x1|0x2|0x10);   // NOTOPMOST
    }
}
'@
[True]::Aware()
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Remove-Item "$OutDir\*.png" -Force -EA SilentlyContinue
$p = Get-Process -Name 'xhy' -EA SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未运行"; exit 1 }
$ws = [True]::Of([uint32]$p.Id)
if ($Raise) { foreach ($w in $ws) { [True]::Peek($w.H) }; Start-Sleep -Milliseconds 600 }
$bmp = New-Object System.Drawing.Bitmap 1920,1080
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen(0,0,0,0,(New-Object System.Drawing.Size 1920,1080))
$g.Dispose()
foreach ($w in $ws) {
  $safe = ($w.Title -replace '[^\w\u4e00-\u9fff]','')
  $c = New-Object System.Drawing.Bitmap $w.Wd, $w.Ht
  $gg = [System.Drawing.Graphics]::FromImage($c)
  $gg.DrawImage($bmp, (New-Object System.Drawing.Rectangle 0,0,$w.Wd,$w.Ht),
                    (New-Object System.Drawing.Rectangle $w.L,$w.T,$w.Wd,$w.Ht), [System.Drawing.GraphicsUnit]::Pixel)
  $gg.Dispose()
  $path = Join-Path $OutDir "$safe.png"
  $c.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $c.Dispose()
  "saved $safe.png $($w.Wd)x$($w.Ht)"
}
$bmp.Dispose()
