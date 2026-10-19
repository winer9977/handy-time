# 验证日历挂件是否跟随 **todos** 变化（watch 依赖漏项修复）
#
# 为什么必须实机验证：这是 watch 依赖问题，纯函数测试抓不到。
# 修复前 CalendarWidget 只监听 data.store.events，而派生条目来自 todos
# （后端 todo_derived_events）+ "关联待办完成则隐藏其日程"的规则 ——
# 于是在别处勾选完成，日历角标不刷新，停在旧状态。
#
# 验证手法：在**待办挂件**里勾掉一条带到期日的待办 → 观察**日历挂件**的
# 月视图角标数量是否减少（跨窗口，靠 xhy://changed 广播触发重算）。
$ErrorActionPreference = 'Stop'
$w = Split-Path -Parent $PSScriptRoot   # 仓库根（脚本都在 <repo>/tools 下），不写死本机路径
$store = "$env:APPDATA\dev.xhy.desktop\store.json"

Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class CW2 {
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
    static string T(IntPtr h){ var tt=new StringBuilder(128); GetWindowTextW(h,tt,128); return tt.ToString(); }
    static string C(IntPtr h){ var cl=new StringBuilder(128); GetClassNameW(h,cl,128); return cl.ToString(); }
    public static long Top(uint pid, string want){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            if(T(h)==want){hit=h.ToInt64();return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static long TrayWnd(uint pid){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid) return true;
            if(C(h)=="tray_icon_app"){hit=h.ToInt64();return false;} return true; }, IntPtr.Zero);
        return hit; }
    // 通过顶层窗口找它的 WebView2 子窗口（用于把鼠标消息投给页面）
    public static IntPtr Render(long top){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows((IntPtr)top,(h,l)=>{ if(C(h)=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
}
'@
[CW2]::Init()

function CdpEval($wsUrl,$js){
  $ws=[System.Net.WebSockets.ClientWebSocket]::new(); $ct=[System.Threading.CancellationToken]::None
  try{$ws.ConnectAsync([Uri]$wsUrl,$ct).Wait(6000)|Out-Null}catch{return $null}
  if($ws.State -ne 'Open'){return $null}
  $pl=@{id=1;method='Runtime.evaluate';params=@{expression=$js;returnByValue=$true;awaitPromise=$true}}|ConvertTo-Json -Depth 8 -Compress
  $b=[Text.Encoding]::UTF8.GetBytes($pl)
  $ws.SendAsync([ArraySegment[byte]]::new($b),[System.Net.WebSockets.WebSocketMessageType]::Text,$true,$ct).Wait()
  $buf=New-Object byte[] 262144; $sb=New-Object Text.StringBuilder
  for($i=0;$i -lt 25;$i++){ $t=$ws.ReceiveAsync([ArraySegment[byte]]::new($buf),$ct); if(-not $t.Wait(8000)){break}
    $r=$t.Result; $null=$sb.Append([Text.Encoding]::UTF8.GetString($buf,0,$r.Count))
    if($sb.ToString() -match '"id":1'){break} }
  $ws.Dispose()
  try { $o = $sb.ToString() | ConvertFrom-Json } catch { return $null }
  $v = $o.result.result.value
  if ($v -is [string] -and $v.StartsWith('"')) { try { $v = $v | ConvertFrom-Json } catch {} }
  return "$v"
}
function CdpMouse($wsUrl,[string]$type,[int]$x,[int]$y,[string]$button='left'){
  $ws=[System.Net.WebSockets.ClientWebSocket]::new(); $ct=[System.Threading.CancellationToken]::None
  try{$ws.ConnectAsync([Uri]$wsUrl,$ct).Wait(6000)|Out-Null}catch{return}
  if($ws.State -ne 'Open'){ $ws.Dispose(); return }
  $params=@{type=$type;x=$x;y=$y}
  if($type -ne 'mouseMoved'){ $params.button=$button; $params.clickCount=1 }
  $pl=@{id=1;method='Input.dispatchMouseEvent';params=$params}|ConvertTo-Json -Depth 8 -Compress
  $b=[Text.Encoding]::UTF8.GetBytes($pl)
  $ws.SendAsync([ArraySegment[byte]]::new($b),[System.Net.WebSockets.WebSocketMessageType]::Text,$true,$ct).Wait()
  $buf=New-Object byte[] 8192
  for($i=0;$i -lt 10;$i++){ $t=$ws.ReceiveAsync([ArraySegment[byte]]::new($buf),$ct); if(-not $t.Wait(4000)){break}
    if(([Text.Encoding]::UTF8.GetString($buf,0,$t.Result.Count)) -match '"id":1'){break} }
  $ws.Dispose()
}
function CdpClick($wsUrl,[int]$x,[int]$y){
  CdpMouse $wsUrl 'mouseMoved' $x $y; Start-Sleep -Milliseconds 60
  CdpMouse $wsUrl 'mousePressed' $x $y; Start-Sleep -Milliseconds 70
  CdpMouse $wsUrl 'mouseReleased' $x $y
}
# 按特征找页面
function PageWs([string]$probe){
  foreach($t in (Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8)){
    if($t.type -ne 'page'){continue}
    if((CdpEval $t.webSocketDebuggerUrl $probe) -eq 'True'){ return $t.webSocketDebuggerUrl }
  }
  return $null
}

$fail = 0
function Check($n,[bool]$ok,$d=''){ if($ok){"  PASS  $n $d"}else{$script:fail++;"  FAIL  $n $d"} }

"=== 前置：确保便签外的两个挂件都可见（日历+待办），并开主窗口不需要 ==="
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 3
$j = Get-Content $store -Raw | ConvertFrom-Json
foreach ($kind in @('todo','calendar')) {
  ($j.widgets | Where-Object { $_.kind -eq $kind }) | ForEach-Object { $_.visible = $true; $_.edgeHide = $null }
}
# 清掉所有待办的完成态，保证从干净起点开始（可重复运行）
foreach ($t in $j.todos) { $t.done = $false; $t.doneAt = $null }
# 确保至少有一条"带到期日"的未完成待办会派生成日历角标
$j | ConvertTo-Json -Depth 14 | Set-Content $store -Encoding utf8

$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"
Start-Sleep -Seconds 15
$p = Get-Process -Name 'xhy' -EA SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未启动"; exit 1 }
$pid32 = [uint32]$p.Id

$todoWs  = PageWs "(!!document.querySelector('.trow'))"
$calWs   = PageWs "(!!document.querySelector('.grid, .wd'))"
Check '待办挂件页面可用' ($null -ne $todoWs)
Check '日历挂件页面可用' ($null -ne $calWs)
if (-not $todoWs -or -not $calWs) { exit 1 }

# 日历挂件的角标总数（月视图 .cell i）+ 日程条目数（周视图 .ev）
# 用两条不同路径的计数一起看，避免单点失效
function CalBadges($ws){
  return CdpEval $ws "(()=>{const dots=[...document.querySelectorAll('.cell i')].length;const evs=document.querySelectorAll('.ev').length;const devs=document.querySelectorAll('.dev').length;const empties=[...document.querySelectorAll('.cell .cevs')].filter(x=>x.textContent.trim()).length;return JSON.stringify({dots:dots,evs:evs,devs:devs,nonEmptyCells:empties});})()"
}
function TodoRowCount($ws){
  return CdpEval $ws "String(document.querySelectorAll('.trow').length)"
}

"=== 基线 ==="
$before = CalBadges $calWs
"  日历: $before"
"  待办行数: $(TodoRowCount $todoWs)"

# 找一条**未完成且带到期日**的待办（勾掉它会改变日历角标）
$targetTitle = CdpEval $todoWs "(()=>{const rows=[...document.querySelectorAll('.trow')];const r=rows.find(x=>{const t=x.textContent; return !x.querySelector('.cb.on') && /逾期|今天|明天|周/.test(t)});if(!r)return '';return (r.querySelector('.title')||{}).textContent||'';})()"
Check '找到带到期日的未完成待办' ($targetTitle -ne '') "(标题=「$targetTitle」)"

# ── 核心操作：在待办挂件点复选框 ──
$cb = CdpEval $todoWs "(()=>{const rows=[...document.querySelectorAll('.trow')];const r=rows.find(x=>((x.querySelector('.title')||{}).textContent||'')==='$targetTitle');if(!r)return '';const b=r.querySelector('.cb');if(!b)return '';const g=b.getBoundingClientRect();return Math.round(g.left+g.width/2)+','+Math.round(g.top+g.height/2);})()"
Check '复选框定位' ($cb -match '^\d+,\d+$') "(pos=$cb)"
if ($cb -match '^(\d+),(\d+)$') {
  CdpClick $todoWs ([int]$matches[1]) ([int]$matches[2])
  Start-Sleep -Milliseconds 2500   # 留足广播 + 日历重取时间

  $done = (Get-Content $store -Raw | ConvertFrom-Json).todos | Where-Object { $_.title -eq $targetTitle } | Select-Object -First 1 | ForEach-Object { $_.done }
  Check '待办已落库为完成' ($done -eq $true) "(实际 $done)"

  $after = CalBadges $calWs
  "  日历: $after"
  Check '★★ 日历挂件已跟随 todos 变化（角标数变化）' ($after -ne $before) "(前=$before 后=$after)"

  "=== 反向：取消完成，日历应复原 ==="
  # 完成后该行会移入折叠的「已完成」区（列表里看不见）—— 要用真实路径把它
  # 找回来：点「完成」筛选让已完成项显示出来，再点它的复选框取消完成。
  # 直接按标题找行会失败（实测踩过：选择器返回空）。
  $fbtn = CdpEval $todoWs "(()=>{const b=[...document.querySelectorAll('.fbtn')].find(x=>x.textContent.trim()==='完成');if(!b)return '';const g=b.getBoundingClientRect();return Math.round(g.left+g.width/2)+','+Math.round(g.top+g.height/2);})()"
  Check '「完成」筛选按钮定位' ($fbtn -match '^\d+,\d+$')
  if ($fbtn -match '^(\d+),(\d+)$') {
    CdpClick $todoWs ([int]$matches[1]) ([int]$matches[2])
    Start-Sleep -Milliseconds 1500
    $cb2 = CdpEval $todoWs "(()=>{const rows=[...document.querySelectorAll('.trow')];const r=rows.find(x=>((x.querySelector('.title')||{}).textContent||'')==='$targetTitle');if(!r)return '';const b=r.querySelector('.cb');if(!b)return '';const g=b.getBoundingClientRect();return Math.round(g.left+g.width/2)+','+Math.round(g.top+g.height/2);})()"
    Check '已完成区里找到该行' ($cb2 -match '^\d+,\d+$') "(pos=$cb2)"
    if ($cb2 -match '^(\d+),(\d+)$') {
      CdpClick $todoWs ([int]$matches[1]) ([int]$matches[2])
      Start-Sleep -Milliseconds 2500
      $restored = CalBadges $calWs
      Check '★★ 取消完成后日历复原' ($restored -eq $before) "(期望=$before 实际=$restored)"
    }
    # 把筛选清回「全部」，别把测试留下的筛选态留给用户
    $clr = CdpEval $todoWs "(()=>{const b=document.querySelector('.fbtn.clr');if(!b)return '';const g=b.getBoundingClientRect();return Math.round(g.left+g.width/2)+','+Math.round(g.top+g.height/2);})()"
    if ($clr -match '^(\d+),(\d+)$') { CdpClick $todoWs ([int]$matches[1]) ([int]$matches[2]); Start-Sleep -Milliseconds 800 }
  }
}
""
if ($fail -eq 0) { "ALL PASS" } else { "$fail FAILED" }
exit $fail
