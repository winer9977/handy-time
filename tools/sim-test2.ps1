# 真实输入仿真：① 挂件「全」按钮  ② 主窗口双击清单改名
# 点击/双击 = PostMessage 真实鼠标消息（走浏览器命中测试）；
# 中文输入 = CDP Input.insertText（WM_CHAR 注不进 CJK，实测教训）；
# 提交 = 真鼠标点弹窗「改名」按钮。
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class S3 {
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
    public static void Click(long top, int x, int y){ var h=Render((IntPtr)top); if(h==IntPtr.Zero) return;
        PostMessage(h,0x0200,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(90);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(70);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
    // 真实双击序列：DOWN UP DBLCLK(带MK_LBUTTON) UP —— 浏览器据此合成 dblclick
    public static void DblClick(long top, int x, int y){ var h=Render((IntPtr)top); if(h==IntPtr.Zero) return;
        PostMessage(h,0x0200,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(90);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(60);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(60);
        PostMessage(h,0x0203,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(60);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
}
'@
[S3]::Init()
function CdpSend($wsUrl, [hashtable[]]$msgs) {
  $ws=[System.Net.WebSockets.ClientWebSocket]::new(); $ct=[System.Threading.CancellationToken]::None
  try { $ws.ConnectAsync([Uri]$wsUrl,$ct).Wait(6000)|Out-Null } catch { return $null }
  if ($ws.State -ne 'Open') { return $null }
  $buf=New-Object byte[] 524288
  foreach ($m in $msgs) {
    $pl = $m | ConvertTo-Json -Depth 8 -Compress
    $b=[Text.Encoding]::UTF8.GetBytes($pl)
    $ws.SendAsync([ArraySegment[byte]]::new($b),[System.Net.WebSockets.WebSocketMessageType]::Text,$true,$ct).Wait()
    # 排干到这条 id 的响应
    $sb=New-Object Text.StringBuilder
    for($i=0;$i -lt 30;$i++){
      $t=$ws.ReceiveAsync([ArraySegment[byte]]::new($buf),$ct); if(-not $t.Wait(8000)){break}
      $r=$t.Result; $null=$sb.Append([Text.Encoding]::UTF8.GetString($buf,0,$r.Count))
      if($sb.ToString() -match "`"id`":$($m.id)\b"){break}
    }
    if ($m.id -eq 1) { $script:last = $sb.ToString() }
  }
  $ws.Dispose()
}
function CdpVal($wsUrl,$js){
  CdpSend $wsUrl @(@{id=1;method='Runtime.evaluate';params=@{expression=$js;returnByValue=$true;awaitPromise=$true}}) | Out-Null
  $s=$script:last
  # 整个响应用 JSON 解析器处理（\uXXXX、引号转义都是它的本职），
  # 不再手工正则 —— 上一版手工解码在中文上崩过两次。
  try { $obj = $s | ConvertFrom-Json } catch { return $null }
  $v = $obj.result.result.value
  if ($null -eq $v) { return $null }
  if ($v -is [string]) {
    # returnByValue 对字符串会二次编码（值本身是 JSON 字面量 "365,24"），再解一层
    if ($v.StartsWith('"')) { try { $v = $v | ConvertFrom-Json } catch {} }
    return $v
  }
  return "$v"
}
function FindPage($probeJs) {
  foreach ($t in (Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8)) {
    if ($t.type -ne 'page') { continue }
    if ((CdpVal $t.webSocketDebuggerUrl $probeJs) -eq 'true') { return $t.webSocketDebuggerUrl }
  }
  return $null
}
$SCALE = 1.25
$store = "$env:APPDATA\dev.xhy.desktop\store.json"
$fails = 0
function Check($name,[bool]$ok,$detail='') {
  if ($ok) { "  PASS  $name $detail" } else { $script:fails++; "  FAIL  $name $detail" }
}
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$id = [uint32]$p.Id

"=== ① 挂件「全」按钮（真实单击） ==="
$todoTop = [S3]::Top($id,'待办清单')
Check '待办挂件窗口存在' ($todoTop -ne 0)
$todoWs = FindPage "(!!document.querySelector('.xh-btn')&&[...document.querySelectorAll('.xh-btn')].some(b=>b.textContent.trim()==='全'))"
Check '定位到待办挂件页面' ($null -ne $todoWs)
if ($todoWs) {
  $posJson = CdpVal $todoWs "JSON.stringify((()=>{const b=[...document.querySelectorAll('.xh-btn')].find(x=>x.textContent.trim()==='全');if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})())"
  if ($posJson -match '^(\d+),(\d+)$') {
    [S3]::Click($todoTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1800
    $title = CdpVal $todoWs "((document.querySelector('.xh-title')||{}).textContent)||''"
    $lid = (Get-Content $store -Raw | ConvertFrom-Json).widgets | Where-Object { $_.kind -eq 'todo' } | ForEach-Object { $_.listId }
    Check '点「全」→ 标题变「待办 · 全部」' ($title -eq '待办 · 全部') "(实际「$title」)"
    Check '点「全」→ store.listId 清空' ($null -eq $lid) "(实际 $lid)"
    # 点回收集箱标签，恢复原状态并验证双向
    $pos2 = CdpVal $todoWs "JSON.stringify((()=>{const b=[...document.querySelectorAll('button')].find(x=>x.textContent.trim().startsWith('收集箱'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})())"
    if ($pos2 -match '^(\d+),(\d+)$') {
      [S3]::Click($todoTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1500
      $t2 = CdpVal $todoWs "((document.querySelector('.xh-title')||{}).textContent)||''"
      Check '点清单标签切回「收集箱」' ($t2 -eq '收集箱') "(实际「$t2」)"
    } else { Check '收集箱标签定位' $false }
  } else { Check '「全」按钮定位' $false }
}

"=== ② 主窗口双击清单改名（真实双击 + IME 文本 + 真点击确定） ==="
$mainTop = [S3]::Top($id,'xhy 工作台')
Check '主窗口存在' ($mainTop -ne 0)
$mainWs = FindPage "(!!document.querySelector('.main-root'))"
Check '主窗口页面定位' ($null -ne $mainWs)
if ($mainTop -and $mainWs) {
  # 侧栏「待办」清单行（排除「待办清单」挂件行）
  $pos = CdpVal $mainWs "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>{const s=x.textContent.trim();return s.startsWith('待办')&&!s.includes('清单')});if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
  if ($pos -match '^(\d+),(\d+)$') {
    [S3]::DblClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1500
    $shown = CdpVal $mainWs "document.querySelector('.mask .dlg input')?'YES':'NO'"
    Check '双击弹出改名框（真实 dblclick 链）' ($shown -eq 'YES')
    if ($shown -eq 'YES') {
      # 全选旧值（overlay 挂载时已 focus+select，直接 IME 提交替换）
      CdpSend $mainWs @(
        @{id=2;method='Input.insertText';params=@{text='改名验证'}}
      ) | Out-Null
      Start-Sleep -Milliseconds 600
      $val = CdpVal $mainWs "document.querySelector('.mask .dlg input').value"
      Check '输入新名（insertText 进 v-model）' ($val -eq '改名验证') "(实际「$val」)"
      # 真鼠标点「改名」按钮
      $okPos = CdpVal $mainWs "(()=>{const b=document.querySelector('.mask .dlg button.ok');if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
      if ($okPos -match '^(\d+),(\d+)$') {
        [S3]::Click($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 2000
        $names = ((Get-Content $store -Raw | ConvertFrom-Json).lists | ForEach-Object { $_.name }) -join ','
        Check '改名落库' ($names -match '改名验证') "(lists=$names)"
        $shown2 = CdpVal $mainWs "([...document.querySelectorAll('.sitem')].some(x=>x.textContent.includes('改名验证')))?'YES':'NO'"
        Check '侧栏即时显示新名' ($shown2 -eq 'YES')
        # 还原：再双击那一行
        $pos3 = CdpVal $mainWs "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.textContent.trim().startsWith('改名验证'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
        if ($pos3 -match '^(\d+),(\d+)$') {
          [S3]::DblClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
          CdpSend $mainWs @(@{id=3;method='Input.insertText';params=@{text='待办'}}) | Out-Null
          Start-Sleep -Milliseconds 500
          $okPos2 = CdpVal $mainWs "(()=>{const b=document.querySelector('.mask .dlg button.ok');if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
          if ($okPos2 -match '^(\d+),(\d+)$') {
            [S3]::Click($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1800
          }
          $names2 = ((Get-Content $store -Raw | ConvertFrom-Json).lists | ForEach-Object { $_.name }) -join ','
          Check '还原清单名（二次改名）' ($names2 -match '待办' -and $names2 -notmatch '改名验证') "(lists=$names2)"
        } else { Check '改名后行定位' $false }
      } else { Check '确定按钮定位' $false }
    }
  } else { Check '侧栏「待办」行定位' $false "(pos=$pos)" }
}

""
if ($fails -eq 0) { "═══ 全部通过 ═══" } else { "═══ $fails 项失败 ═══"; exit 1 }
