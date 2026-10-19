# 主窗口日历多视图：逐个点击视图按钮并截图（对照滴答的年/月/周/日/多日/多周）
# 用法: powershell -File tools/shot-calviews.ps1
$w = Split-Path -Parent $PSScriptRoot | Split-Path -Parent   # 项目根（tools\capture-windows.ps1 在这）
$cap = Join-Path $w 'tools\capture-windows.ps1'
if (-not (Test-Path $cap)) { "找不到 $cap"; exit 1 }
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class CV {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref PT p);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }
    public struct PT { public int x,y; }
    public static void Aware(){SetProcessDPIAware();}
    public static IntPtr Top(uint pid, string want){ IntPtr hit=IntPtr.Zero;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            var tt=new StringBuilder(64); GetWindowTextW(h,tt,64);
            if(tt.ToString().Contains(want)){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static IntPtr Render(IntPtr top){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(top,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static int[] Offsets(IntPtr h){
        RC r; GetWindowRect(h,out r);
        PT p=new PT(); p.x=0; p.y=0; ClientToScreen(h,ref p);
        return new[]{ p.x-r.L, p.y-r.T };   // 截图坐标 → 客户区坐标要减掉的偏移
    }
    static IntPtr Lp(int x,int y){ return (IntPtr)((y<<16)|(x&0xFFFF)); }
    public static void Click(IntPtr h,int x,int y){ PostMessage(h,0x0200,(IntPtr)0,Lp(x,y));
        System.Threading.Thread.Sleep(70);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(170);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
}
'@
[CV]::Aware()
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$mw = [CV]::Top([uint32]$p.Id, '工作台')
if (-not $mw) { "找不到主窗口"; exit 1 }
$rc = [CV]::Render($mw)
$off = [CV]::Offsets($mw)
$ox = $off[0]; $oy = $off[1]
"标题栏偏移 = ($ox,$oy)"

function ClickShot([int]$sx, [int]$sy, [string]$name) {
    [CV]::Click($rc, $sx - $ox, $sy - $oy)
    Start-Sleep -Milliseconds 1300
    $tmp = "$w\.cs-$name"
    & $cap -OutDir $tmp -ProcessName 'xhy' 2>&1 | Out-Null
    $src = Get-ChildItem $tmp -Filter '*工作台*' | Select-Object -First 1
    if ($src) { Copy-Item $src.FullName "$dst\cv-$name.png" -Force; "  截图 cv-$name.png" }
    else { "  MISS $name" }
    Remove-Item $tmp -Recurse -Force -EA SilentlyContinue
}
$dst = "$w\.calshots"
New-Item -ItemType Directory -Force -Path $dst | Out-Null

# 1) 进日历视图（顶栏第 4 个 tab）
"进日历视图"
ClickShot 494 59 'month'
# 2) CalendarPane 自己的视图条。y 从首张截图读准：按钮中心在 y≈114
#    （先前用 95 拍出来六张与 month 逐字节相同 = 根本没点中）
"逐个切视图 (y=114)"
ClickShot 270 114 'year'
ClickShot 308 114 'month2'
ClickShot 340 114 'week'
ClickShot 373 114 'day'
ClickShot 410 114 'days'
ClickShot 452 114 'weeks'
"完成：$dst"
