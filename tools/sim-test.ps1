# 真实输入仿真测试：PostMessage WM_LBUTTONDOWN/UP 到 WebView2 子窗口
# 与 CDP .click() 的本质区别：走浏览器命中测试与 pointerdown→mousedown→click
# 全链，等价真实鼠标 —— 上次交付没做这层，把"主线程死锁"放了出去。
#
# 用法：先以 CDP 端口 + XHY_UI_MAIN 启动 xhy，再跑本脚本。
param([switch]$Quick)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class SIM {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct R { public int L,T,Rt,B; }
    public static void Init(){SetProcessDPIAware();}
    static string T(IntPtr h){ var tt=new StringBuilder(64); GetWindowTextW(h,tt,64); return tt.ToString(); }
    // 顶层窗口 hwnd（按标题），0=没找到
    public static long Top(uint pid, string want){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            if(T(h)==want){hit=h.ToInt64(); return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static bool Vis(long hwnd){ return hwnd!=0 && IsWindowVisible((IntPtr)hwnd); }
    public static string Title(long hwnd){ return T((IntPtr)hwnd); }
    static IntPtr Render(IntPtr top){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(top,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)(((y & 0xFFFF)<<16)|(x & 0xFFFF)); }
    // 真实点击：MOVE → DOWN → UP（物理像素，子窗口客户区坐标）
    public static void Click(long top, int x, int y){
        var h = Render((IntPtr)top);
        if (h == IntPtr.Zero) return;
        PostMessage(h, 0x0200, IntPtr.Zero, Lp(x,y));          // WM_MOUSEMOVE
        System.Threading.Thread.Sleep(90);
        PostMessage(h, 0x0201, (IntPtr)1, Lp(x,y));            // WM_LBUTTONDOWN MK_LBUTTON
        System.Threading.Thread.Sleep(110);
        PostMessage(h, 0x0202, IntPtr.Zero, Lp(x,y));          // WM_LBUTTONUP
    }
}
'@
[SIM]::Init()

# ---------- CDP 工具 ----------
function Cdp-Targets { Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8 }
function Cdp-Eval([string]$wsUrl, [string]$js) {
  $ws = [System.Net.WebSockets.ClientWebSocket]::new()
  $ct = [System.Threading.CancellationToken]::None
  try { $ws.ConnectAsync([Uri]$wsUrl, $ct).Wait(6000) | Out-Null } catch { return $null }
  if ($ws.State -ne 'Open') { $ws.Dispose(); return $null }
  $payload = @{ id=1; method='Runtime.evaluate'; params=@{ expression=$js; returnByValue=$true; awaitPromise=$true } } | ConvertTo-Json -Depth 8 -Compress
  $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
  $ws.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait()
  $buf = New-Object byte[] 524288
  $sb = New-Object Text.StringBuilder
  for ($i=0; $i -lt 20; $i++) {
    $task = $ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $ct)
    if (-not $task.Wait(6000)) { break }
    $r = $task.Result
    $null = $sb.Append([Text.Encoding]::UTF8.GetString($buf, 0, $r.Count))
    if ($sb.ToString() -match '"id":1') { break }
  }
  $ws.Dispose()
  $s = $sb.ToString()
  if ($s -match '"value":"((?:[^"\\]|\\.)*)"') { return ($matches[1] -replace '\\u([0-9a-fA-F]{4})', { param($m) [char][Convert]::ToInt32($m.Groups[1].Value,16) } -replace '\\"','"' -replace '\\\\','\') }
  if ($s -match '"value":(true|false|null|[0-9.]+)') { return $matches[1] }
  return $null
}
# 找页面：JS 探针返回 true 的那个
function Cdp-Page([string]$probeJs) {
  foreach ($t in (Cdp-Targets)) {
    if ($t.type -ne 'page') { continue }
    $v = Cdp-Eval $t.webSocketDebuggerUrl $probeJs
    if ($v -eq 'true') { return $t }
  }
  return $null
}
# 元素中心 CSS 坐标（按 .xh-btn 文本精确匹配）
function El-Css([string]$wsUrl, [string]$text, [string]$sel = '.xh-btn') {
  $js = "JSON.stringify((()=>{const b=[...document.querySelectorAll('$sel')].find(x=>x.textContent.trim()==='$text');if(!b)return null;const r=b.getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2};})())"
  $v = Cdp-Eval $wsUrl $js
  if (-not $v -or $v -eq 'null') { return $null }
  try { return ($v | ConvertFrom-Json) } catch { return $null }
}

$SCALE = 1.25   # 125% 显示缩放（实测 AppliedDPI=120）
$store = "$env:APPDATA\dev.xhy.desktop\store.json"
function Store-Get { Get-Content $store -Raw | ConvertFrom-Json }
function Widget-Vis($j, $kind) { ($j.widgets | Where-Object { $_.kind -eq $kind }).visible }
function Widget-View($j) { ($j.widgets | Where-Object { $_.kind -eq 'calendar' }).view }

$p = Get-Process -Name 'xhy' | Select-Object -First 1
$pid_ = [uint32]$p.Id
$fails = 0
function Check([string]$name, [bool]$ok, [string]$detail = '') {
  if ($ok) { "  PASS  $name $detail" }
  else { $script:fails++; "  FAIL  $name $detail" }
}

"=== 真实输入仿真测试（PostMessage 鼠标消息）==="

# ---------- 定位三个窗口 ----------
$calTop = [SIM]::Top($pid_, '日历清单')
$todoTop = [SIM]::Top($pid_, '待办清单')
$mainTop = [SIM]::Top($pid_, 'xhy 工作台')
Check '窗口存在（日历/待办/工作台）' ($calTop -ne 0 -and $todoTop -ne 0 -and $mainTop -ne 0) "cal=$calTop todo=$todoTop main=$mainTop"
if ($calTop -eq 0 -or $mainTop -eq 0) { "前置窗口不齐，终止"; exit 1 }

$calPage = Cdp-Page "(!!document.querySelector('.xh-btn')&&[...document.querySelectorAll('.xh-btn')].some(b=>b.textContent.trim()==='月'))"
$mainPage = Cdp-Page "(!!document.querySelector('.main-root'))"
Check 'CDP 页面定位（日历挂件/主窗口）' ($null -ne $calPage -and $null -ne $mainPage)

# ---------- A. 日历挂件：真实点击 月/周/日 ----------
# 先置为 day（store 直写 + 等 sync 生效），再真实点「月」
foreach ($pair in @(@('月','month'), @('周','week'), @('日','day'))) {
  $btn = $pair[0]; $want = $pair[1]
  $pos = El-Css $calPage.webSocketDebuggerUrl $btn
  if (-not $pos) { Check "日历按钮「$btn」定位" $false; continue }
  $px = [int]($pos.x * $SCALE); $py = [int]($pos.y * $SCALE)
  [SIM]::Click($calTop, $px, $py)
  Start-Sleep -Milliseconds 1600
  $j = Store-Get
  $got = Widget-View $j
  Check "真实点击日历「$btn」→ store.view=$want" ($got -eq $want) "(实际 $got)"
}

# ---------- B. 主窗口侧栏：真实点击收起/显示 待办 ----------
# 找侧栏里"待办清单"那一行
$sbPos = Cdp-Eval $mainPage.webSocketDebuggerUrl "JSON.stringify((()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.textContent.trim().startsWith('待办清单'));if(!b)return null;const r=b.getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2};})())"
if (-not $sbPos -or $sbPos -eq 'null') { Check '侧栏「待办清单」定位' $false } else {
  $o = $sbPos | ConvertFrom-Json
  $px = [int]($o.x * $SCALE); $py = [int]($o.y * $SCALE)
  # 当前应为显示 → 点一下收起
  $before = Widget-Vis (Store-Get) 'todo'
  [SIM]::Click($mainTop, $px, $py)
  Start-Sleep -Milliseconds 1800
  $after1 = Widget-Vis (Store-Get) 'todo'
  Check '真实点击侧栏：待办 visible 翻转' ($before -ne $after1) "($before → $after1)"
  $vis1 = [SIM]::Vis($todoTop)
  Check '收起后待办窗口不可见' ($vis1 -eq $false -or $after1 -eq $true) "(vis=$vis1 store=$after1)"
  # 再点一下恢复
  [SIM]::Click($mainTop, $px, $py)
  Start-Sleep -Milliseconds 2000
  $after2 = Widget-Vis (Store-Get) 'todo'
  $vis2 = [SIM]::Vis($todoTop)
  Check '真实点击侧栏：待办恢复显示' ($after2 -eq $true -and $vis2 -eq $true) "(store=$after2 vis=$vis2)"
}

# ---------- C. 死锁回归核心：无窗口时点显示（build 路径） ----------
# 用户场景：重启后待办是收起态（没有窗口），点侧栏"显示" → 必须建窗且不卡死
$j = Store-Get
$tw = $j.widgets | Where-Object { $_.kind -eq 'todo' }
$tw.visible = $false
$j | ConvertTo-Json -Depth 14 | Set-Content $store -Encoding utf8
Get-Process -Name 'xhy' | Stop-Process -Force; Start-Sleep 2
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'
$env:XHY_UI_MAIN='1'
Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"; Start-Sleep 15
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$pid_ = [uint32]$p.Id
$mainTop = [SIM]::Top($pid_, 'xhy 工作台')
$todoTop = [SIM]::Top($pid_, '待办清单')   # 应为 0：visible=false 不建窗
Check '重启后待办无窗口（前置）' ($todoTop -eq 0)
$mainPage = Cdp-Page "(!!document.querySelector('.main-root'))"
$sbPos = Cdp-Eval $mainPage.webSocketDebuggerUrl "JSON.stringify((()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.textContent.trim().startsWith('待办清单'));if(!b)return null;const r=b.getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2};})())"
$o = $sbPos | ConvertFrom-Json
$px = [int]($o.x * $SCALE); $py = [int]($o.y * $SCALE)
[SIM]::Click($mainTop, $px, $py)          # ← 这一击在旧版必死锁（主线程 build）
Start-Sleep -Milliseconds 3000
$todoTop2 = [SIM]::Top($pid_, '待办清单')
$vis = [SIM]::Vis($todoTop2)
Check '★ 无窗口时点显示：窗口被创建且可见（旧版死锁点）' ($todoTop2 -ne 0 -and $vis) "(hwnd=$todoTop2 vis=$vis)"
# 死锁判据：应用还能响应 CDP + 还能再点一次日历按钮
$alive = Cdp-Eval $mainPage.webSocketDebuggerUrl "'ALIVE'"
Check '★ 死锁回归：应用仍响应' ($alive -eq 'ALIVE')
$calPage = Cdp-Page "(!!document.querySelector('.xh-btn')&&[...document.querySelectorAll('.xh-btn')].some(b=>b.textContent.trim()==='月'))"
$calTop = [SIM]::Top($pid_, '日历清单')
$pos = El-Css $calPage.webSocketDebuggerUrl '月'
if ($pos) {
  [SIM]::Click($calTop, [int]($pos.x*$SCALE), [int]($pos.y*$SCALE))
  Start-Sleep -Milliseconds 1600
  Check '★ 死锁回归：日历按钮仍可切换' ((Widget-View (Store-Get)) -eq 'month')
}

""
if ($fails -eq 0) { "═══ 全部通过（$fails 失败）═══" } else { "═══ $fails 项失败 ═══"; exit 1 }
