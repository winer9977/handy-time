# 重复日程端到端验证：真实点击切视图 → 加日程 → 设重复 → 校验落盘与虚拟展开
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Collections.Generic;using System.Runtime.InteropServices;
public class Rc {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }
    public struct Win { public IntPtr H; public string Title; public int L,T,Wd,Ht; }
    public static void Aware(){SetProcessDPIAware();}
    public static List<Win> Find(uint pid){
        var res=new List<Win>();
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p);
            if(p!=pid||!IsWindowVisible(h)) return true;
            RC r; GetWindowRect(h,out r); if((r.R-r.L)<40) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            res.Add(new Win{H=h,Title=tt.ToString(),L=r.L,T=r.T,Wd=r.R-r.L,Ht=r.B-r.T});
            return true; }, IntPtr.Zero);
        return res;
    }
    public static IntPtr Render(IntPtr parent){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(parent,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)((y<<16)|(x&0xFFFF)); }
    public static void Click(IntPtr h,int x,int y){ PostMessage(h,0x0201,(IntPtr)1,Lp(x,y));
        System.Threading.Thread.Sleep(140); PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
    public static void Type(IntPtr h,string s){ foreach(char c in s){ if(c<=127) PostMessage(h,0x0102,(IntPtr)(int)c,IntPtr.Zero); System.Threading.Thread.Sleep(50);} }
    public static void Enter(IntPtr h){ PostMessage(h,0x0100,(IntPtr)13,IntPtr.Zero); PostMessage(h,0x0102,(IntPtr)13,IntPtr.Zero); PostMessage(h,0x0101,(IntPtr)13,IntPtr.Zero); }
}
'@
[Rc]::Aware()
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$pid32 = [uint32]$p.Id
$store = "$env:APPDATA\dev.xhy.desktop\store.json"
function EvCount { ((Get-Content $store -Raw | ConvertFrom-Json).events | Measure-Object).Count }

$cal = [Rc]::Find($pid32) | Where-Object { $_.Title -match '日历' } | Select-Object -First 1
if (-not $cal) { "找不到日历窗口"; exit 1 }
$r = [Rc]::Render($cal.H)
"日历 @($($cal.L),$($cal.T)) $($cal.Wd)x$($cal.Ht)"

# 1) 切到日视图：头部「日」按钮 CSS(296,16) → 物理 *1.25
[Rc]::Click($r, [int](296*1.25), [int](16*1.25))
Start-Sleep -Milliseconds 1000
$v = ((Get-Content $store -Raw | ConvertFrom-Json).widgets | Where-Object { $_.kind -eq 'calendar' }).view
"1) 切日视图 -> view=$v"

# 2) 点「加日程」按钮（日视图底部，物理约 (194,163)）
[Rc]::Click($r, 194, 163)
Start-Sleep -Milliseconds 1000
# 3) 输入标题并回车
[Rc]::Type($r, 'weekly sync')
Start-Sleep -Milliseconds 300
[Rc]::Enter($r)
Start-Sleep -Seconds 2
$j = Get-Content $store -Raw | ConvertFrom-Json
$e1 = $j.events | Select-Object -First 1
"2) 新建日程 -> title='$($e1.title)' date=$($e1.date) rc=$($e1.recurrence)"

# 4) 点该行的 ↻ 按钮打开重复弹层。行在 day-h 下方：物理约 (330, 100)
[Rc]::Click($r, 330, 100)
Start-Sleep -Milliseconds 900
& (Join-Path $PSScriptRoot 'capture-windows.ps1') -OutDir (Join-Path (Split-Path -Parent $PSScriptRoot) '.t\rc') -ProcessName 'xhy' 2>&1 | Out-Null
"3) 已点 ↻，截图见 .t\rc"
