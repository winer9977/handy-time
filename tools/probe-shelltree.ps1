# 枚举顶层 Progman / WorkerW，以及各自是否承载 SHELLDLL_DefView
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public class ShellTree {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowExW(IntPtr p, IntPtr after, string c, string n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);

    public static List<string> Run() {
        var res = new List<string>();
        // 所有顶层 Progman / WorkerW
        IntPtr top = IntPtr.Zero;
        while (true) {
            top = FindWindowExW(IntPtr.Zero, top, null, null);
            if (top == IntPtr.Zero) break;
            var cls = new StringBuilder(64); GetClassNameW(top, cls, 64);
            string c = cls.ToString();
            if (c != "Progman" && c != "WorkerW") continue;
            uint pid; GetWindowThreadProcessId(top, out pid);
            // 直接子窗口清单
            var kids = new List<string>();
            IntPtr ch = IntPtr.Zero;
            int guard = 0;
            while (guard++ < 30) {
                ch = FindWindowExW(top, ch, null, null);
                if (ch == IntPtr.Zero) break;
                var kc = new StringBuilder(64); GetClassNameW(ch, kc, 64);
                uint kpid; GetWindowThreadProcessId(ch, out kpid);
                kids.Add(string.Format("{0}(pid={1},vis={2})", kc, kpid, IsWindowVisible(ch)));
            }
            IntPtr dv = FindWindowExW(top, IntPtr.Zero, "SHELLDLL_DefView", null);
            res.Add(string.Format("{0} hwnd=0x{1:X} pid={2} vis={3} hasDefView={4}",
                c, top.ToInt64(), pid, IsWindowVisible(top), dv != IntPtr.Zero));
            foreach (var k in kids) res.Add("    child: " + k);
        }
        return res;
    }
}
'@
[ShellTree]::Run() | ForEach-Object { $_ }
