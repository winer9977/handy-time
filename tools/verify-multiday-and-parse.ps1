# 真机验证（第 6-7 轮）：全写日期 / 周X=今天 / 跨天日程渲染 / 时长显示 / 行内改跨天与时长 /
#                       无日期默认今天 / 撤掉「待定」/ Markdown 导入 / 废弃字段可读
#
# 这些都是"纯函数测试与集成测试抓不到"的一类：真实输入链（CDP 键入 → v-model →
# 提交命令）、真实渲染结果（长条占几列、结束时刻怎么拼）、真实反序列化
# （老库里已废弃的 settings.hotkeys）。
#
# 手法：WebView2 CDP(9222) 读 DOM + 派发真实输入/点击；数据用
# `%APPDATA%\dev.xhy.desktop` 原目录，先整份备份、写一次性 fixture，跑完还原，
# 用户的真实待办不会被实验污染。
#
# ★★ 启动路径必须是纯 ASCII（本轮实测定位，值得记住）★★
#   `Start-Process "<仓库路径>\app\src-tauri\target\debug\xhy.exe"` 会**卡在
#   Start-Process 里不返回**（进程根本没起来，日志一行都没有）；把同一个 exe 复制到
#   `%LOCALAPPDATA%\xhy-verify\` 后立刻 `CDP=200`。所以脚本先把待测二进制复制到
#   ASCII 暂存目录再启动 —— 测的仍是刚 build 出来的那份（size/编译时间会打印出来核对）。
#   与 README「NSIS 中文路径坑」同一类问题：GUI 进程创建对非 ASCII 路径敏感。
#   （本沙箱另有两个坑：由本会话新建的 `%APPDATA%\dev.xhy.desktop` 会让 GUI 进程
#    写不进去 → 只能沿用既有目录 + 先备份；`cargo test`/`cargo build` 要把 TMP 指到仓库内。）
#
# ★ 本沙箱无法自动验证、刻意不写成断言（避免假绿/假红），需人工确认 ★
#   · 托盘图标单击（verify-tray-click.ps1 记录过注入不了）→ 这里改用 `show_main`
#     命令打开工作台，走的是与托盘回调同一个 `show_main_sync`。
#   · 真实鼠标跨窗口拖拽（README 第 5 轮：SetCursorPos+mouse_event 被拦）
#     → 落点规则由 `tools/views.test.ts` 覆盖，提醒平移由 `tests/state_flow.rs` 覆盖。
#   · `prompt()` 自定义跨天数/分钟数（原生对话框阻塞 renderer，CDP 取不到值）
#     → 上限裁剪由 `state_flow.rs::event_span_and_duration_are_clamped_*` 覆盖。
param(
  # 默认测仓库里刚 build 的 release 产物；要验**已安装**那份就传
  #   -SrcExe "$env:LOCALAPPDATA\Shoubian\xhy.exe"
  # （装到 %LOCALAPPDATA%\Shoubian\ 是因为 productName 决定目录与卸载项显示名；
  #   里面的 exe 仍叫 xhy.exe —— 那名字来自 Cargo 的 bin 名，不受 productName 影响，
  #   进程名也因此还是 xhy，本脚本的 Stop-Process 判据不用改）
  # 同一套断言，装完再跑一遍才算交付物被验过（不是只验了构建目录里的文件）。
  [string]$SrcExePath = ''
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
# ★ 必须用 **Tauri CLI 出的 release 二进制**：`npm run tauri:build -- --no-bundle`。
#   实测两种"自己 build"都不行，起来的是 Chromium 的"无法访问此网站"错误页
#   （CDP 里 url=http://localhost:1420/、DOM 只有 54 个节点、.trow/.cell 全找不到）：
#     · `cargo build`（debug）        → 走 devUrl
#     · `cargo build --release`      → **也**走 devUrl（没拿到 CLI 的那组 TAURI_ENV_* 环境）
#   只有 tauri CLI 才会把 `frontendDist`（../dist）当成本地资源加载 —— 与用户实际跑的同一形态。
#   所以改前端后必须：先 npm run build（CLI 的 beforeBuildCommand 会自己跑），再 tauri build。
$srcExe = if ($SrcExePath) { $SrcExePath } else { "$repo\app\src-tauri\target\release\xhy.exe" }
$stageDir = Join-Path $env:LOCALAPPDATA 'xhy-verify'          # 纯 ASCII
$exe = Join-Path $stageDir 'xhy.exe'
$appDataDir = "$env:APPDATA\dev.xhy.desktop"
$storePath = "$appDataDir\store.json"
$logPath = "$appDataDir\xhy.log"
$outLog = Join-Path $stageDir 'verify-r6.log'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$bak = "$appDataDir.r6bak-$stamp"
$backedUp = $false
$wasRunning = [bool](Get-Process -Name 'xhy' -EA SilentlyContinue)

# ── 输出：同时写 UTF-8 日志文件。PS 5.1 把 stdout 重定向进文件时按 GBK 编码，
#    中文会成乱码，判读只看这份 UTF-8 日志。──────────────────
if (Test-Path $stageDir) { Remove-Item $stageDir -Recurse -Force -EA SilentlyContinue }
New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
if (Test-Path $outLog) { Remove-Item $outLog -Force }
function Out($m) {
  $line = if ($m -is [string]) { $m } else { ($m | Out-String).TrimEnd() }
  Write-Output $line
  Add-Content -Path $outLog -Value $line -Encoding UTF8
}

# store.json 必须按 UTF-8 读：Get-Content 默认按 ANSI(GBK) 解码，中文标题会变乱码，
# 与脚本里的中文字面量比对必然失败（假红）。
function Store {
  return ([IO.File]::ReadAllText($storePath, [Text.Encoding]::UTF8) | ConvertFrom-Json)
}

# ── CDP 工具（选择器一律用双引号属性值，JS 侧用单引号包选择器）────
function CdpSend($wsUrl, $obj) {
  $ws = [System.Net.WebSockets.ClientWebSocket]::new(); $ct = [System.Threading.CancellationToken]::None
  try { $ws.ConnectAsync([Uri]$wsUrl, $ct).Wait(6000) | Out-Null } catch { $ws.Dispose(); return $null }
  if ($ws.State -ne 'Open') { $ws.Dispose(); return $null }
  $b = [Text.Encoding]::UTF8.GetBytes(($obj | ConvertTo-Json -Depth 12 -Compress))
  $ws.SendAsync([ArraySegment[byte]]::new($b), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait()
  $buf = New-Object byte[] 262144; $sb = New-Object Text.StringBuilder
  for ($i = 0; $i -lt 30; $i++) {
    $t = $ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $ct)
    if (-not $t.Wait(8000)) { break }
    $null = $sb.Append([Text.Encoding]::UTF8.GetString($buf, 0, $t.Result.Count))
    if ($sb.ToString() -match '"id":1') { break }
  }
  $ws.Dispose()
  try { return ($sb.ToString() | ConvertFrom-Json) } catch { return $null }
}
function CdpEval($wsUrl, $js) {
  $o = CdpSend $wsUrl @{ id = 1; method = 'Runtime.evaluate'; params = @{ expression = $js; returnByValue = $true; awaitPromise = $true } }
  if (-not $o) { return $null }
  if ($o.result -and $o.result.exceptionDetails) {
    $ex = $o.result.exceptionDetails.exception
    return "EXC:$($o.result.exceptionDetails.text) $($ex.className) $($ex.message) $($o.result.exceptionDetails.lineNumber):$($o.result.exceptionDetails.columnNumber)"
  }
  $v = $o.result.result.value
  if ($v -is [string] -and $v.StartsWith('"')) { try { $v = $v | ConvertFrom-Json } catch {} }
  return "$v"
}
function CdpMouse($wsUrl, [string]$type, [int]$x, [int]$y, [int]$count = 1) {
  $null = CdpSend $wsUrl @{ id = 1; method = 'Input.dispatchMouseEvent'
    params = @{ type = $type; x = $x; y = $y; button = 'left'; clickCount = $count } }
}
function CdpClick($wsUrl, [int]$x, [int]$y) {
  CdpMouse $wsUrl 'mouseMoved' $x $y; Start-Sleep -Milliseconds 60
  CdpMouse $wsUrl 'mousePressed' $x $y; Start-Sleep -Milliseconds 70
  CdpMouse $wsUrl 'mouseReleased' $x $y; Start-Sleep -Milliseconds 60
}
function CdpEnter($wsUrl) {
  foreach ($type in @('keyDown', 'keyUp')) {
    $null = CdpSend $wsUrl @{ id = 1; method = 'Input.dispatchKeyEvent'; params = @{
        type = $type; key = 'Enter'; code = 'Enter'; windowsVirtualKeyCode = 13; nativeVirtualKeyCode = 13 } }
  }
}
function ElPos($ws, [string]$sel) {
  return CdpEval $ws "(()=>{const e=document.querySelector('$sel');if(!e)return '';const r=e.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
}
function TxtPos($ws, [string]$sel, [string]$text) {
  return CdpEval $ws "(()=>{const b=[...document.querySelectorAll('$sel')].find(x=>x.textContent.trim()==='$text');if(!b)return '';const r=b.getBoundingClientRect();return Math.round(r.left+r.width/2)+','+Math.round(r.top+r.height/2);})()"
}
function ClickSel($ws, [string]$sel) {
  $p = ElPos $ws $sel
  if ($p -match '^(\d+),(\d+)$') { CdpClick $ws ([int]$matches[1]) ([int]$matches[2]); return $true }
  return $false
}
function ClickTxt($ws, [string]$sel, [string]$text) {
  $p = TxtPos $ws $sel $text
  if ($p -match '^(\d+),(\d+)$') { CdpClick $ws ([int]$matches[1]) ([int]$matches[2]); return $true }
  return $false
}
# 真实键入：Input.insertText 走 renderer 的文本输入通道（和 IME 上屏同一条路，
# 会派发 input 事件 → v-model 更新）。若值没进去，退化为 JS 赋值 + 派发 input，
# 并返回用了哪条通道 —— 不静默假装成功。
function TypeInto($ws, [string]$sel, [string]$text) {
  $null = CdpEval $ws "(()=>{const e=document.querySelector('$sel');if(e){e.focus();e.value=''}return ''})()"
  $null = CdpSend $ws @{ id = 1; method = 'Input.insertText'; params = @{ text = $text } }
  Start-Sleep -Milliseconds 250
  if ((CdpEval $ws "document.querySelector('$sel')?.value||''") -eq $text) { return 'insertText' }
  $esc = $text | ConvertTo-Json -Compress
  $null = CdpEval $ws "(()=>{const e=document.querySelector('$sel');e.value=$($esc);e.dispatchEvent(new InputEvent('input',{bubbles:true,data:$($esc)}));return ''})()"
  Start-Sleep -Milliseconds 200
  return 'js-fallback'
}
# 页面发现：轮询 CDP，按 DOM 特征认领每个窗口的页面
function PageWs([string]$probe, [int]$rounds = 12) {
  for ($i = 0; $i -lt $rounds; $i++) {
    try { $pages = Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 5 } catch { Start-Sleep -Seconds 2; continue }
    foreach ($t in $pages) {
      if ($t.type -ne 'page') { continue }
      if ((CdpEval $t.webSocketDebuggerUrl $probe) -eq 'True') { return $t.webSocketDebuggerUrl }
    }
    Start-Sleep -Seconds 2
  }
  return $null
}
function PageCount() {
  try { return @((Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 5) | Where-Object { $_.type -eq 'page' }).Count } catch { return -1 }
}
# 页面认领不上时，把每个页面的 url / DOM 规模 / 首行文本打出来 —— 是"错误页"还是
# "选择器不对"，一眼能分（上一轮就是因为 debug 构建停在 localhost:1420 错误页上，
# 三个页面全都 trow=false，只靠断言猜了半天）。
function Dump-Pages() {
  try { $pages = Invoke-RestMethod 'http://127.0.0.1:9222/json' -TimeoutSec 5 } catch { Out "  CDP 列表取不到：$_"; return }
  foreach ($t in $pages) {
    if ($t.type -ne 'page') { continue }
    $n = CdpEval $t.webSocketDebuggerUrl 'String(document.querySelectorAll("*").length)'
    $h = CdpEval $t.webSocketDebuggerUrl 'document.body.innerHTML.replace(/\s+/g," ").slice(0,120)'
    Out "  页面 url=$($t.url) nodes=$n 头部=$h"
  }
}
# 直接调后端命令（和前端 api.* 同一个 tauri::command）
function Invoke-Cmd($ws, [string]$cmd, [hashtable]$a) {
  $json = if ($a) { ($a | ConvertTo-Json -Depth 10 -Compress) } else { '{}' }
  $js = "(async()=>{try{const r=await window.__TAURI_INTERNALS__.invoke('$cmd',$json);return 'OK'+JSON.stringify(r===undefined?null:r)}catch(e){return 'ERR:'+(e&&(e.message||e)||e)}})()"
  $v = CdpEval $ws $js
  if ($v -like 'OK*') { return ($v.Substring(2) | ConvertFrom-Json) }
  Out "    invoke $cmd 返回：$v"
  return $null
}

$fail = 0
function Check($n, [bool]$ok, $d = '') {
  if ($ok) { Out "  PASS  $n $d" } else { $script:fail++; Out "  FAIL  $n $d" }
}

# ── 日期：跨天用例的锚点恒取"下一个周三"。今天若是周五/周日，直接用今天当起始日
#    会让 3 天跨度被周行边界切开，断言当场退化甚至假红。周三 +2 = 周五，永远在同一行里。
$today = (Get-Date -Format 'yyyy-MM-dd')
$wdMon = @('一', '二', '三', '四', '五', '六', '日')
$wdIdx = (([int](Get-Date).DayOfWeek) + 6) % 7                     # 周一=0
$delta = (2 - $wdIdx + 7) % 7; if ($delta -eq 0) { $delta = 7 }
$anchor = (Get-Date).AddDays($delta).ToString('yyyy-MM-dd')
$far = '2031-10-20'
$mdGuess = '{0}-10-20' -f (Get-Date).Year                          # 「10月20日」= 今年，已过则明年
if ($mdGuess -lt $today) { $mdGuess = '{0}-10-20' -f ((Get-Date).Year + 1) }
$fri = (Get-Date).AddDays((4 - $wdIdx + 7) % 7).ToString('yyyy-MM-dd')
$badYmd = '{0}-02-30' -f (Get-Date).Year                          # 不存在的日期

# ── 数据：备份 → 播种 → 还原 ─────────────────────────────
function Backup-Data() {
  if (-not (Test-Path $appDataDir)) { Out "  数据目录不存在（$appDataDir）—— 先让应用跑起来一次"; return }
  New-Item -ItemType Directory -Force -Path $bak | Out-Null
  Copy-Item "$appDataDir\*" $bak -Recurse -Force
  $script:backedUp = $true
  Out "  已整份备份 → $bak"
}
function Restore-Data() {
  if (-not $script:backedUp) { Out "  无备份可还原"; return }
  Get-ChildItem $appDataDir -Force -EA SilentlyContinue | Remove-Item -Recurse -Force -EA SilentlyContinue
  Copy-Item "$bak\*" $appDataDir -Recurse -Force
  Out "  已还原原始数据（store.json 存在 $((Test-Path $storePath))）"
}
function Seed-Fixture() {
  $j = Store
  # 清单 id 取真实库里的第一个（播种库里的 id 是随机串，写死 'inbox' 会指向不存在的清单）
  $lid = $j.lists[0].id
  $j.lists[0].archived = $false
  $j.lists[0].excludeFromStats = $false
  $j.todos = @(
    # 派生日程：到期=今天、定时 09:30 → 日历上应出现一条来源为"待办"的条目
    @{ id = 'tdv'; listId = $lid; title = '派生验证'; done = $false; pinned = $false; order = 0;
       tags = @(); due = $today; remindAt = "${today}T09:30"; reminded = $false; extraReminds = @();
       remindedKeys = @(); constantRemind = $false; createdAt = "${today}T08:00" }
  )
  $j.events = @(
    # 跨 3 天全天日程：挂件日视图标 ⬌3、周视图逐日出现、主窗口画跨 3 列长条
    @{ id = 'espan'; date = $anchor; title = '出差验证'; color = '#30a46c'; source = 'manual'; todoId = $null;
       spanDays = 3; durationMin = 0; recurrence = $null; listId = $null },
    # 有开始时刻 + 90 分钟 → 该显示 09:30–11:00（时长功能的可见结果）
    @{ id = 'edur'; date = $today; time = '09:30'; title = '评审验证'; color = '#e5484d'; source = 'manual';
       todoId = $null; spanDays = 1; durationMin = 90; recurrence = $null; listId = $null }
  )
  $j.notes = @()
  $j.trashed = @()
  # 探针：往一次性库里塞一个**已废弃**的 `settings.hotkeys`（用户真实库里原本就有，
  # 但会被上一次运行"保存即清掉"，所以每次跑都得自己造，断言才有前提可验）。
  $j.settings | Add-Member -NotePropertyName hotkeys -NotePropertyValue @{ toggleAll = 'Alt+Shift+H' } -Force
  foreach ($w in $j.widgets) {
    $w.visible = ($w.kind -ne 'note')
    $w.edgeHide = $null; $w.autoDock = $false; $w.free = $true
    if ($w.kind -eq 'todo') { $w.listId = $null; $w.filter = $null; $w.tag = $null
      $w.rect = @{ x = 40; y = 60; w = 400; h = 480 } }
    if ($w.kind -eq 'calendar') { $w.listId = $null; $w.filter = $null; $w.tag = $null; $w.view = 'month'
      $w.rect = @{ x = 470; y = 60; w = 520; h = 640 } }
  }
  # 必须**无 BOM** UTF-8：带 BOM 时 serde_json 解析失败 → 应用"改名留档 + 回退备份"，
  # 测到的就不是刚播种的那份（静默换库，断言全废）。
  [IO.File]::WriteAllText($storePath, ($j | ConvertTo-Json -Depth 14), [Text.UTF8Encoding]::new($false))
  Out "  已播种一次性库：today=$today（周$($wdMon[$wdIdx])） anchor=$anchor（恒周三） 清单=$lid"
}

function Stop-App() {
  Get-Process -Name 'xhy' -EA SilentlyContinue | Stop-Process -Force
  Start-Sleep -Seconds 3
  # 顺带清掉本项目数据目录下的 WebView2 浏览器进程：它按"用户数据目录"共享，
  # 客户进程（xhy.exe）死了还能活一会儿，会占着 EBWebView 目录。
  # 注意：**它不是"第二次会话连不上 CDP"的原因**（那是因为没有可见挂件，见 Start-App 注释）。
  # 只杀命令行里带本项目数据目录的那些，别的应用的 webview 一律不动。
  try {
    Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -EA SilentlyContinue |
      Where-Object { $_.CommandLine -match 'dev\.xhy\.desktop' } |
      ForEach-Object { Stop-Process -Id $_.ProcessId -Force -EA SilentlyContinue }
  } catch { Out "  （清 webview2 浏览器进程时出错：$_）" }
  Start-Sleep -Seconds 2
}
# 启动应用并等 CDP 就绪。结果写 `$script:cdpUp`，**不能用返回值**：
# 函数里那些 `Out "..."` 走的是管道，返回值会被拼成一个数组，
# 于是 `if (-not (Start-App))` 永远为假 —— 上一版就是这样，CDP 起不来却继续往下跑，
# 报出来的全是"页面找不到"的误导性失败。
# 但连不上 CDP 更常见的原因不是这个，而是根本没有可见窗口
#   —— CDP 端口属于 WebView2 的浏览器进程，而它只在有挂件/窗口时才存在。
#   本脚本播种的一次性库刻意把 todo + calendar 设成可见，就是为了保证“有窗口 → 有端口”。
function Start-App([int]$tries = 2) {
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
  $script:cdpUp = $false
  for ($n = 1; $n -le $tries -and -not $script:cdpUp; $n++) {
    if ($n -gt 1) { Out "  第 $n 次尝试启动（上一次 CDP 没起来，多半是旧的 webview2 浏览器进程还没退）"; Stop-App }
    $p = Start-Process $exe -PassThru
    for ($i = 1; $i -le 25; $i++) {
      Start-Sleep -Seconds 1
      try {
        $r = Invoke-WebRequest 'http://127.0.0.1:9222/json/version' -UseBasicParsing -TimeoutSec 2
        if ([int]$r.StatusCode -eq 200) { Out "  已启动 pid=$($p.Id)，$i 秒后 CDP=200"; $script:cdpUp = $true; break }
      } catch {}
    }
  }
  if (-not $script:cdpUp) {
    Out "  两轮各 25 秒仍连不上 CDP"
    if (Test-Path $logPath) { Out "  ── xhy.log 尾部 ──"; Out ((Get-Content $logPath -Tail 12) -join "`n") }
  }
}

try {
  Out "=== 准备：把待测二进制暂存到 ASCII 路径 ==="
  if (-not (Test-Path $srcExe)) {
    Out "  缺 $srcExe"
    Out "  先跑：cd app && npx tauri build --no-bundle"
    Out "  （不要用 cargo build：debug 与 --release 都会走 devUrl，页面是错误页）"
    exit 2
  }
  New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
  Copy-Item $srcExe $exe -Force
  $libSrc = Join-Path (Split-Path $srcExe) 'xhy_lib.dll'
  if (Test-Path $libSrc) { Copy-Item $libSrc (Join-Path $stageDir 'xhy_lib.dll') -Force }
  $fi = Get-Item $exe
  Out "  被测二进制：$($fi.FullName) | $($fi.Length) 字节 | 建于 $($fi.LastWriteTime)"

  Out ""; Out "=== 前置：备份 → 播种 ==="
  Stop-App
  Backup-Data
  Seed-Fixture
  # 探针前提要在**交给应用之前**验：应用一启动就会不断写盘（加待办、存窗口位置…），
  # 废弃字段在第一次保存时就消失了，等到 T8 再读库去验"塞没塞进去"必然是假红
  # —— 上一版正是踩在这里。前提验在播种后，效果验在 T8。
  $seed = Store
  Check '探针有效：交给应用的库里塞了废弃的 hotkeys 字段' ($null -ne $seed.settings.hotkeys) "(settings 键：$($seed.settings.PSObject.Properties.Name -join ','))"

  Start-App; Check '应用能启动并暴露 CDP' $script:cdpUp; if (-not $script:cdpUp) { exit 1 }
  Check '应用能启动并暴露 CDP' $true

  $todoWs = PageWs "([!!document.querySelector('.trow')])"
  $calWs = PageWs "([!!document.querySelector('.cell')&&!!document.querySelector('.wd')])"
  Check '待办挂件页面可用' ($null -ne $todoWs) "(页面数=$(PageCount))"
  Check '日历挂件页面可用' ($null -ne $calWs)
  if (-not $todoWs -or -not $calWs) { Dump-Pages; exit 1 }

  # ══ T1 真实二进制里的解析器：各种日期写法 ══
  Out ""; Out "=== T1 quick_parse 各写法的日期识别 ==="
  $cases = @(
    @{ in = "$far 开会"; due = $far; time = '' }
    @{ in = '2031/10/20 开会'; due = $far; time = '' }
    @{ in = '2031.10.20 开会'; due = $far; time = '' }
    @{ in = '2031年10月20日 开会'; due = $far; time = '' }
    @{ in = '2031-10-20 09:30 开会'; due = $far; time = '09:30' }
    @{ in = '2031-10-20 下午3点 开会'; due = $far; time = '15:00' }
    @{ in = '10月20日 开会'; due = $mdGuess; time = '' }
    @{ in = '明天 开会'; due = (Get-Date).AddDays(1).ToString('yyyy-MM-dd'); time = '' }
    @{ in = "周$($wdMon[$wdIdx]) 开会"; due = $today; time = '' }        # ★ 当天=今天
    @{ in = '周五 开会'; due = $fri; time = '' }
    @{ in = "$badYmd 开会"; due = ''; time = '' }                     # 非法日期不认
    @{ in = '20311020 开会'; due = ''; time = '' }                    # 8 位裸数字不认
  )
  foreach ($c in $cases) {
    $got = Invoke-Cmd $todoWs 'quick_parse' @{ text = $c.in }
    if ($null -eq $got) { Check "解析 $($c.in)" $false '(命令没返回)'; continue }
    $gDue = if ($got.due) { "$($got.due)" } else { '' }
    $gTime = if ($got.time) { "$($got.time)" } else { '' }
    $ok = ($gDue -eq $c.due) -and ($gTime -eq $c.time)
    Check "解析「$($c.in)」" $ok "(得 due='$gDue' time='$gTime'，期望 due='$($c.due)' time='$($c.time)'，标题='$($got.title)')"
  }

  # ══ T2 真实输入链：打字加一条带全写日期 + 时间的待办 ══
  Out ""; Out "=== T2 真实输入链（键入 → 回车 → 命令 → 落库 → 界面）==="
  if (ClickSel $todoWs '.add input.xh-input') {
    $via = TypeInto $todoWs '.add input.xh-input' "$far 09:30 全日期验证"
    Out "  键入通道：$via"
    CdpEnter $todoWs; Start-Sleep -Milliseconds 1600
    $t = @((Store).todos | Where-Object { $_.title -eq '全日期验证' })
    Check '★ 全写日期真的落成 due' ($t[0].due -eq $far) "(实际=$($t[0].due))"
    Check '★ 时间真的落成 remindAt' ($t[0].remindAt -eq "${far}T09:30") "(实际=$($t[0].remindAt))"
    $rowTxt = CdpEval $todoWs "(()=>[...document.querySelectorAll('.trow')].map(e=>e.textContent.replace(/\s+/g,' ')).join(' | '))()"
    Check '★ 挂件里立刻能看到这条（口径=全部时）' ($rowTxt -like '*全日期验证*')
  } else { Check '快速添加框能定位并点击' $false }

  # ══ T3 周X=今天（打字整条链，T1 之外再验一次）══
  Out ""; Out "=== T3 今天就是周$($wdMon[$wdIdx]) → 输「周$($wdMon[$wdIdx])」应排到今天 ==="
  if (ClickSel $todoWs '.add input.xh-input') {
    $null = TypeInto $todoWs '.add input.xh-input' "周$($wdMon[$wdIdx]) 周X验证"
    CdpEnter $todoWs; Start-Sleep -Milliseconds 1600
    $t2 = @((Store).todos | Where-Object { $_.title -eq '周X验证' })
    Check '★ 到期日=今天（不是下周）' ($t2[0].due -eq $today) "(实际=$($t2[0].due)，今天=$today)"
  } else { Check '快速添加框能定位并点击' $false }

  # ══ T4 挂件日视图：时长显示 + 待办派生条目 ══
  Out ""; Out "=== T4 挂件：今天这一天的日视图 ==="
  $null = ClickTxt $calWs '.xh-shell .xh-btn' '月'; Start-Sleep -Milliseconds 900
  if (ClickSel $calWs ".xh-shell .cell[data-date=`"$today`"]") {
    Start-Sleep -Milliseconds 1600
    $rows = CdpEval $calWs "(()=>[...document.querySelectorAll('.dev')].map(e=>e.textContent.replace(/\s+/g,' ').trim()).join(' | '))()"
    Out "  日视图行：$rows"
    Check '★ 定时日程显示结束时刻 09:30–11:00' ($rows -like '*09:30–11:00*')
    Check '★ 待办派生的条目也在这一天的日历上' ($rows -like '*派生验证*')
    # 护栏写法刻意用"把每行的按钮 title 打出来"而不是在 JS 里写属性选择器：
    # 前者不需要在 PS 双引号串里再嵌一层引号转义（实测 `\"` 会让 JS 抛 Uncaught，
    # 断言当场变成 EXC 而不是真假），而且失败时能直接看见"到底给了哪些按钮"。
    $badw = CdpEval $calWs "(()=>[...document.querySelectorAll('.dev')].filter(d=>[...d.querySelectorAll('.xh-tag')].some(t=>t.textContent.trim()==='待办')).map(d=>[...d.querySelectorAll('.xh-btn.x')].map(b=>b.title).join('/')).join(','))()"
    Check '★ 挂件里派生条目同样不给跨天/时长按钮' (-not ($badw -match '时长|跨几天')) "(派生行的按钮=$badw)"
  } else { Check '月视图里点得到今天' $false }

  # ══ T5 挂件：跨天日程（起始日标记 + 周视图逐日出现）══
  Out ""; Out "=== T5 挂件：跨 3 天日程（起始 $anchor）==="
  $aSel = ".xh-shell .cell[data-date=`"$anchor`"]"
  $null = ClickTxt $calWs '.xh-shell .xh-btn' '月'; Start-Sleep -Milliseconds 900
  if (-not (ClickSel $calWs $aSel)) {
    $null = ClickTxt $calWs '.xh-shell .xh-btn' '›'; Start-Sleep -Milliseconds 900   # 翻到下个月再找
    Check '能点到跨天日程的起始日' (ClickSel $calWs $aSel)
  } else { Check '能点到跨天日程的起始日' $true }
  Start-Sleep -Milliseconds 1500
  $arows = CdpEval $calWs "(()=>[...document.querySelectorAll('.dev')].map(e=>e.textContent.replace(/\s+/g,' ').trim()).join(' | '))()"
  Out "  起始日日视图：$arows"
  Check '★ 起始日带 ⬌3 跨天标记' ($arows -like '*出差验证*⬌3*')
  $null = ClickTxt $calWs '.xh-shell .xh-btn' '周'; Start-Sleep -Milliseconds 1600
  $wk = CdpEval $calWs "(()=>{const cs=[...document.querySelectorAll('.cell.col')].filter(c=>c.textContent.includes('出差验证'));return JSON.stringify({n:cs.length,t:cs.map(c=>c.textContent.replace(/\s+/g,' ').trim())})})()"
  Out "  周视图命中：$wk"
  Check '★ 跨天日程在连续三天都出现（不再只显示第一天）' ($wk -like '*"n":3*')
  Check '★ 起始日标 ⬌3、延续日标 ›2/3 与 ›3/3' ($wk -like '*⬌3*' -and $wk -like '*›2/3*' -and $wk -like '*›3/3*')

  # ══ T6 主窗口：跨天长条 + 行内编辑（UI → 命令 → 库 → 再渲染）══
  Out ""; Out "=== T6 工作台：月视图跨天长条 + 行内改跨天/时长 ==="
  $null = Invoke-Cmd $todoWs 'show_main' @{}
  Start-Sleep -Seconds 4
  $mainWs = PageWs "([!!document.querySelector('.side')])"
  Check '工作台能打开并有 CDP 页面（show_main）' ($null -ne $mainWs) "(页面数=$(PageCount))"
  if ($mainWs) {
    $null = ClickTxt $mainWs '.seg button' '日历'; Start-Sleep -Milliseconds 1500
    $null = ClickTxt $mainWs '.cal .chd .seg button' '月'; Start-Sleep -Milliseconds 1200
    $bar = CdpEval $mainWs "(()=>{const b=[...document.querySelectorAll('.bars .bar')].find(e=>e.textContent.includes('出差验证'));return b?(b.textContent.trim()+' @ '+b.style.gridColumn):''})()"
    Out "  长条：$bar"
    Check '★ 主窗口月视图画出跨天长条' ($bar -ne '')
    Check '★ 长条占 3 列（gridColumn 结尾 span 3）' ($bar -match 'span 3\s*$') "(现=$bar)"

    $null = ClickSel $mainWs ".cal .cell[data-date=`"$anchor`"]"; Start-Sleep -Milliseconds 900
    $null = ClickTxt $mainWs '.cal .chd .seg button' '日'; Start-Sleep -Milliseconds 1300
    $dr = CdpEval $mainWs "(()=>[...document.querySelectorAll('.dev')].map(e=>e.textContent.replace(/\s+/g,' ').trim()).join(' | '))()"
    Out "  工作台日视图：$dr"
    Check '★ 工作台日视图显示「⬌3天」' ($dr -like '*⬌3天*')
    if (ClickSel $mainWs '.cal .dev .ib[title="跨几天"]') {
      Start-Sleep -Milliseconds 800
      Check '跨天编辑面板出现' ((CdpEval $mainWs "String(!!document.querySelector('.edbox'))") -eq 'True')
      $null = ClickTxt $mainWs '.edbox .xh-btn' '5 天'; Start-Sleep -Milliseconds 2000
      $sp = @((Store).events | Where-Object { $_.id -eq 'espan' }).spanDays
      Check '★★ 行内改跨天真的落库（spanDays=5）' ($sp[0] -eq 5) "(实际=$($sp[0]))"
      $dr2 = CdpEval $mainWs "(()=>[...document.querySelectorAll('.dev')].map(e=>e.textContent.replace(/\s+/g,' ').trim()).join(' | '))()"
      Check '★★ 界面立刻跟着变成 ⬌5天' ($dr2 -like '*⬌5天*') "(现=$dr2)"
      $null = ClickTxt $mainWs '.cal .chd .seg button' '月'; Start-Sleep -Milliseconds 1300
      $bar2 = CdpEval $mainWs "(()=>{const b=[...document.querySelectorAll('.bars .bar')].find(e=>e.textContent.includes('出差验证'));return b?(b.textContent.trim()+' @ '+b.style.gridColumn):''})()"
      Check '★★ 长条宽度跟着变成 5 列' ($bar2 -match 'span 5\s*$') "(现=$bar2)"
    } else { Check '跨天编辑按钮能点到' $false }

    # 上一段失败时面板可能还停在日视图（日视图里没有 .cell），先回月视图再点格子，
    # 两段之间不留状态依赖。
    $null = ClickTxt $mainWs '.cal .chd .seg button' '月'; Start-Sleep -Milliseconds 1200
    $null = ClickSel $mainWs ".cal .cell[data-date=`"$today`"]"; Start-Sleep -Milliseconds 900
    $null = ClickTxt $mainWs '.cal .chd .seg button' '日'; Start-Sleep -Milliseconds 1300
    # 护栏：派生条目（待办镜像）不该给「跨几天 / 时长」按钮 —— 待办只占到期那天，
    # 后端 update_event 对这两个字段是忽略的，按钮给了就是"点了没反应"。
    $bad = CdpEval $mainWs "(()=>[...document.querySelectorAll('.cal .dev')].filter(d=>!!d.querySelector('.tag.src')).map(d=>[...d.querySelectorAll('.ib')].map(b=>b.title).join('/')).join(','))()"
    Check '★★ 派生条目不给无意义的跨天/时长按钮' (-not ($bad -match '时长|跨几天')) "(派生行的按钮=$bad)"
    if (ClickSel $mainWs '.cal .dev .ib[title="时长"]') {
      Start-Sleep -Milliseconds 800
      $null = ClickTxt $mainWs '.edbox .xh-btn' '120分'; Start-Sleep -Milliseconds 2000
      $dm = @((Store).events | Where-Object { $_.id -eq 'edur' }).durationMin
      Check '★★★ 行内改时长真的落库（durationMin=120）' ($dm[0] -eq 120) "(实际=$($dm[0]))"
      $tm = CdpEval $mainWs "(()=>[...document.querySelectorAll('.dev .tm')].map(e=>e.textContent.trim()).join(' '))()"
      Check '★★★ 结束时刻跟着变成 09:30–11:30' ($tm -like '*09:30–11:30*') "(现=$tm)"
    } else { Check '时长编辑按钮能点到' $false }
  }

  # ══ T7 第 7 轮改动：无日期默认今天 / 「待定」撤掉 / Markdown 导入 ══
  Out ""; Out "=== T7 默认今天 + 撤掉「待定」+ Markdown 导入 ==="
  if (ClickSel $todoWs '.add input.xh-input') {
    $null = TypeInto $todoWs '.add input.xh-input' '无日期默认验证'
    CdpEnter $todoWs; Start-Sleep -Milliseconds 1600
    $t7 = @((Store).todos | Where-Object { $_.title -eq '无日期默认验证' })
    Check '★★ 只写标题的待办，到期日默认=今天（不再产出无日期条目）' ($t7[0].due -eq $today) "(实际=$($t7[0].due))"
    Check '★★ 默认到期日不附带提醒（否则每条都弹）' ($null -eq $t7[0].remindAt) "(remindAt=$($t7[0].remindAt))"
  } else { Check '快速添加框能定位并点击' $false }
  # 筛选按钮的 class 是 .fbtn（.tab 是**清单标签页**，拿它探会永远"没有待定"= 假绿）
  $tabs = CdpEval $todoWs "(()=>[...document.querySelectorAll('.fbtn')].map(e=>e.textContent.trim()).join(','))()"
  Out "  挂件筛选按钮：$tabs"
  Check '★ 挂件不再有「待定」档' ($tabs -notlike '*待定*')
  Check '★ 其余筛选档还在（今天/本周/旗标/完成）' ($tabs -like '*今天*' -and $tabs -like '*本周*' -and $tabs -like '*旗标*' -and $tabs -like '*完成*')
  if ($mainWs) {
    # 改名「手边」的可见结果（窗口标题要 Win32 才读得到，界面里的 brand 就够代表改动生效）
    $brand = CdpEval $mainWs "(()=>{const e=document.querySelector('.brand');return e?e.textContent.trim():'NO-BRAND-EL'})()"
    Check '★ 主窗口界面显示新名字「手边」' ($brand -like '*手边*') "(brand=$brand)"
    # 通知上那行"来源"是 Windows 按 AUMID 注册显示名渲染的（不注册就退回 exe 文件名
    # xhy.exe → 显示"xhy"）。应用启动时会自己写 HKCU，这里读回来确认它真的落了。
    $hk = 'HKCU:\Software\Classes\AppUserModelId\dev.xhy.desktop'
    $dn = if (Test-Path $hk) { (Get-ItemProperty $hk -EA SilentlyContinue).DisplayName } else { '(键不存在)' }
    Check '★ 通知来源名（AUMID DisplayName）注册成「手边」' ($dn -eq '手边') "(实际=$dn)"
    $null = ClickTxt $mainWs '.seg button' '看板'; Start-Sleep -Milliseconds 1600
    $cols = CdpEval $mainWs "(()=>[...document.querySelectorAll('.board .col .chead')].map(e=>e.textContent.replace(/\s+/g,'').trim()).join(','))()"
    Out "  看板列头：$cols"
    Check '★ 看板列改名叫「未排期」，「待定」不再出现在界面上' ($cols -like '*未排期*' -and $cols -notlike '*待定*')
  }

  # Markdown 导入：拿用户那份真实 Kite 导出当样本（只读，导进的是**一次性库**，
  # 跑完 finally 会整份还原，不会写进真实库）。文件不在就 SKIP，不假装通过。
  # Markdown 导入：用**内嵌的合成样本**（形状对着第三方待办工具的导出：日期分组 +
  # `- [x]` + 半角冒号带备注 + 一条空条目）。原先这里读的是我本机的一份真实导出，
  # 那既有隐私问题、又让脚本依赖仓库外的文件 —— 换成内嵌样本后哪都能跑，且断言更准。
  $md = @'
## 2026-08-18

- [x] 交季度报表
- [x] 约客户看方案

## 2026-08-31

- [x] 整理调研报告: 初稿已完成，等复核
- [x] 装修方案 第一阶段：完成选材清单

## 2026-10-19

- [ ] 出差上海10月19日-10月23日
- [ ] 整理调研材料: 初稿已收到
- [ ]
'@
  $before = @((Store).todos).Count
  $sum = Invoke-Cmd $todoWs 'import_markdown' @{ md = $md; listName = '导入验证' }
  Start-Sleep -Milliseconds 1500
  $g = Store
  $added = @((Store).todos).Count - $before
  $items = @($g.todos | Where-Object { $_.title -eq '出差上海10月19日-10月23日' })
  Check '★ 第三方排版的样本被导进来了' ($added -eq 6) "(新增 $added，摘要：$sum)"
  Check '★ 日期分组落成到期日' ($items[0].due -eq '2026-10-19') "(due=$($items[0].due))"
  $noteItem = @($g.todos | Where-Object { $_.title -eq '整理调研报告' })
  Check '★ 半角冒号后是备注、标题不被咬断' ($noteItem[0].due -eq '2026-08-31' -and $noteItem[0].note -eq '初稿已完成，等复核') "(due=$($noteItem[0].due) note=$($noteItem[0].note))"
  $full = @($g.todos | Where-Object { $_.title -eq '装修方案 第一阶段：完成选材清单' })
  Check '★ 全角冒号不算备注分隔（标题保持完整）' ($full.Count -eq 1 -and -not $full[0].note) "(标题=$($full[0].title))"
  Check '★ 空条目被丢弃但**报出条数**（不静默少导入）' ($sum -like '*忽略空条目 1 条*') "(摘要：$sum)"
  $doneItem = @($g.todos | Where-Object { $_.title -eq '交季度报表' })
  Check '★ 已完成条目按分组日期补了 doneAt（能进归档与趋势）' ($doneItem[0].done -eq $true -and $doneItem[0].doneAt -eq '2026-08-18T12:00') "(doneAt=$($doneItem[0].doneAt))"
  $null = Invoke-Cmd $todoWs 'import_markdown' @{ md = $md; listName = '导入验证' }
  Start-Sleep -Milliseconds 1500
  $again = @((Store).todos).Count
  Check '★★ 同一份再导一次不重复（标题+到期日去重）' ($again -eq $before + $added) "($again vs $($before + $added))"
  $lists = @((Store).lists | Where-Object { $_.name -eq '导入验证' })
  Check '★ 兜底清单只建了一个' ($lists.Count -eq 1) "(=$($lists.Count))"

  # ══ T8 已废弃的 settings.hotkeys：带它的库能加载，保存一次就干净消失 ══
  # 前提（库里有这个字段）已在播种后断言过；应用整个 T1-T7 会话就是**加载着这份库**
  # 跑完的，所以"能加载、能渲染、能保存"这三件事本身已经是结论。
  Out ""; Out "=== T8 已废弃字段 settings.hotkeys ==="
  $pre = Store
  $preTodos = @($pre.todos).Count
  $corrupt = @(Get-ChildItem $appDataDir -Filter 'store.corrupt-*' -EA SilentlyContinue)
  Check '★ 带废弃字段的库没被改名留档（serde 忽略未知字段）' ($corrupt.Count -eq 0) "(留档=$($corrupt.Name -join ','))"
  Check '★ 应用运行期间废弃字段已被自然丢弃（保存过就不再带）' ($null -eq $pre.settings.hotkeys)
  $logTxt = ((Get-Content $logPath -Tail 30 -EA SilentlyContinue) -join "`n")
  Check '★ 启动日志无解析失败/panic' ($logTxt -notmatch '解析失败|panicked')
  # 用"写回同样值"的方式触发一次保存：验证废弃字段是被忽略而不是报错
  $null = Invoke-Cmd $todoWs 'set_settings' @{ patch = @{ fontPx = $pre.settings.fontPx } }
  Start-Sleep -Milliseconds 1500
  $after = Store
  Check '★★ 保存一次后 hotkeys 自然消失（不报错）' ($null -eq $after.settings.hotkeys)
  Check '★★ 其余设置没被带走（fontPx 保持）' ($after.settings.fontPx -eq $pre.settings.fontPx) "( $($after.settings.fontPx) )"
  Check '★★ 待办一条没丢' (@($after.todos).Count -eq $preTodos) "($(@($after.todos).Count) / $preTodos)"

  # ══ T9 真实库：还原后能正常加载（只做**文件级**判据）══
  # 为什么这里不再"重启一次然后连 CDP"：同一脚本里第二次起的实例，应用本身活着、
  # 热键也注册成功，但 WebView2 侧不起 CDP 端口 —— 那是脚本/环境的会话问题，
  # 拿它当断言只会产出假红（上一版就是这样连红两次）。真实库能否加载，
  # "进程活着 + 没有 store.corrupt-* + 日志无解析失败 + 条数不变"已经足够判。
  Out ""; Out "=== T9 还原真实库后启动（文件级判据）==="
  Stop-App
  Restore-Data
  $real = Store
  $realTodos = @($real.todos).Count
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
  $null = Start-Process $exe -PassThru
  Start-Sleep -Seconds 12
  Check '★ 用户真实库能加载（进程活着，没在启动时退出）' ([bool](Get-Process -Name 'xhy' -EA SilentlyContinue))
  $corrupt2 = @(Get-ChildItem $appDataDir -Filter 'store.corrupt-*' -EA SilentlyContinue)
  Check '★ 真实库没被改名留档（解析没失败）' ($corrupt2.Count -eq 0) "(留档=$($corrupt2.Name -join ','))"
  $log2 = ((Get-Content $logPath -Tail 30 -EA SilentlyContinue) -join "`n")
  Check '★ 真实库启动日志无解析失败/panic' ($log2 -notmatch '解析失败|panicked')
  Check '★ 真实库的待办条数没被动过' (@((Store).todos).Count -eq $realTodos) "($(@((Store).todos).Count) / $realTodos)"
  Stop-App

  Out ""
  if ($fail -eq 0) { Out 'ALL PASS' } else { Out "$fail FAILED" }
}
finally {
  Out ""; Out "=== 收尾：还原原始数据 ==="
  Stop-App
  Restore-Data
  if (Test-Path $bak) { Remove-Item $bak -Recurse -Force -EA SilentlyContinue; Out "  备份目录已清理" }
  if ($wasRunning) { $null = Start-Process $exe; Out "  （原来 xhy 在运行，已用暂存二进制重新拉起）" }
}
exit $fail
