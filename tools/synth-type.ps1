# 合成输入验证：向 Tauri 挂件投递真实鼠标点击 + 键盘输入
#
# 为什么不用 mouse_event：桌面挂件在 z-order 最底层，屏幕上任何普通窗口
# （包括本 harness 自己的全屏窗）都会截走命中测试，导致点击永远落不到挂件上。
# 这里改用 PostMessage 直接发给 webview 的子窗口（Chrome_RenderWidgetHostHWND），
# 绕开 z-order 命中测试，Chromium 仍按真实输入处理。
#
# 用法：
#   .\synth-type.ps1 -Target 便签 -Text 'ABC-123'
param(
  [string]$Target = '便签',
  [string]$Text = 'HELLO',
  [int]$ClientX = 190,
  [int]$ClientY = 200,
  [string]$ProcessName = 'xhy',
  [int]$WaitMs = 1500
)

Add-Type -AssemblyName System.Windows.Forms

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class Synth {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(WE cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int i);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern IntPtr ChildWindowFromPoint(IntPtr parent, PT p);
    delegate bool WE(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }
    public struct PT { public int X,Y; }
    public struct Win { public IntPtr H; public string Title; public int L,T,Wd,Ht; }

    public static void Aware(){ SetProcessDPIAware(); }

    public static List<Win> Top(uint pid, string want){
        var res=new List<Win>();
        EnumWindows((h,l)=>{
            uint p; GetWindowThreadProcessId(h,out p);
            if(p!=pid || !IsWindowVisible(h)) return true;
            RC r; GetWindowRect(h,out r);
            if((r.R-r.L)<40) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            if(tt.ToString().Contains(want))
                res.Add(new Win{H=h,Title=tt.ToString(),L=r.L,T=r.T,Wd=r.R-r.L,Ht=r.B-r.T});
            return true;
        }, IntPtr.Zero);
        return res;
    }

    /** 找 webview 的输入子窗口 */
    public static IntPtr Render(IntPtr parent){
        IntPtr hit=IntPtr.Zero;
        EnumChildWindows(parent,(h,l)=>{
            var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){ hit=h; return false; }
            return true;
        }, IntPtr.Zero);
        return hit;
    }

    public static string ChildAt(IntPtr parent, int x, int y){
        IntPtr c = ChildWindowFromPoint(parent, new PT{X=x,Y=y});
        if(c==IntPtr.Zero) return "<none>";
        var cl=new StringBuilder(128); GetClassNameW(c,cl,128);
        return cl.ToString();
    }

    const uint WM_LBUTTONDOWN=0x0201, WM_LBUTTONUP=0x0202, MK_LBUTTON=0x0001;
    static IntPtr MakeLParam(int x,int y){ return (IntPtr)((y<<16) | (x & 0xFFFF)); }

    public static bool Click(IntPtr h,int x,int y){
        bool a = PostMessage(h, WM_LBUTTONDOWN, (IntPtr)MK_LBUTTON, MakeLParam(x,y));
        System.Threading.Thread.Sleep(120);
        bool b = PostMessage(h, WM_LBUTTONUP, IntPtr.Zero, MakeLParam(x,y));
        return a&&b;
    }

    public static string Fg(uint pid){
        IntPtr f=GetForegroundWindow(); uint p; GetWindowThreadProcessId(f,out p);
        var tt=new StringBuilder(128); GetWindowTextW(f,tt,128);
        return string.Format("'{0}' mine={1}", tt, p==pid);
    }
    public static bool NoAct(IntPtr h){ return (GetWindowLong(h,-20)&0x08000000)!=0; }
}
'@

[Synth]::Aware()
$p = Get-Process -Name $ProcessName -EA SilentlyContinue | Select-Object -First 1
if (-not $p) { "$ProcessName 未运行"; exit 1 }
$pid32 = [uint32]$p.Id

$w = [Synth]::Top($pid32, $Target) | Select-Object -First 1
if (-not $w) { "找不到窗口 '$Target'"; exit 1 }
"窗口 '$($w.Title)' @($($w.L),$($w.T)) $($w.Wd)x$($w.Ht)"

$render = [Synth]::Render($w.H)
if ($render -eq [IntPtr]::Zero) { "找不到 Chrome_RenderWidgetHostHWND 子窗口"; exit 1 }
"webview 子窗口 hwnd=0x$($render.ToInt64().ToString('X'))"
"  客户区该点命中: " + [Synth]::ChildAt($w.H, $ClientX, $ClientY)

"点击前: 前台=$([Synth]::Fg($pid32))  NOACTIVATE=$([Synth]::NoAct($w.H))"
$null = [Synth]::Click($render, $ClientX, $ClientY)
Start-Sleep -Milliseconds $WaitMs
"点击后: 前台=$([Synth]::Fg($pid32))  NOACTIVATE=$([Synth]::NoAct($w.H))"

# 只有窗口真拿到前台，键盘输入才会进 webview
[System.Windows.Forms.SendKeys]::SendWait($Text)
Start-Sleep -Seconds 2
"type sent: $Text"
