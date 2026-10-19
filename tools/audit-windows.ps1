# xhy 窗口体检：枚举进程窗口，验证样式/几何/父窗口/可见性
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Collections.Generic;

public class WinAudit {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern int GetWindowTextLengthW(IntPtr h);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint f);
    [DllImport("user32.dll")] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", SetLastError=true)] static extern int GetWindowLongW(IntPtr h, int i);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    public struct RECT { public int L, T, R, B; }

    const uint GA_ROOT = 2;
    const int GWL_EXSTYLE = -20;
    const int GWL_STYLE = -16;
    const uint WS_CHILD = 0x40000000, WS_VISIBLE = 0x10000000;
    const uint WS_EX_NOACTIVATE = 0x08000000, WS_EX_TOOLWINDOW = 0x00000080, WS_EX_LAYERED = 0x00080000, WS_EX_TRANSPARENT = 0x00000020;

    public static List<string> Audit(uint targetPid) {
        var res = new List<string>();
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;
            // 也含子窗口：对每个 root 检查
            IntPtr root = GetAncestor(h, GA_ROOT);
            if (root != h) return true;   // 只报顶层
            var cls = new StringBuilder(128); GetClassNameW(h, cls, 128);
            var title = new StringBuilder(256); GetWindowTextW(h, title, 256);
            RECT r; GetWindowRect(h, out r);
            int ex = GetWindowLongW(h, GWL_EXSTYLE);
            int st = GetWindowLongW(h, GWL_STYLE);
            IntPtr par = GetParent(h);
            res.Add(string.Format(
                "hwnd=0x{0:X} cls={1} title='{2}' rect=({3},{4})-({5},{6}) vis={7} child={8} noactivate={9} toolwin={10} layered={11} parent=0x{12:X}",
                h.ToInt64(), cls, title, r.L, r.T, r.R, r.B,
                IsWindowVisible(h), (st & WS_CHILD) != 0, (ex & WS_EX_NOACTIVATE) != 0,
                (ex & WS_EX_TOOLWINDOW) != 0, (ex & WS_EX_LAYERED) != 0,
                par == IntPtr.Zero ? 0 : par.ToInt64()));
            return true;
        }, IntPtr.Zero);

        // 第二遍：该进程拥有的所有窗口（含 WorkerW 里的子窗口）
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;
            IntPtr root = GetAncestor(h, GA_ROOT);
            if (root == h) return true; // 顶层已报过
            var cls = new StringBuilder(128); GetClassNameW(h, cls, 128);
            RECT r; GetWindowRect(h, out r);
            IntPtr par = GetParent(h);
            uint ppid; GetWindowThreadProcessId(par, out ppid);
            var pcls = new StringBuilder(128); if (par != IntPtr.Zero) GetClassNameW(par, pcls, 128);
            res.Add(string.Format(
                "CHILD hwnd=0x{0:X} cls={1} rect=({2},{3})-({4},{5}) parent=0x{6:X} pcls={7} ppid={8}",
                h.ToInt64(), cls, r.L, r.T, r.R, r.B,
                par == IntPtr.Zero ? 0 : par.ToInt64(), pcls, ppid));
            return true;
        }, IntPtr.Zero);
        return res;
    }
}
'@

$ps = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue
if (-not $ps) { "xhy 未运行"; exit 1 }
foreach ($p in $ps) {
  "=== pid $($p.Id) ws=$([int]($p.WorkingSet64/1MB))MB ==="
  [WinAudit]::Audit([uint32]$p.Id) | ForEach-Object { "  " + $_ }
}
