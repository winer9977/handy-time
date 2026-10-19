# 端到端验证「倒数日 + 到点后继续累计」（v3：用 CDP 真实键鼠事件）
#
# 为什么不用 PostMessage WM_CHAR：Chromium 的 input[type=date] 是**原生内部
# 控件**，字符走它自己的子窗口，我们往 WebView2 主机窗口投 WM_CHAR 到不了
# （实测：定位成功、点击成功、面板没关，但 value 始终为空）。
# CDP 的 Input.dispatchKeyEvent / insertText 走浏览器自身输入管线，等价真键盘。
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class CW {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public static void Init(){SetProcessDPIAware();}
    static string T(IntPtr h){ var tt=new StringBuilder(64); GetWindowTextW(h,tt,64); return tt.ToString(); }
    public static long Top(uint pid, string want){ long hit=0;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            if(T(h)==want){hit=h.ToInt64(); return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Render(IntPtr top){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(top,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)(((y & 0xFFFF)<<16)|(x & 0xFFFF)); }
    /// 真实鼠标点击（走命中测试 + pointer 链）
    public static void Click(long top,int x,int y){ var h=Render((IntPtr)top); if(h==IntPtr.Zero) return;
        PostMessage(h,0x0200,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(90);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(80);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(60); }
}
'@
[CW]::Init()

# --- CDP 会话（一次连接跑多条命令）---
function New-Cdp($wsUrl){
  $ws=[System.Net.WebSockets.ClientWebSocket]::new(); $ct=[System.Threading.CancellationToken]::None
  $ws.ConnectAsync([Uri]$wsUrl,$ct).Wait(6000) | Out-Null
  return @{ ws=$ws; ct=$ct; id=0 }
}
function Cdp-Call($s,[string]$method,[hashtable]$params){
  $s.id++
  $id=$s.id
  $pl=@{id=$id;method=$method;params=$params}|ConvertTo-Json -Depth 10 -Compress
  $b=[Text.Encoding]::UTF8.GetBytes($pl)
  $s.ws.SendAsync([ArraySegment[byte]]::new($b),[System.Net.WebSockets.WebSocketMessageType]::Text,$true,$s.ct).Wait()
  $buf=New-Object byte[] 524288; $sb=New-Object Text.StringBuilder
  for($i=0;$i -lt 30;$i++){
    $t=$s.ws.ReceiveAsync([ArraySegment[byte]]::new($buf),$s.ct); if(-not $t.Wait(8000)){break}
    $r=$t.Result; $null=$sb.Append([Text.Encoding]::UTF8.GetString($buf,0,$r.Count))
    if($sb.ToString() -match "`"id`":$id\b"){break}
  }
  return $sb.ToString()
}
function Cdp-Val($s,[string]$js){
  $raw = Cdp-Call $s 'Runtime.evaluate' @{ expression=$js; returnByValue=$true; awaitPromise=$true }
  try { $o = $raw | ConvertFrom-Json } catch { return $null }
  $v = $o.result.result.value
  if ($v -is [string] -and $v.StartsWith('"')) { try { $v = $v | ConvertFrom-Json } catch {} }
  return "$v"
}
function Cdp-Pos($s,[string]$js){ $r = Cdp-Val $s $js; if ($r -match '^(\d+),(\d+)$') { return @([int]$matches[1],[int]$matches[2]) }; return $null }

$p = Get-Process -Name 'xhy' | Select-Object -First 1
$id=[uint32]$p.Id
$top=[CW]::Top($id,'待办清单')
# 找待办挂件页面
$wsUrl=$null
foreach($t in (Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8)){ if($t.type -ne 'page'){continue}
  $probe = New-Cdp $t.webSocketDebuggerUrl
  if((Cdp-Val $probe "(!!document.querySelector('.xh-title'))") -eq 'True'){ $wsUrl=$t.webSocketDebuggerUrl; $probe.ws.Dispose(); break }
  $probe.ws.Dispose()
}
if (-not $wsUrl) { "找不到待办挂件页面"; exit 1 }
$s = New-Cdp $wsUrl
$fail=0
function Check($n,[bool]$ok,$d=''){ if($ok){"  PASS  $n $d"}else{$script:fail++;"  FAIL  $n $d"} }

"=== 打开面板 ==="
if ((Cdp-Val $s "(document.querySelector('.xh-pop')?'YES':'NO')") -ne 'YES') {
  $d = Cdp-Pos $s "(()=>{const b=[...document.querySelectorAll('.ic')].filter(x=>x.textContent.trim()==='⋯')[0];if(!b)return '';const g=b.getBoundingClientRect();return Math.round((g.left+g.width/2)*1.25)+','+Math.round((g.top+g.height/2)*1.25);})()"
  if ($d) { [CW]::Click($top,$d[0],$d[1]); Start-Sleep -Milliseconds 1200 }
}
Check '面板打开' ((Cdp-Val $s "(document.querySelector('.xh-pop')?'YES':'NO')") -eq 'YES')

"=== 用 CDP 真实键盘给日期框输入（原生控件必须走浏览器输入管线）==="
$ip = Cdp-Pos $s "(()=>{const r=[...document.querySelectorAll('.xh-pop .prow')].find(x=>x.textContent.includes('倒数日'));const i=r&&r.querySelector('input');if(!i)return '';const g=i.getBoundingClientRect();return Math.round((g.left+12)*1.25)+','+Math.round((g.top+g.height/2)*1.25);})()"
Check '倒数日输入框定位' ($null -ne $ip)
if ($ip) {
  [CW]::Click($top,$ip[0],$ip[1]); Start-Sleep -Milliseconds 500
  # CDP 鼠标点击 + 键盘输入
  Cdp-Call $s 'Input.dispatchMouseEvent' @{ type='mousePressed'; x=$ip[0]; y=$ip[1]; button='left'; clickCount=1 } | Out-Null
  Cdp-Call $s 'Input.dispatchMouseEvent' @{ type='mouseReleased'; x=$ip[0]; y=$ip[1]; button='left'; clickCount=1 } | Out-Null
  Start-Sleep -Milliseconds 300
  # 全选 + 输入 8 位数字（Chromium 日期框接受 MM DD YYYY 连打）
  Cdp-Call $s 'Input.dispatchKeyEvent' @{ type='keyDown'; key='a'; code='KeyA'; modifiers=2; windowsVirtualKeyCode=65 } | Out-Null
  Cdp-Call $s 'Input.dispatchKeyEvent' @{ type='keyUp'; key='a'; code='KeyA'; modifiers=2; windowsVirtualKeyCode=65 } | Out-Null
  Start-Sleep -Milliseconds 200
  Cdp-Call $s 'Input.insertText' @{ text='10102026' } | Out-Null
  Start-Sleep -Milliseconds 900
  $val = Cdp-Val $s "(()=>{const r=[...document.querySelectorAll('.xh-pop .prow')].find(x=>x.textContent.includes('倒数日'));const i=r&&r.querySelector('input');return i?i.value:'';})()"
  Check '日期框写入成功' ($val -match '2026-10-10') "(value=「$val」)"
}

"=== 落库 / 徽标 / 开关 ==="
$st = (Get-Content "$env:APPDATA\dev.xhy.desktop\store.json" -Raw | ConvertFrom-Json).todos | Where-Object { $_.countdown } | Select-Object -First 1
Check '倒数日已落库' ($null -ne $st) "(date=$(if($st){$st.countdown.date}else{'无'}))"
if ($st) {
  $badge = Cdp-Val $s "((document.querySelector('.xh-badge.cd')||{}).textContent)||''"
  Check '徽标显示倒数文案' ($badge -ne '') "(「$badge」)"
  $row = Cdp-Val $s "([...document.querySelectorAll('.xh-pop .prow')].some(r=>r.textContent.includes('到点后'))?'YES':'NO')"
  Check '「到点后」开关行出现' ($row -eq 'YES')
  if ($row -eq 'YES') {
    $btn = Cdp-Pos $s "(()=>{const r=[...document.querySelectorAll('.xh-pop .prow')].find(x=>x.textContent.includes('到点后'));const b=r&&r.querySelector('button');if(!b)return '';const g=b.getBoundingClientRect();return Math.round((g.left+g.width/2)*1.25)+','+Math.round((g.top+g.height/2)*1.25);})()"
    if ($btn) {
      [CW]::Click($top,$btn[0],$btn[1]); Start-Sleep -Milliseconds 1700
      $keep = ((Get-Content "$env:APPDATA\dev.xhy.desktop\store.json" -Raw | ConvertFrom-Json).todos | Where-Object { $_.countdown } | Select-Object -First 1).countdown.keepCountingUp
      $after = Cdp-Val $s "((document.querySelector('.xh-badge.cd')||{}).textContent)||''"
      Check 'keepCountingUp=true 落库' ($keep -eq $true) "(实际 $keep)"
      Check '文案变「已经 N 天」' ($after -like '已经*') "(「$after」)"
      [CW]::Click($top,$btn[0],$btn[1]); Start-Sleep -Milliseconds 1700
      $back = Cdp-Val $s "((document.querySelector('.xh-badge.cd')||{}).textContent)||''"
      Check '关回后变「已过 N 天」' ($back -like '已过*') "(「$back」)"
    } else { Check '开关按钮定位' $false }
  }
}
$s.ws.Dispose()
""
if ($fail -eq 0) { "═══ 全部通过 ═══" } else { "═══ $fail 项失败 ═══"; exit 1 }
