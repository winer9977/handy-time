# 复现并验证：**在总结页**右键清单勾选「不计入总结」→ 总结数字应立刻变化
#
# 用户报的 bug 原话："当我在总结页面时，右键选择了某个待办清单不计入总结，
# 总结的内容应该变化，目前没有。"
# 根因：SummaryPanel 的 watch 依赖是 [kind, props.listId]，而"不计入总结"
# 改的是 store.lists[].excludeFromStats —— 两者都不变，所以不重新计算。
#
# 这个脚本必须**真的停在总结页**做操作，并读页面上渲染出的数字
# （不是只查 store，否则测不到"UI 没反应"这个症状本身）。
$ErrorActionPreference = 'Stop'
$w = Split-Path -Parent $PSScriptRoot   # 仓库根，不写死本机路径
$store = "$env:APPDATA\dev.xhy.desktop\store.json"

Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class SW {
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
    static string C(IntPtr h){ var cl=new StringBuilder(128); GetClassNameW(h,cl,128); return cl.ToString(); }
    static string T(IntPtr h){ var tt=new StringBuilder(128); GetWindowTextW(h,tt,128); return tt.ToString(); }
    public static long TrayWnd(uint pid){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid) return true;
            if(C(h)=="tray_icon_app"){hit=h.ToInt64();return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static long Main(uint pid, bool visOnly){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid) return true;
            if(visOnly && !IsWindowVisible(h)) return true;
            if(T(h)=="xhy 工作台"){hit=h.ToInt64();return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static bool ClickTray(long h){ return PostMessage((IntPtr)h, 6002, IntPtr.Zero, (IntPtr)0x0202); }
}
'@
[SW]::Init()

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
function CdpRightClick($wsUrl,[int]$x,[int]$y){
  CdpMouse $wsUrl 'mouseMoved' $x $y; Start-Sleep -Milliseconds 60
  CdpMouse $wsUrl 'mousePressed' $x $y 'right'; Start-Sleep -Milliseconds 70
  CdpMouse $wsUrl 'mouseReleased' $x $y 'right'
}
function MainWs {
  foreach($t in (Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8)){
    if($t.type -ne 'page'){continue}
    if((CdpEval $t.webSocketDebuggerUrl "(!!document.querySelector('.main-root'))") -eq 'True'){ return $t.webSocketDebuggerUrl }
  }
  return $null
}
# 读总结页上**渲染出来的**关键数字（用户真正看到的东西）
# 类名取自 SummaryPanel.vue 实际模板：.card(卡片) / .out .txt(摘要) / .left .v(遗留清单)
# 曾猜成 .kpi/.digest —— 读出来是空数组与空串，导致"前后相等"的**假通过**，
# 掩盖了真正的 bug。测试选择器必须对着源码核，不能猜。
function SummaryNums($ws){
  return CdpEval $ws "(()=>{const cards=[...document.querySelectorAll('.card')].map(e=>e.textContent.trim().replace(/\s+/g,''));const txt=(document.querySelector('.out .txt')||{}).textContent||'';const left=(document.querySelector('.left .v')||{}).textContent||'';const tags=[...document.querySelectorAll('.tags .xh-tag')].map(e=>e.textContent.trim());const bars=[...document.querySelectorAll('.bars .b i')].map(e=>e.style.height).join(',');return JSON.stringify({cards:cards,txt:txt.slice(0,80),left:left.slice(0,80),tags:tags,bars:bars});})()"
}

$fail = 0
function Check($n,[bool]$ok,$d=''){ if($ok){"  PASS  $n $d"}else{$script:fail++;"  FAIL  $n $d"} }

# ── 前置：启动应用 → 托盘单击开主窗口 → 切到总结页 ──
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 3
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"
Start-Sleep -Seconds 14
$p = Get-Process -Name 'xhy' -EA SilentlyContinue | Select-Object -First 1
if (-not $p) { "xhy 未启动"; exit 1 }
$pid32 = [uint32]$p.Id
if (-not [SW]::Main($pid32, $true)) {
  $tray = [SW]::TrayWnd($pid32)
  if ($tray -ne 0) { [void][SW]::ClickTray($tray); Start-Sleep -Seconds 4 }
}
Check '主窗口已打开' ([SW]::Main($pid32, $true) -ne 0)
$ws = MainWs
Check '主窗口 CDP 可用' ($null -ne $ws)
if (-not $ws) { exit 1 }

# 前置：确保目标清单"未被排除"（从干净状态开始，保证可重复运行）
$j = Get-Content $store -Raw | ConvertFrom-Json
$targetName = '待办'
$tl = $j.lists | Where-Object { $_.name -eq $targetName } | Select-Object -First 1
if ($tl.excludeFromStats) {
  ($j.lists | Where-Object { $_.name -eq $targetName }).excludeFromStats = $false
  $j | ConvertTo-Json -Depth 14 | Set-Content $store -Encoding utf8
  Get-Process -Name 'xhy' | Stop-Process -Force; Start-Sleep -Seconds 3
  Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"; Start-Sleep -Seconds 14
  $p = Get-Process -Name 'xhy' | Select-Object -First 1
  $pid32 = [uint32]$p.Id
  if (-not [SW]::Main($pid32, $true)) { $tray=[SW]::TrayWnd($pid32); if($tray -ne 0){[void][SW]::ClickTray($tray); Start-Sleep -Seconds 4} }
  $ws = MainWs
}

# ── 切到「总结」页（用户报 bug 时的所在页面）──
"=== 切到总结页 ==="
$sumTab = CdpEval $ws "(()=>{const b=[...document.querySelectorAll('.seg button')].find(x=>x.textContent.trim()==='总结');if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
Check '总结 tab 定位' ($sumTab -match '^\d+,\d+$')
if ($sumTab -match '^(\d+),(\d+)$') { CdpClick $ws ([int]$matches[1]) ([int]$matches[2]); Start-Sleep -Milliseconds 1800 }
Check '已停在总结页' ((CdpEval $ws "(!!document.querySelector('.sum .cards'))") -eq 'True')

$before = SummaryNums $ws
"  排除前: $before"

# ── ★ 核心：在总结页上右键清单 → 点「不计入总结」→ 数字必须变 ──
"=== 在总结页右键清单行，勾选「不计入总结」==="
$pos = CdpEval $ws "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.querySelector('i')&&x.textContent.trim().startsWith('$targetName'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
Check '清单行定位' ($pos -match '^\d+,\d+$') "(pos=$pos)"
if ($pos -match '^(\d+),(\d+)$') {
  CdpRightClick $ws ([int]$matches[1]) ([int]$matches[2])
  Start-Sleep -Milliseconds 1300
  $items = CdpEval $ws "([...document.querySelectorAll('.ctxmenu .mi')].map(x=>x.textContent.trim()).join('|'))"
  Check '右键菜单已弹出' ($items -ne '')
  $mp = CdpEval $ws "(()=>{const b=[...document.querySelectorAll('.ctxmenu .mi')].find(x=>x.textContent.includes('不计入总结'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
  if ($mp -match '^(\d+),(\d+)$') {
    CdpClick $ws ([int]$matches[1]) ([int]$matches[2])
    Start-Sleep -Milliseconds 2200   # 留足重算 + 渲染时间
    $excluded = (Get-Content $store -Raw | ConvertFrom-Json).lists | Where-Object { $_.name -eq $targetName } | Select-Object -First 1 | ForEach-Object { $_.excludeFromStats }
    Check '★ excludeFromStats 已落库' ($excluded -eq $true) "(实际 $excluded)"
    $after = SummaryNums $ws
    "  排除后: $after"
    Check '★★ 总结页数字已变化（用户报的 bug）' ($after -ne $before) "(前=$before 后=$after)"

    # 反向：取消排除 → 数字应回到原值
    "=== 取消排除，数字应复原 ==="
    $pos2 = CdpEval $ws "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.querySelector('i')&&x.textContent.trim().startsWith('$targetName'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
    if ($pos2 -match '^(\d+),(\d+)$') {
      CdpRightClick $ws ([int]$matches[1]) ([int]$matches[2]); Start-Sleep -Milliseconds 1300
      $mp2 = CdpEval $ws "(()=>{const b=[...document.querySelectorAll('.ctxmenu .mi')].find(x=>x.textContent.includes('不计入总结'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
      if ($mp2 -match '^(\d+),(\d+)$') {
        CdpClick $ws ([int]$matches[1]) ([int]$matches[2]); Start-Sleep -Milliseconds 2200
        $restored = SummaryNums $ws
        Check '★ 取消排除后数字复原' ($restored -eq $before) "(期望=$before 实际=$restored)"
      } else { Check '第二次菜单项定位' $false }
    } else { Check '第二次清单行定位' $false }
  } else { Check '「不计入总结」菜单项定位' $false "(mp=$mp)" }
}

""
if ($fail -eq 0) { "ALL PASS" } else { "$fail FAILED" }
exit $fail
