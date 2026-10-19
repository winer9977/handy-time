# 检查挂件是否真的被挂进桌面层（Progman / WorkerW 的子窗口）
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class DeskProbe {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr p, IntPtr after, string c, string n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);

    public struct RECT { public int L,T,R,B; }

    public static List<string> Probe(uint targetPid) {
        var res = new List<string>();
        IntPtr progman = FindWindowW("Progman", null);
        res.Add("Progman hwnd = 0x" + progman.ToInt64().ToString("X"));

        // 1) Progman 直接子窗口里属于目标进程的
        ScanChildren(progman, "Progman", targetPid, res);

        // 2) 找 WorkerW：Progman 之后、承载 SHELLDLL_DefView 的兄弟窗口
        IntPtr prev = progman;
        for (int i = 0; i < 40; i++) {
            IntPtr sib = FindWindowExW(IntPtr.Zero, prev, null, null);
            if (sib == IntPtr.Zero) break;
            var cls = new StringBuilder(64); GetClassNameW(sib, cls, 64);
            IntPtr dv = FindWindowExW(sib, IntPtr.Zero, "SHELLDLL_DefView", null);
            if (dv != IntPtr.Zero && cls.ToString() == "WorkerW") {
                res.Add("WorkerW hwnd = 0x" + sib.ToInt64().ToString("X"));
                ScanChildren(sib, "WorkerW", targetPid, res);
            }
            prev = sib;
        }
        // 3) 全局兜底：直接扫所有 WorkerW
        IntPtr ww = IntPtr.Zero;
        while (true) {
            ww = FindWindowExW(IntPtr.Zero, ww, "WorkerW", null);
            if (ww == IntPtr.Zero) break;
            ScanChildren(ww, "WorkerW*", targetPid, res);
        }
        return res;
    }

    static void ScanChildren(IntPtr parent, string tag, uint targetPid, List<string> res) {
        IntPtr child = IntPtr.Zero;
        int n = 0;
        while (true) {
            child = FindWindowExW(parent, child, null, null);
            if (child == IntPtr.Zero) break;
            uint pid; GetWindowThreadProcessId(child, out pid);
            var cls = new StringBuilder(64); GetClassNameW(child, cls, 64);
            var tt = new StringBuilder(128); GetWindowTextW(child, tt, 128);
            RECT r; GetWindowRect(child, out r);
            bool mine = pid == targetPid;
            if (mine) {
                res.Add(string.Format("  [{0}] MINE hwnd=0x{1:X} cls={2} title='{3}' vis={4} rect=({5},{6})-({7},{8})",
                    tag, child.ToInt64(), cls, tt, IsWindowVisible(child), r.L, r.T, r.R, r.B));
            }
            if (++n > 60) break;
        }
    }
}
'@

$p = Get-Process -Name 'xhy' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未运行"; exit 1 }
"目标 pid = $($p.Id)"
[DeskProbe]::Probe([uint32]$p.Id) | ForEach-Object { $_ }
