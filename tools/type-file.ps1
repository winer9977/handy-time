param([string]$TextFile, [switch]$Enter, [int]$X = 194, [int]$Y = 431, [string]$Target = '待办')
# 从 UTF-8 文件读入待输入文本，绕开 pwsh 命令行参数对 CJK 的截断
$Text = [System.IO.File]::ReadAllText($TextFile, [System.Text.Encoding]::UTF8).TrimEnd("`r","`n")
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class T5 {
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
    public static void Aware(){SetProcessDPIAware();}
    public static IntPtr Find(uint pid, string want){ IntPtr hit=IntPtr.Zero;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            RC r; GetWindowRect(h,out r); if((r.R-r.L)<40) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            if(tt.ToString().Contains(want)){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static IntPtr Render(IntPtr parent){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(parent,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)((y<<16)|(x&0xFFFF)); }
    public static void Click(IntPtr h,int x,int y){ PostMessage(h,0x0201,(IntPtr)1,Lp(x,y));
        System.Threading.Thread.Sleep(120); PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
    /** ASCII 走 WM_CHAR（实测逐字符准确落盘）。
     *  非 ASCII 无法用合成消息可靠注入（WM_CHAR 乱码、WM_IME_* 被忽略、
     *  WM_PASTE/Ctrl+V 均不进 DOM）—— 中文请见 README 的人工验证项。 */
    public static void Type(IntPtr h, string s){
        foreach(char c in s){
            if(c <= 127){
                PostMessage(h,0x0102,(IntPtr)(int)c,IntPtr.Zero);
            }
            System.Threading.Thread.Sleep(60);
        }
    }
    public static void Enter(IntPtr h){ PostMessage(h,0x0100,(IntPtr)13,IntPtr.Zero);
        PostMessage(h,0x0102,(IntPtr)13,IntPtr.Zero); PostMessage(h,0x0101,(IntPtr)13,IntPtr.Zero); }
    public static bool NA(IntPtr h){ return (GetWindowLong(h,-20)&0x08000000)!=0; }
    public static string Fg(uint pid){ IntPtr f=GetForegroundWindow(); uint p; GetWindowThreadProcessId(f,out p);
        var tt=new StringBuilder(128); GetWindowTextW(f,tt,128); return string.Format("'{0}' mine={1}", tt, p==pid); }
}
'@
[T5]::Aware()
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$pid32 = [uint32]$p.Id
$top = [T5]::Find($pid32, $Target)
$render = [T5]::Render($top)
"点击前 NA=$([T5]::NA($top)) 前台=$([T5]::Fg($pid32))"
[T5]::Click($render, $X, $Y)
Start-Sleep -Milliseconds 900
"点击后 NA=$([T5]::NA($top)) 前台=$([T5]::Fg($pid32))"
[T5]::Type($render, $Text)
Start-Sleep -Milliseconds 400
if ($Enter) { [T5]::Enter($render) }
Start-Sleep -Seconds 2
"sent: $Text (len=$($Text.Length) codepoints=$(($Text.ToCharArray()|ForEach-Object{[int]$_}) -join ','))"
