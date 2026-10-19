# 拖拽链路诊断：合成 pointer 事件起拖 + 真实光标定位到日历某格
#
# 原理：前端 armDrag 只依赖 DOM pointer 事件（可用 PostMessage 伪造），
# 而 Rust poller 用 GetAsyncKeyState 读真实按键 —— 没人真按着，
# 所以它会在 drag_begin 后**立刻**判定"已抬起"并 finish，
# 落点 = 当前真实光标位置（用 SetCursorPos 精确放到日历某格）。
# 若 events 表新增一条，说明「起拖 → 广播 → 坐标换算 → 命中 → 落库」全通；
# 若没新增，则断点在这条链上，可逐环排查。
param(
  [int]$TodoX = 1508, [int]$TodoY = 30,
  [switch]$Seed
)
# 全新首启时 todos 为空，没东西可拖 —— armDrag 根本不会触发，
# 测出来是"假失败"。-Seed 会停应用、播一条待办、再启动。
if ($Seed) {
  $st = "$env:APPDATA\dev.xhy.desktop\store.json"
  Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
  Start-Sleep 2
  $o = Get-Content $st -Raw | ConvertFrom-Json
  if (($o.todos | Measure-Object).Count -eq 0) {
    $o.todos = @([pscustomobject]@{ id='dragseed'; listId=$o.lists[0].id; title='拖我进日历'
      note=''; done=$false; doneAt=$null; pinned=$false; order=1; tags=@()
      due=$null; remindAt=$null; reminded=$false; countdown=$null; recurrence=$null
      createdAt='2026-10-16T22:00'; archivedAt=$null })
    $o | ConvertTo-Json -Depth 12 | Set-Content $st -Encoding utf8
    "已播种待办 dragseed"
  } else { "已有 $($o.todos.Count) 条待办，跳过播种" }
  Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"
  Start-Sleep 13
}
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class DG {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] static extern short GetAsyncKeyState(int vk);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }
    public struct Win { public IntPtr H; public string Title; public int L,T,Wd,Ht; }
    public static void Aware(){SetProcessDPIAware();}
    public static System.Collections.Generic.List<Win> Find(uint pid){
        var res=new System.Collections.Generic.List<Win>();
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p);
            if(p!=pid||!IsWindowVisible(h)) return true;
            RC r; GetWindowRect(h,out r); if((r.R-r.L)<40) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            res.Add(new Win{H=h,Title=tt.ToString(),L=r.L,T=r.T,Wd=r.R-r.L,Ht=r.B-r.T});
            return true; }, IntPtr.Zero);
        return res; }
    public static IntPtr Render(IntPtr parent){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(parent,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)((y<<16)|(x&0xFFFF)); }
    /** 伪造一次 pointerdown + 多次 pointermove（超 6px 阈值），不发 up */
    public static void FakeDrag(IntPtr h,int x,int y){
        PostMessage(h,0x0200,(IntPtr)0,Lp(x,y));                 // WM_MOUSEMOVE 定位
        System.Threading.Thread.Sleep(60);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y));                 // WM_LBUTTONDOWN
        System.Threading.Thread.Sleep(120);
        for (int i=1;i<=6;i++){
            PostMessage(h,0x0200,(IntPtr)1,Lp(x+i*8,y+i*6));      // WM_MOUSEMOVE 带下键
            System.Threading.Thread.Sleep(60);
        }
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x+48,y+36));          // WM_LBUTTONUP
    }
    public static bool LButtonDown(){ return (GetAsyncKeyState(0x01) & 0x8000) != 0; }
}
'@
[DG]::Aware()
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$pid32 = [uint32]$p.Id
$store = "$env:APPDATA\dev.xhy.desktop\store.json"
$before = ((Get-Content $store -Raw | ConvertFrom-Json).events | Measure-Object).Count
"起拖前 events = $before"
"真实 LBUTTON 当前按下? " + [DG]::LButtonDown()

$wins = [DG]::Find($pid32)
$todo = $wins | Where-Object { $_.Title -match '待办' } | Select-Object -First 1
$cal  = $wins | Where-Object { $_.Title -match '日历' } | Select-Object -First 1
if (-not $todo -or -not $cal) { "缺窗口"; exit 1 }
"待办 @($($todo.L),$($todo.T))   日历 @($($cal.L),$($cal.T)) $($cal.Wd)x$($cal.Ht)"

# 落点：日历月视图中间一格（CSS 约 (155,168) → 物理 = 窗口原点 + *1.25）
$cellX = $cal.L + [int]($cal.Wd * 0.50)
$cellY = $cal.T + [int]($cal.Ht * 0.42)
"落点目标（日历内物理坐标）= ($cellX,$cellY)"

# 关键：光标必须先落在日历格上，再在待办**首行**上起拖。
# 首行中心 CSS≈(150,85) → 物理 *1.25 = (188,106)。
# 用 (150,120) 会落在行的空白区，armDrag 根本不触发（踩过这个坑）。
[void][DG]::SetCursorPos($cellX, $cellY)
Start-Sleep -Milliseconds 250
$rt = [DG]::Render($todo.H)
if ($rt -eq [IntPtr]::Zero) { "待办无 webview 子窗"; exit 1 }
"在待办窗口内伪造拖拽（起点物理 188,106 = 首行中心）"
$lg = "$env:APPDATA\dev.xhy.desktop\xhy.log"
$mark = (Get-Content $lg -EA SilentlyContinue | Measure-Object).Count
[DG]::FakeDrag($rt, 188, 106)
Start-Sleep -Seconds 3
$after = ((Get-Content $store -Raw | ConvertFrom-Json).events | Measure-Object).Count
"起拖后 events = $after"
"`n=== 本次新增日志 ==="
Get-Content $lg -EA SilentlyContinue | Select-Object -Skip $mark | ForEach-Object { "  " + $_ }
if ($after -gt $before) {
  "`n✅ 拖拽链路全通：新增日程"
  (Get-Content $store -Raw | ConvertFrom-Json).events | ForEach-Object { "   title='$($_.title)' date=$($_.date) source=$($_.source)" }
} else {
  "`n❌ 未生成日程 —— 按上面日志定位断环"
}
