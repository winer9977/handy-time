# 最小验证：合成鼠标点击能否触达挂件 webview
# 点待办第一行的复选框 → 若 store 里 done 变 true，说明输入链路通
param([switch]$Restore)

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class Tiny {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint d, uint e, uint extra, IntPtr ptr);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr p, IntPtr after, string c, string n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
    public struct RECT { public int L,T,R,B; }
    public struct POINT { public int X,Y; }
    public struct Hit { public IntPtr H; public string Title; public int L,T,R,B; }
    public static void Aware(){ SetProcessDPIAware(); }
    public static void Down(){ mouse_event(0x0002,0,0,0,IntPtr.Zero); }
    public static void Up(){ mouse_event(0x0004,0,0,0,IntPtr.Zero); }
    public static void Go(int x,int y){ SetCursorPos(x,y); }
    public static string At(int x,int y){
        var p = new POINT{X=x,Y=y};
        IntPtr h = WindowFromPoint(p);
        var tt = new StringBuilder(128); if (h != IntPtr.Zero) GetWindowTextW(h, tt, 128);
        uint pid; GetWindowThreadProcessId(h, out pid);
        return string.Format("hwnd=0x{0:X} title='{1}' pid={2}", h.ToInt64(), tt, pid);
    }
    public static List<Hit> Find(uint pid){
        var res = new List<Hit>();
        var ps = new List<IntPtr>{ IntPtr.Zero, FindWindowW("Progman", null) };
        IntPtr ww = IntPtr.Zero;
        while (true){ ww = FindWindowExW(IntPtr.Zero, ww, "WorkerW", null); if (ww==IntPtr.Zero) break; ps.Add(ww); }
        foreach (var par in ps){
            IntPtr ch = IntPtr.Zero; int g=0;
            while (g++ < 5000){
                ch = FindWindowExW(par, ch, null, null);
                if (ch == IntPtr.Zero) break;
                uint p; GetWindowThreadProcessId(ch, out p);
                if (p != pid) continue;
                RECT r; GetWindowRect(ch, out r);
                if (!IsWindowVisible(ch) || (r.R-r.L) < 40) continue;
                var tt = new StringBuilder(128); GetWindowTextW(ch, tt, 128);
                res.Add(new Hit{H=ch,Title=tt.ToString(),L=r.L,T=r.T,R=r.R,B=r.B});
            }
        }
        return res;
    }
}
'@
[Tiny]::Aware()
$p = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未运行"; exit 1 }
$id = [uint32]$p.Id
$wins = [Tiny]::Find($id)
$todo = $wins | Where-Object { $_.Title -match '待办' } | Select-Object -First 1
if (-not $todo) { "无待办窗口"; exit 1 }

# 第一行复选框：CSS 约 (16, 84)（头部29 + 标签26 + 行内16 + 半高）
$s = 1.25
$cbX = $todo.L + [int](16 * $s)
$cbY = $todo.T + [int](84 * $s)
"复选框物理坐标 = ($cbX, $cbY)"
"WindowFromPoint 判定该点属于: " + [Tiny]::At($cbX, $cbY)
"对照-待办窗口自身 = hwnd=0x{0:X}" -f $todo.H.ToInt64()

$sf = "$env:APPDATA\dev.xhy.desktop\store.json"
"点击前 done 状态: " + ((Get-Content $sf -Raw | ConvertFrom-Json).todos | ForEach-Object { $_.done })
[Tiny]::Go($cbX, $cbY); Start-Sleep -Milliseconds 300
[Tiny]::Down(); Start-Sleep -Milliseconds 80; [Tiny]::Up()
Start-Sleep -Milliseconds 1200
"点击后 done 状态: " + ((Get-Content $sf -Raw | ConvertFrom-Json).todos | ForEach-Object { $_.done })
