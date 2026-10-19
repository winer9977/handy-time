# 真实输入仿真：清单右键菜单（编辑/归档/删除）
# 右键 = PostMessage WM_RBUTTONDOWN/UP（浏览器据此触发 contextmenu）
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class LM {
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
    public static void LClick(long top, int x, int y){ var h=Render((IntPtr)top); if(h==IntPtr.Zero) return;
        PostMessage(h,0x0200,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(90);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(70);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
    public static void RClick(long top, int x, int y){ var h=Render((IntPtr)top); if(h==IntPtr.Zero) return;
        PostMessage(h,0x0200,IntPtr.Zero,Lp(x,y)); System.Threading.Thread.Sleep(90);
        PostMessage(h,0x0204,(IntPtr)2,Lp(x,y)); System.Threading.Thread.Sleep(70);
        PostMessage(h,0x0205,IntPtr.Zero,Lp(x,y)); }
}
'@
[LM]::Init()
function CdpSend($wsUrl,[hashtable[]]$msgs){ $ws=[System.Net.WebSockets.ClientWebSocket]::new(); $ct=[System.Threading.CancellationToken]::None
  try{$ws.ConnectAsync([Uri]$wsUrl,$ct).Wait(6000)|Out-Null}catch{return $null}
  if($ws.State -ne 'Open'){return $null}
  $buf=New-Object byte[] 524288
  foreach($m in $msgs){ $pl=$m|ConvertTo-Json -Depth 8 -Compress; $b=[Text.Encoding]::UTF8.GetBytes($pl)
    $ws.SendAsync([ArraySegment[byte]]::new($b),[System.Net.WebSockets.WebSocketMessageType]::Text,$true,$ct).Wait()
    $sb=New-Object Text.StringBuilder
    for($i=0;$i -lt 30;$i++){ $t=$ws.ReceiveAsync([ArraySegment[byte]]::new($buf),$ct); if(-not $t.Wait(8000)){break}
      $r=$t.Result; $null=$sb.Append([Text.Encoding]::UTF8.GetString($buf,0,$r.Count))
      if($sb.ToString() -match "`"id`":$($m.id)\b"){break} }
    if($m.id -eq 1){$script:last=$sb.ToString()} }
  $ws.Dispose() }
function CdpVal($wsUrl,$js){ CdpSend $wsUrl @(@{id=1;method='Runtime.evaluate';params=@{expression=$js;returnByValue=$true;awaitPromise=$true}})|Out-Null
  try{$o=$script:last|ConvertFrom-Json}catch{return $null}
  $v=$o.result.result.value
  if($v -is [string] -and $v.StartsWith('"')){try{$v=$v|ConvertFrom-Json}catch{}}
  return "$v" }
$SCALE=1.25
$store="$env:APPDATA\dev.xhy.desktop\store.json"
$fails=0
function Check($n,[bool]$ok,$d=''){ if($ok){"  PASS  $n $d"}else{$script:fails++;"  FAIL  $n $d"} }

function ListNames { ((Get-Content $store -Raw|ConvertFrom-Json).lists | ForEach-Object { $_.name }) -join ',' }
function ArchivedNames { ((Get-Content $store -Raw|ConvertFrom-Json).lists | Where-Object { $_.archived } | ForEach-Object { $_.name }) -join ',' }

"=== 准备：停应用 → 注入测试清单+待办 → 重启 ==="
# 关键：应用运行时持有 store 的**内存副本**，直接改文件不会生效（退出时
# 还会把副本写回、覆盖注入）。注入必须在启动前做。
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 3
if (-not (Test-Path "$store.sim3bak")) { Copy-Item $store "$store.sim3bak" -Force }
$j = Get-Content $store -Raw | ConvertFrom-Json
if (($j.lists | ForEach-Object { $_.id }) -notcontains 'simlist1') {
  $j.lists = @($j.lists) + [pscustomobject]@{ id='simlist1'; name='仿真清单'; color='#e5484d'; order=99; collapsed=$false; archived=$false }
}
if (($j.todos | ForEach-Object { $_.id }) -notcontains 'simtodo1') {
  $j.todos = @($j.todos) + [pscustomobject]@{ id='simtodo1'; listId='simlist1'; title='仿真待办'; note=''; done=$false; doneAt=$null; pinned=$false; order=99
    tags=@(); due=$null; remindAt=$null; reminded=$false; extraReminds=@(); remindedKeys=@(); constantRemind=$false; lastRemindedAt=$null
    countdown=$null; priority=$null; recurrence=$null; createdAt='2026-10-18T08:00'; archivedAt=$null }
}
$j | ConvertTo-Json -Depth 14 | Set-Content $store -Encoding utf8
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'
$env:XHY_UI_MAIN='1'
Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"
Start-Sleep -Seconds 15

$p=Get-Process -Name 'xhy'|Select-Object -First 1; $id=[uint32]$p.Id
$mainTop=[LM]::Top($id,'xhy 工作台')
$mainWs=$null
foreach($t in (Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8)){ if($t.type -ne 'page'){continue}
  if((CdpVal $t.webSocketDebuggerUrl "(!!document.querySelector('.main-root'))") -eq 'true'){$mainWs=$t.webSocketDebuggerUrl} }
Check '主窗口定位' ($mainTop -ne 0 -and $null -ne $mainWs)
Check '测试清单渲染在侧栏' ((CdpVal $mainWs "([...document.querySelectorAll('.sitem')].some(x=>x.textContent.trim().startsWith('仿真清单')))?'YES':'NO'") -eq 'YES')

# 目标行定位（cl 归零，返回 "x,y"）
function RowPos($name){ CdpVal $mainWs "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.textContent.trim().startsWith('$name'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()" }
# 菜单项定位
function MenuPos($label){ CdpVal $mainWs "(()=>{const b=[...document.querySelectorAll('.ctxmenu .mi')].find(x=>x.textContent.trim()==='$label');if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()" }


"=== ① 右键 → 归档 ==="
$pos = RowPos '仿真清单'
if ($pos -match '^(\d+),(\d+)$') {
  [LM]::RClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
  $menuShown = CdpVal $mainWs "document.querySelector('.ctxmenu')?'YES':'NO'"
  Check '右键弹出菜单' ($menuShown -eq 'YES')
  $labels = CdpVal $mainWs "([...document.querySelectorAll('.ctxmenu .mi')].map(x=>x.textContent.trim()).join('|'))"
  Check '菜单含 编辑/归档/删除' ($labels -match '编辑' -and $labels -match '归档' -and $labels -match '删除') "(实际 $labels)"
  $mp = MenuPos '归档'
  if ($mp -match '^(\d+),(\d+)$') {
    [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1800
    Check '归档落库' ((ArchivedNames) -match '仿真清单') "(archived=$(ArchivedNames))"
    $inMain = CdpVal $mainWs "([...document.querySelectorAll('.sitem')].some(x=>x.textContent.trim().startsWith('仿真清单')))?'YES':'NO'"
    Check '归档后从侧栏主区消失' ($inMain -eq 'NO')
    $folded = CdpVal $mainWs "([...document.querySelectorAll('.sitem')].some(x=>x.textContent.includes('已归档')))?'YES':'NO'"
    Check '出现「已归档」折叠区' ($folded -eq 'YES')
  } else { Check '归档菜单项定位' $false "(mp=$mp)" }
} else { Check '仿真清单行定位' $false "(pos=$pos)" }

"=== ② 展开已归档 → 右键 → 取消归档 ==="
$foldPos = CdpVal $mainWs "(()=>{const b=[...document.querySelectorAll('.sitem')].find(x=>x.textContent.includes('已归档'));if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
if ($foldPos -match '^(\d+),(\d+)$') {
  [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1000
  $pos2 = RowPos '仿真清单'
  if ($pos2 -match '^(\d+),(\d+)$') {
    [LM]::RClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
    $mp2 = MenuPos '取消归档'
    Check '归档项文案变为「取消归档」' ($mp2 -ne '') "(mp=$mp2)"
    if ($mp2 -match '^(\d+),(\d+)$') {
      [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1800
      Check '取消归档落库' ((ArchivedNames) -notmatch '仿真清单') "(archived=$(ArchivedNames))"
      $back = CdpVal $mainWs "([...document.querySelectorAll('.sitem')].some(x=>x.textContent.trim().startsWith('仿真清单')))?'YES':'NO'"
      Check '回到侧栏主区' ($back -eq 'YES')
    }
  } else { Check '归档区内行定位' $false "(pos=$pos2)" }
} else { Check '「已归档」区定位' $false }

"=== ③ 右键 → 编辑（改名+换色）==="
$pos3 = RowPos '仿真清单'
if ($pos3 -match '^(\d+),(\d+)$') {
  [LM]::RClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
  $mp3 = MenuPos '编辑'
  if ($mp3 -match '^(\d+),(\d+)$') {
    [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
    $d1 = CdpVal $mainWs "document.querySelector('.mask .dlg input')?'YES':'NO'"
    Check '编辑弹第一个框（名称）' ($d1 -eq 'YES')
    CdpSend $mainWs @(@{id=2;method='Input.insertText';params=@{text='仿真改名'}})|Out-Null
    Start-Sleep -Milliseconds 400
    $okPos = CdpVal $mainWs "(()=>{const b=document.querySelector('.mask .dlg button.ok');if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
    if ($okPos -match '^(\d+),(\d+)$') {
      [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
      $d2 = CdpVal $mainWs "document.querySelector('.mask .dlg input')?'YES':'NO'"
      Check '编辑弹第二个框（颜色）' ($d2 -eq 'YES')
      CdpSend $mainWs @(@{id=3;method='Input.insertText';params=@{text='#30a46c'}})|Out-Null
      Start-Sleep -Milliseconds 400
      $okPos2 = CdpVal $mainWs "(()=>{const b=document.querySelector('.mask .dlg button.ok');if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
      if ($okPos2 -match '^(\d+),(\d+)$') {
        [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1800
        $jl = Get-Content $store -Raw | ConvertFrom-Json
        $t2 = $jl.lists | Where-Object { $_.id -eq 'simlist1' }
        Check '编辑落库（名+色）' ($t2.name -eq '仿真改名' -and $t2.color -eq '#30a46c') "(name=$($t2.name) color=$($t2.color))"
      } else { Check '颜色框确定按钮' $false }
    } else { Check '名称框确定按钮' $false }
  } else { Check '编辑菜单项定位' $false "(mp=$mp3)" }
} else { Check '编辑目标行定位' $false }

"=== ④ 右键 → 删除（带确认，待办移入第一个清单）==="
# 测试待办已在启动前注入（simtodo1）。此处只做真实点击验证。
$pos4 = RowPos '仿真改名'
if ($pos4 -match '^(\d+),(\d+)$') {
  [LM]::RClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
  $mp4 = MenuPos '删除'
  if ($mp4 -match '^(\d+),(\d+)$') {
    [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 1200
    $confirm = CdpVal $mainWs "document.querySelector('.mask .dlg input')?'YES':'NO'"
    Check '删除前二次确认' ($confirm -eq 'YES')
    $okPos3 = CdpVal $mainWs "(()=>{const b=document.querySelector('.mask .dlg button.ok');if(!b)return '';const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*$SCALE)+','+Math.round((r.top+r.height/2)*$SCALE);})()"
    if ($okPos3 -match '^(\d+),(\d+)$') {
      [LM]::LClick($mainTop,[int]$matches[1],[int]$matches[2]); Start-Sleep -Milliseconds 2000
      $jd = Get-Content $store -Raw | ConvertFrom-Json
      Check '清单已删除' ((($jd.lists | ForEach-Object { $_.id }) -notcontains 'simlist1'))
      $kept = $jd.todos | Where-Object { $_.id -eq 'simtodo1' }
      Check '★ 待办未被连带删除' ($null -ne $kept)
      Check '★ 待办移入第一个剩余清单' ($kept.listId -eq $jd.lists[0].id) "(list=$($kept.listId))"
    } else { Check '删除确认按钮' $false }
  } else { Check '删除菜单项定位' $false }
} else { Check '删除目标行定位' $false }

"=== 清理：停应用 → 从备份整体还原（最干净，避免残留测试数据）==="
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 3
Copy-Item "$store.sim3bak" $store -Force
Remove-Item "$store.sim3bak" -Force
$restored = Get-Content $store -Raw | ConvertFrom-Json
"  已还原；清单=$(($restored.lists | ForEach-Object { $_.name }) -join ',') 待办=$(($restored.todos|Measure-Object).Count) 归档=$(($restored.lists | Where-Object { $_.archived } | Measure-Object).Count)"
""
if($fails -eq 0){"═══ 全部通过 ═══"}else{"═══ $fails 项失败 ═══";exit 1}
