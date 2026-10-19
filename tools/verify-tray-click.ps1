# 验证「托盘左键单击打开工作台」
#
# 方法：直接向托盘隐藏窗口投递 WM_USER_TRAYICON(6002)，wParam=0、lParam=WM_LBUTTONUP(0x0202)。
# 为什么这条路可行而 SendInput 不行：
#   · SendInput 走全局输入队列，被本沙箱完整性级别拦截（早前实测确认）
#   · 托盘回调是**进程内窗口消息**，PostMessage 直达，不经输入队列
# 依据来自 tray-icon 源码（权威，非猜测）：
#   · 回调消息号 WM_USER_TRAYICON = 6002
#   · 窗口类名 "tray_icon_app"
#   · WM_LBUTTONUP → TrayIconEvent::Click { button: Left, button_state: Up }
#     （正是 tray_opens_main 要求的组合）
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class TR {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public static void Init(){SetProcessDPIAware();}

    /// 找托盘隐藏窗口：类名 tray_icon_app，属于 xhy 进程
    /// 注意 EnumWindows 只枚举**顶层**窗口，托盘窗口不可见但仍是顶层，能枚举到
    public static long TrayWnd(uint pid){
        long hit = 0;
        EnumWindows((h,l)=>{
            uint p; GetWindowThreadProcessId(h, out p);
            if (p != pid) return true;
            var cl = new StringBuilder(128); GetClassNameW(h, cl, 128);
            if (cl.ToString() == "tray_icon_app") { hit = h.ToInt64(); return false; }
            return true;
        }, IntPtr.Zero);
        return hit;
    }
    /// 找主窗口（标题 xhy 工作台），可见
    public static bool MainVisible(uint pid){
        bool found = false;
        EnumWindows((h,l)=>{
            uint p; GetWindowThreadProcessId(h, out p);
            if (p != pid || !IsWindowVisible(h)) return true;
            var tt = new StringBuilder(128); GetWindowTextW(h, tt, 128);
            if (tt.ToString() == "xhy 工作台") { found = true; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static bool MainExists(uint pid){
        bool found = false;
        EnumWindows((h,l)=>{
            uint p; GetWindowThreadProcessId(h, out p);
            if (p != pid) return true;
            var tt = new StringBuilder(128); GetWindowTextW(h, tt, 128);
            if (tt.ToString() == "xhy 工作台") { found = true; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    /// 模拟托盘左键单击：投 WM_USER_TRAYICON + lParam=WM_LBUTTONUP
    public static bool ClickTray(long trayHwnd){
        if (trayHwnd == 0) return false;
        return PostMessage((IntPtr)trayHwnd, 6002, IntPtr.Zero, (IntPtr)0x0202);
    }
}
'@
[TR]::Init()

$exe = "$env:LOCALAPPDATA\xhy\xhy.exe"
$fail = 0
function Check($n,[bool]$ok,$d=''){ if($ok){"  PASS  $n $d"}else{$script:fail++;"  FAIL  $n $d"} }

Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 3
Start-Process $exe
Start-Sleep -Seconds 14

$p = Get-Process -Name 'xhy' -EA SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未启动"; exit 1 }
$pid32 = [uint32]$p.Id
"PID=$pid32"

$tray = [TR]::TrayWnd($pid32)
Check '找到托盘窗口（类名 tray_icon_app）' ($tray -ne 0) "(hwnd=$tray)"

# 前置确认：主窗口此刻不存在（应用刚启动，主窗口只在需要时建）
Check '前置：主窗口尚未创建' (-not [TR]::MainExists($pid32))

"=== 投递托盘左键单击（WM_USER_TRAYICON + WM_LBUTTONUP）==="
$posted = [TR]::ClickTray($tray)
Check '消息投递成功' $posted
Start-Sleep -Seconds 4

Check '★ 主窗口已被创建' ([TR]::MainExists($pid32))
Check '★ 主窗口可见（已显示到前台）' ([TR]::MainVisible($pid32))

# 再点一次：主窗口已存在，应只是重新显示/置前，不该出错
"=== 再单击一次（主窗口已存在）==="
[void][TR]::ClickTray($tray)
Start-Sleep -Seconds 3
Check '重复单击仍可见（幂等，不崩）' ([TR]::MainVisible($pid32))
Check '进程仍响应' ($null -ne (Get-Process -Name 'xhy' -EA SilentlyContinue))

# 反向验证：左键**按下**不该开窗（tray_opens_main 只认 Up）
"=== 反例：只投 WM_LBUTTONDOWN 不该触发 ==="
Get-Process -Name 'xhy' | Stop-Process -Force
Start-Sleep -Seconds 3
Start-Process $exe
Start-Sleep -Seconds 14
$p2 = Get-Process -Name 'xhy' | Select-Object -First 1
$pid2 = [uint32]$p2.Id
$tray2 = [TR]::TrayWnd($pid2)
if ($tray2 -ne 0) {
    # 只投"按下"，不投"松开"
    [TR]::PostMessage([IntPtr]$tray2, 6002, [IntPtr]::Zero, [IntPtr]0x0201) | Out-Null
}
Start-Sleep -Seconds 3
Check '反例：仅按下不开窗（符合 tray_opens_main 只认 Up）' (-not [TR]::MainExists($pid2))

""
if ($fail -eq 0) { "ALL PASS" } else { "$fail FAILED" }
exit $fail
