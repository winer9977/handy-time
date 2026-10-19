# 单独验证 Alt+Shift+H 双向切换：量 -> 按 -> 量 -> 按 -> 量
# 之前一次跑出现"第一次没反应"，怀疑是 SendKeys 首击被前台控制台吃掉，
# 这里每步都留足时间并显式打印计数，避免误判成应用 bug。
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class HK {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    delegate bool EnumProc(IntPtr h, IntPtr l);
    public struct RECT { public int L,T,R,B; }

    public static int VisibleWidgets(uint target) {
        int n = 0;
        EnumWindows((h,l)=>{
            uint pid; GetWindowThreadProcessId(h,out pid);
            if (pid != target) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            string t = tt.ToString();
            bool widget = t.Length >= 2 && (t.IndexOf("清单") >= 0 || t.IndexOf("便签") >= 0);
            if (!widget) return true;
            RECT r; GetWindowRect(h,out r);
            if (IsWindowVisible(h) && (r.R-r.L) > 40) n++;
            return true;
        }, IntPtr.Zero);
        return n;
    }
    public static string Foreground(uint target) {
        IntPtr h = GetForegroundWindow();
        uint pid; GetWindowThreadProcessId(h, out pid);
        return pid == target ? "xhy" : ("other:" + pid);
    }
}
'@
Add-Type -AssemblyName System.Windows.Forms

$p = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未运行"; exit 1 }
$id = [uint32]$p.Id

function Count-Vis { [HK]::VisibleWidgets($id) }
function Press {
  # 先把前台切到资源管理器，避免按键被控制台/挂件自己吃掉
  $expl = Get-Process -Name 'explorer' -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if ($expl) { [void][HK]::SetForegroundWindow($expl.MainWindowHandle); Start-Sleep -Milliseconds 250 }
  [System.Windows.Forms.SendKeys]::SendWait('%+h')
  Start-Sleep -Milliseconds 1500
}

"初始可见挂件数 : $(Count-Vis)  前台=$(( [HK]::Foreground($id) ))"
Press
"第一次按之后   : $(Count-Vis)"
Press
"第二次按之后   : $(Count-Vis)"
Press
"第三次按之后   : $(Count-Vis)"
