# 逐个点击筛选按钮 + 标签，验证：① widget.filter 真的落库 ② 渲染符合口径
# 用法: powershell -File tools\verify-filter.ps1
$w = Split-Path -Parent $PSScriptRoot   # 仓库根，不写死本机路径
$s = "$env:APPDATA\dev.xhy.desktop\store.json"
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class FV {
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
    public static void Aware(){SetProcessDPIAware();}
    public static IntPtr Render(uint pid, string want){ IntPtr top=IntPtr.Zero;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            var tt=new StringBuilder(64); GetWindowTextW(h,tt,64);
            if(tt.ToString().Contains(want)){top=h;return false;} return true; }, IntPtr.Zero);
        IntPtr hit=IntPtr.Zero;
        EnumChildWindows(top,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)((y<<16)|(x&0xFFFF)); }
    public static void Click(IntPtr h,int x,int y){ PostMessage(h,0x0200,(IntPtr)0,Lp(x,y));
        System.Threading.Thread.Sleep(70);
        PostMessage(h,0x0201,(IntPtr)1,Lp(x,y)); System.Threading.Thread.Sleep(150);
        PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
}
'@
[FV]::Aware()
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$rt = [FV]::Render([uint32]$p.Id, '待办')
$script:fail = 0

# ── 前置：把 filter/tag 重置为初始态 ──
# 脚本原本假定起点是 all（"点今天"= 设置），一旦上次跑完留下 today，
# 本次"点今天"就变成取消，后续断言集体反向 —— 实测踩过（3 项 FAIL 全是假警报）。
# 测试脚本必须自己建立前置状态，不能依赖调用方把应用留成什么样。
#
# 重置手段：直接改 store（应用运行时持有内存副本，所以必须**重启**才生效）。
Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
$j = Get-Content $s -Raw | ConvertFrom-Json
($j.widgets | Where-Object { $_.kind -eq 'todo' }) | ForEach-Object { $_.filter = 'all'; $_.tag = $null }
$j | ConvertTo-Json -Depth 14 | Set-Content $s -Encoding utf8
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
Start-Process "$env:LOCALAPPDATA\xhy\xhy.exe"
Start-Sleep -Seconds 14
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$rt = [FV]::Render([uint32]$p.Id, '待办')
"前置：filter 已重置为 all，应用已重启"

function WidgetState {
    $j = Get-Content $s -Raw | ConvertFrom-Json
    $j.widgets | Where-Object { $_.kind -eq 'todo' } | Select-Object -First 1
}
function Check($name, $ok, $detail) {
    if ($ok) { "  OK   $name" } else { "  FAIL $name  $detail"; $script:fail++ }
}
function Shot($tag) {
    & "$w\tools\capture-windows.ps1" -OutDir "$w\.fv" -ProcessName 'xhy' 2>&1 | Out-Null
    $src = Get-ChildItem "$w\.fv" -Filter 'win0-*.png' | Select-Object -First 1
    if ($src) { Move-Item $src.FullName (Join-Path "$w\.fv" ("f-" + $tag + ".png")) -Force }
}
function ClickFilter($label, $x, $want) {    "[$label]"
    [FV]::Click($rt, $x, 89)
    Start-Sleep -Milliseconds 1000
    $st = WidgetState
    Check ("落库 filter=" + $st.filter + "（期望 $want）") ($st.filter -eq $want) ("实际=" + $st.filter)
    Shot $label
}

