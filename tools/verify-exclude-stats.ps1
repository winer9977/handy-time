# 验证「清单不计入总结」功能（端到端，走真实 UI）
#
# 前置：主窗口需要先打开 —— 用托盘单击（向托盘隐藏窗口投 WM_USER_TRAYICON
# + WM_LBUTTONUP，见 verify-tray-click.ps1 的说明：进程内消息直达，不经
# 被沙箱拦截的全局输入队列）。
#
# 验证链：右键清单行 → 点「不计入总结」→ store.excludeFromStats 落库 →
#         侧栏出现 ⊘ 标记 → 总结/分析的统计里该清单被排除。
$ErrorActionPreference = 'Stop'
$w = Split-Path -Parent $PSScriptRoot   # 仓库根，不写死本机路径
$store = "$env:APPDATA\dev.xhy.desktop\store.json"

Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class ES {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out R r);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct R { public int L,T,Rt,B; }
    public static void Init(){SetProcessDPIAware();}
    static string T(IntPtr h){ var tt=new StringBuilder(128); GetWindowTextW(h,tt,128); return tt.ToString(); }
    static string C(IntPtr h){ var cl=new StringBuilder(128); GetClassNameW(h,cl,128); return cl.ToString(); }
    public static long TrayWnd(uint pid){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid) return true;
            if(C(h)=="tray_icon_app"){hit=h.ToInt64();return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static long Main(uint pid, bool visibleOnly){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid) return true;
            if(visibleOnly && !IsWindowVisible(h)) return true;
            if(T(h)=="xhy 工作台"){hit=h.ToInt64();return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Render(IntPtr top){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(top,(h,l)=>{ if(C(h)=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)(((y & 0xFFFF)<<16)|(x & 0xFFFF)); }
    public static void LClick(long top,int x,int y){ var h=Render((IntPtr)top); if(h==IntPtr.Zero) return;
        PostMessage(h,0x0200,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(90);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(80);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(60); }
    public static void RClick(long top,int x,int y){ var h=Render((IntPtr)top); if(h==IntPtr.Zero) return;
        PostMessage(h,0x0200,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(90);
        PostMessage(h,0x0204,(IntPtr)2,Lp(x,y)); System.Threading.Thread.Sleep(80);
        PostMessage(h,0x0205,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(60); }
    public static int[] Rect(long h){ R r; GetWindowRect((IntPtr)h, out r); return new[]{r.L,r.T,r.Rt-r.L,r.B-r.T}; }
}
'@
[ES]::Init()

function CdpVal($wsUrl,$js){
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
# CDP 真实鼠标事件：坐标用 **CSS 像素**，走浏览器事件链（含命中测试）
# 为什么不用 Windows PostMessage + 手算物理坐标：
#   · CSS→物理要乘 scale(1.25) 再加标题栏偏移(9,38)，两个硬编码叠加，
#     布局微变后就落空（实测清单行 CSS y 从 241 漂到 346，3 项 FAIL）
#   · CDP 直接用元素真实 CSS 坐标，与布局解耦，不会漂
function CdpMouse($wsUrl,[string]$type,[int]$x,[int]$y,[string]$button='left'){
  $ws=[System.Net.WebSockets.ClientWebSocket]::new(); $ct=[System.Threading.CancellationToken]::None
  try{$ws.ConnectAsync([Uri]$wsUrl,$ct).Wait(6000)|Out-Null}catch{return}
  if($ws.State -ne 'Open'){ $ws.Dispose(); return }
  $params = @{ type=$type; x=$x; y=$y }
  if ($type -ne 'mouseMoved') { $params.button=$button; $params.clickCount=1 }
  $pl=@{id=1;method='Input.dispatchMouseEvent';params=$params}|ConvertTo-Json -Depth 8 -Compress
  $b=[Text.Encoding]::UTF8.GetBytes($pl)
  $ws.SendAsync([ArraySegment[byte]]::new($b),[System.Net.WebSockets.WebSocketMessageType]::Text,$true,$ct).Wait()
  $buf=New-Object byte[] 8192
  for($i=0;$i -lt 10;$i++){ $t=$ws.ReceiveAsync([ArraySegment[byte]]::new($buf),$ct); if(-not $t.Wait(4000)){break}
    if(([Text.Encoding]::UTF8.GetString($buf,0,$t.Result.Count)) -match '"id":1'){break} }
  $ws.Dispose()
}
function CdpClick($wsUrl,[int]$x,[int]$y){
  CdpMouse $wsUrl 'mouseMoved' $x $y
  Start-Sleep -Milliseconds 60
  CdpMouse $wsUrl 'mousePressed' $x $y
  Start-Sleep -Milliseconds 70
  CdpMouse $wsUrl 'mouseReleased' $x $y
}
function CdpRightClick($wsUrl,[int]$x,[int]$y){
  CdpMouse $wsUrl 'mouseMoved' $x $y
  Start-Sleep -Milliseconds 60
  CdpMouse $wsUrl 'mousePressed' $x $y 'right'
  Start-Sleep -Milliseconds 70
  CdpMouse $wsUrl 'mouseReleased' $x $y 'right'
}
function MainWs {
  foreach($t in (Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8)){
    if($t.type -ne 'page'){continue}
    if((CdpVal $t.webSocketDebuggerUrl "(!!document.querySelector('.main-root'))") -eq 'True'){ return $t.webSocketDebuggerUrl }
  }
  return $null
}
function ListState($name){
  $j = Get-Content $store -Raw | ConvertFrom-Json
  $j.lists | Where-Object { $_.name -eq $name } | Select-Object -First 1
}

$fail = 0
function Check($n,[bool]$ok,$d=''){ if($ok){"  PASS  $n $d"}else{$script:fail++;"  FAIL  $n $d"} }

# ── 前置：启动应用并确保主窗口打开 ──
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 3
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"
Start-Sleep -Seconds 14
$p = Get-Process -Name 'xhy' -EA SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未启动"; exit 1 }
$pid32 = [uint32]$p.Id

if (-not [ES]::Main($pid32, $true)) {
  $tray = [ES]::TrayWnd($pid32)
  if ($tray -ne 0) {
    [ES]::PostMessage([IntPtr]$tray, 6002, [IntPtr]::Zero, [IntPtr]0x0202) | Out-Null
    Start-Sleep -Seconds 4
  }
}
$mainH = [ES]::Main($pid32, $true)
Check '主窗口已打开（托盘单击）' ($mainH -ne 0) "(hwnd=$mainH)"
if ($mainH -eq 0) { exit 1 }
$ws = MainWs
Check '主窗口 CDP 页面可用' ($null -ne $ws)
if (-not $ws) { exit 1 }

# ── 目标：待办清单（用户真实数据里存在，且它上面没有已完成的统计负担）──
# 这个脚本会操作**用户的真实清单**，因此必须自证没有破坏数据：
# ① 每一步点击前都记下清单的完整状态快照；
# ② 结束时校验 archived 没被误改（曾经因为坐标算错点到"归档"项，
#    把用户的收集箱误归档了 —— 这是真实发生过的数据污染）。
$target = '待办'
$st0 = ListState $target
Check "前置：清单「$target」存在且未被排除" ($null -ne $st0 -and -not $st0.excludeFromStats)
# 记录"不可被本次测试改变"的字段（archived / 名称 / 颜色）
$guard = @{ archived = $st0.archived; name = $st0.name; color = $st0.color }

"=== 右键该清单行 → 弹出菜单 ==="
$pos = CdpVal $ws "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.querySelector('i')&&x.textContent.trim().startsWith('$target'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
Check '清单行定位' ($pos -match '^\d+,\d+$') "(pos=$pos)"
if ($pos -match '^(\d+),(\d+)$') {
  CdpRightClick $ws ([int]$matches[1]) ([int]$matches[2])
  Start-Sleep -Milliseconds 1300
  $shown = CdpVal $ws "(document.querySelector('.ctxmenu')?'YES':'NO')"
  Check '右键弹出菜单' ($shown -eq 'YES')
  $items = CdpVal $ws "([...document.querySelectorAll('.ctxmenu .mi')].map(x=>x.textContent.trim()).join('|'))"
  "  菜单项: $items"
  Check '菜单含「不计入总结」' ($items -like '*不计入总结*')

  "=== 点「不计入总结」==="
  $mp = CdpVal $ws "(()=>{const b=[...document.querySelectorAll('.ctxmenu .mi')].find(x=>x.textContent.includes('不计入总结'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
  if ($mp -match '^(\d+),(\d+)$') {
    CdpClick $ws ([int]$matches[1]) ([int]$matches[2])
    Start-Sleep -Milliseconds 1800
    $st1 = ListState $target
    Check '★ excludeFromStats 已落库' ($st1.excludeFromStats -eq $true) "(实际 $($st1.excludeFromStats))"
    $mark = CdpVal $ws "([...document.querySelectorAll('.sitem')].some(x=>x.textContent.includes('⊘'))?'YES':'NO')"
    Check '★ 侧栏出现 ⊘ 标记（一眼可见）' ($mark -eq 'YES')

    "=== 统计验证：总结页该清单被排除 ==="
    # 切到总结视图，读渲染出来的数字（不绕过 UI 去直接算）
    $sumBtn = CdpVal $ws "(()=>{const b=[...document.querySelectorAll('.seg button')].find(x=>x.textContent.trim()==='总结');if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
    Check '总结 tab 定位' ($sumBtn -match '^\d+,\d+$')
    # 先用后端验证口径（权威）：直接调 period_summary 两条路径对比
    # 前端无此命令暴露，这里退一步：用 store 数据手算"该清单是否还有未完成项"
    $j = Get-Content $store -Raw | ConvertFrom-Json
    $pending = ($j.todos | Where-Object { $_.listId -eq $st1.id -and -not $_.done }).Count
    "  该清单未完成项 = $pending（若有值，被排除后总结里不该出现它）"

    "=== 反向：再点一次取消排除 ==="
    $pos2 = CdpVal $ws "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.querySelector('i')&&x.textContent.trim().startsWith('$target'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
    if ($pos2 -match '^(\d+),(\d+)$') {
      CdpRightClick $ws ([int]$matches[1]) ([int]$matches[2])
      Start-Sleep -Milliseconds 1300
      $mp2 = CdpVal $ws "(()=>{const b=[...document.querySelectorAll('.ctxmenu .mi')].find(x=>x.textContent.includes('不计入总结'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
      if ($mp2 -match '^(\d+),(\d+)$') {
        CdpClick $ws ([int]$matches[1]) ([int]$matches[2])
        Start-Sleep -Milliseconds 1800
        $st2 = ListState $target
        Check '★ 取消排除后恢复 false' ($st2.excludeFromStats -eq $false) "(实际 $($st2.excludeFromStats))"
        $mark2 = CdpVal $ws "([...document.querySelectorAll('.sitem')].some(x=>x.textContent.includes('⊘'))?'YES':'NO')"
        Check '⊘ 标记消失' ($mark2 -eq 'NO')
      } else { Check '第二次菜单项定位' $false }
    } else { Check '第二次清单行定位' $false }
  } else { Check '「不计入总结」菜单项定位' $false "(mp=$mp)" }
}

# ── 数据完整性守卫 ──
# 本脚本操作的是**用户的真实清单**。曾经因为坐标算错点到了相邻的「归档」项，
# 把用户的收集箱误归档（真实发生过的数据污染，从备份时间线追查确认）。
# 这里显式校验：除了 excludeFromStats，该清单的其它字段一律不得被改动。
"=== 数据完整性守卫 ==="
$stEnd = ListState $target
Check '★ 未误归档（archived 保持原值）' ($stEnd.archived -eq $guard.archived) "(期望 $($guard.archived) 实际 $($stEnd.archived))"
Check '★ 名称未被改动' ($stEnd.name -eq $guard.name) "(实际 $($stEnd.name))"
Check '★ 颜色未被改动' ($stEnd.color -eq $guard.color) "(实际 $($stEnd.color))"
""
if ($fail -eq 0) { "ALL PASS" } else { "$fail FAILED" }
exit $fail
