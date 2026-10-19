# 挂件真实输入测试：PostMessage 点击 + 逐字符 WM_KEYDOWN/WM_CHAR
#
# 为什么不用 SendKeys：WebView2 的渲染子窗口属于 msedgewebview2 进程，
# 与 harness 的 PowerShell 线程不同；SendKeys 走系统级注入，按前台线程路由，
# 送不到那个子窗口（实测 GetFocus 能设进去但字进不去）。
# 而真实键盘最终就是给聚焦窗口产生 WM_KEYDOWN/WM_CHAR —— 直接投递这组消息
# 是对"打字"的忠实模拟，且能验证到 DOM 焦点、v-model、防抖落盘整条链路。
#
# 用法：.\type-test.ps1 -Target 便签 -Text 'ABC' -X 190 -Y 210
param(
  [string]$Target = '便签',
  [string]$Text = 'TEST',
  [int]$X = 190,
  [int]$Y = 210,
  [string]$ProcessName = 'xhy',
  [switch]$Enter
)

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class Typer {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int i);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }

    public static void Aware(){ SetProcessDPIAware(); }

    public static IntPtr Find(uint pid, string want){
        IntPtr hit=IntPtr.Zero;
        EnumWindows((h,l)=>{
            uint p; GetWindowThreadProcessId(h,out p);
            if(p!=pid||!IsWindowVisible(h)) return true;
            RC r; GetWindowRect(h,out r); if((r.R-r.L)<40) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            if(tt.ToString().Contains(want)){ hit=h; return false; }
            return true;
        }, IntPtr.Zero);
        return hit;
    }

    public static IntPtr Render(IntPtr parent){
        IntPtr hit=IntPtr.Zero;
        EnumChildWindows(parent,(h,l)=>{
            var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){ hit=h; return false; }
            return true;
        }, IntPtr.Zero);
        return hit;
    }

    static IntPtr Lp(int x,int y){ return (IntPtr)((y<<16)|(x&0xFFFF)); }

    public static void Click(IntPtr h,int x,int y){
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y));
        System.Threading.Thread.Sleep(120);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y));
    }

    /** 只发 WM_CHAR —— 真实键盘的最终产物就是它。
     *  注意不要同时补发 WM_KEYDOWN：Chromium 会自己从 keydown 合成字符，
     *  两个都发会让每个字重复（实测 'NOTE' 变成 'NnnOooTttEee'）。 */
    public static void Type(IntPtr h, string s){
        foreach(char c in s){
            PostMessage(h,0x0102,(IntPtr)(int)c,IntPtr.Zero);      // WM_CHAR
            System.Threading.Thread.Sleep(50);
        }
    }

    /** VK_RETURN 走 keydown+char(\r)+keyup，触发 Enter 提交 */
    public static void Enter(IntPtr h){
        PostMessage(h,0x0100,(IntPtr)13,IntPtr.Zero);
        PostMessage(h,0x0102,(IntPtr)13,IntPtr.Zero);
        PostMessage(h,0x0101,(IntPtr)13,IntPtr.Zero);
    }

    public static string Fg(uint pid){
        IntPtr f=GetForegroundWindow(); uint p; GetWindowThreadProcessId(f,out p);
        var tt=new StringBuilder(128); GetWindowTextW(f,tt,128);
        return string.Format("'{0}' mine={1}", tt, p==pid);
    }
    public static bool NoAct(IntPtr h){ return (GetWindowLong(h,-20)&0x08000000)!=0; }
}
'@

[Typer]::Aware()
$p = Get-Process -Name $ProcessName -EA SilentlyContinue | Select-Object -First 1
if (-not $p) { "$ProcessName 未运行"; exit 1 }
$pid32 = [uint32]$p.Id

$top = [Typer]::Find($pid32, $Target)
if (-not $top) { "找不到窗口 '$Target'"; exit 1 }
$render = [Typer]::Render($top)
if ($render -eq [IntPtr]::Zero) { "无 webview 子窗口"; exit 1 }

"目标 '$Target'  点击前 NOACTIVATE=$([Typer]::NoAct($top))  前台=$([Typer]::Fg($pid32))"
[Typer]::Click($render, $X, $Y)
Start-Sleep -Milliseconds 900
"点击后 NOACTIVATE=$([Typer]::NoAct($top))  前台=$([Typer]::Fg($pid32))"
[Typer]::Type($render, $Text)
Start-Sleep -Milliseconds 400
if ($Enter) {
  # 便签是防抖自动保存不需要回车；待办/日程的快速添加必须回车才提交
  [Typer]::Enter($render)
  Start-Sleep -Milliseconds 400
}
Start-Sleep -Seconds 2
"已投递文本: $Text$(if($Enter){' + Enter'})"