# ── CDP：读元素真实位置（用于坐标易变的按钮，如 ✕ 清除）──
# 为什么需要：写死坐标在挂件宽度变化后必然偏移（实测 690,89 在 375 宽挂件里
# 落到窗口外，点不到却被误判成"功能坏了"）。动态定位让脚本与布局解耦。
function CdpEval($wsUrl, $js) {
    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    $ct = [System.Threading.CancellationToken]::None
    try { $ws.ConnectAsync([Uri]$wsUrl, $ct).Wait(6000) | Out-Null } catch { return $null }
    if ($ws.State -ne 'Open') { $ws.Dispose(); return $null }
    $pl = @{ id = 1; method = 'Runtime.evaluate'; params = @{ expression = $js; returnByValue = $true; awaitPromise = $true } } | ConvertTo-Json -Depth 8 -Compress
    $b = [Text.Encoding]::UTF8.GetBytes($pl)
    $ws.SendAsync([ArraySegment[byte]]::new($b), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait()
    $buf = New-Object byte[] 262144
    $sb = New-Object Text.StringBuilder
    for ($i = 0; $i -lt 25; $i++) {
        $tk = $ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $ct)
        if (-not $tk.Wait(8000)) { break }
        $r = $tk.Result
        $null = $sb.Append([Text.Encoding]::UTF8.GetString($buf, 0, $r.Count))
        if ($sb.ToString() -match '"id":1') { break }
    }
    $ws.Dispose()
    try { $o = $sb.ToString() | ConvertFrom-Json } catch { return $null }
    $v = $o.result.result.value
    if ($v -is [string] -and $v.StartsWith('"')) { try { $v = $v | ConvertFrom-Json } catch {} }
    return "$v"
}
function WidgetPageWs {
    foreach ($t in (Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 8)) {
        if ($t.type -ne 'page') { continue }
        if ((CdpEval $t.webSocketDebuggerUrl "(!!document.querySelector('.xh-title'))") -eq 'True') {
            return $t.webSocketDebuggerUrl
        }
    }
    return $null
}
# 按 CSS 选择器点一个元素：先 scrollIntoView（避免落在窗口外触发 click-away），
# 再取物理坐标并用真实鼠标消息点击
function ClickBySelector([string]$sel) {
    $ws = WidgetPageWs
    if (-not $ws) { return $false }
    $g = CdpEval $ws "(()=>{const b=document.querySelector('$sel');if(!b)return '';b.scrollIntoView({block:'center'});const r=b.getBoundingClientRect();return Math.round((r.left+r.width/2)*1.25)+','+Math.round((r.top+r.height/2)*1.25);})()"
    if ($g -notmatch '^(\d+),(\d+)$') { return $false }
    [FV]::Click($rt, [int]$matches[1], [int]$matches[2])
    return $true
}

ClickFilter 'today' 35 'today'
[FV]::Click($rt, 35, 89); Start-Sleep -Milliseconds 1000
$st = WidgetState
Check ("再点「今天」取消 → filter=" + $st.filter + "（期望 all）") ($st.filter -eq 'all') ("实际=" + $st.filter)

ClickFilter 'week' 88 'week'
ClickFilter 'flagged' 141 'flagged'
ClickFilter 'someday' 196 'someday'
ClickFilter 'done' 250 'done'

"[tag 工作]"
[FV]::Click($rt, 40, 123)
Start-Sleep -Milliseconds 1000
$st = WidgetState
Check ("落库 tag=" + $st.tag + "（期望 工作）") ($st.tag -eq '工作') ("实际=" + $st.tag)
Shot 'tagwork'

"[叠加 today + 工作]"
[FV]::Click($rt, 35, 89); Start-Sleep -Milliseconds 1000
$st = WidgetState
Check ("filter=" + $st.filter + " tag=" + $st.tag + "（期望 today + 工作）") ($st.filter -eq 'today' -and $st.tag -eq '工作') ("实际=" + $st.filter + "/" + $st.tag)
Shot 'combo'

"[清除]"
# 用动态定位（✕ 按钮的坐标随挂件宽度变，写死会点空）
if (-not (ClickBySelector '.fbtn.clr')) { "  （未找到 ✕ 清除按钮）"; $script:fail++ }
Start-Sleep -Milliseconds 1000
$st = WidgetState
Check ("清除后 filter=" + $st.filter + " tag=[" + $st.tag + "]（期望 all / 空）") ($st.filter -eq 'all' -and -not $st.tag) ("实际=" + $st.filter + "/" + $st.tag)

""
if ($script:fail -eq 0) { "ALL PASS" } else { "$script:fail FAILED" }
exit $script:fail
