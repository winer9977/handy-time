# 验证弹层"点外面即关"：开 → 截图 → 点别处 → 再截图
param([int]$AnchorX = 290, [int]$AnchorY = 20, [int]$OutsideX = 150, [int]$OutsideY = 250)
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Runtime.InteropServices;
public class CA {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern bool EnumWindows(E cb, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, WC cb, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RC r);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate bool E(IntPtr h, IntPtr l);
    delegate bool WC(IntPtr h, IntPtr l);
    public struct RC { public int L,T,R,B; }
    public static void Aware(){SetProcessDPIAware();}
    public static IntPtr Find(uint pid, string want){ IntPtr hit=IntPtr.Zero;
        EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); if(p!=pid||!IsWindowVisible(h)) return true;
            RC r; GetWindowRect(h,out r); if((r.R-r.L)<40) return true;
            var tt=new StringBuilder(128); GetWindowTextW(h,tt,128);
            if(tt.ToString().Contains(want)){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    public static IntPtr Render(IntPtr parent){ IntPtr hit=IntPtr.Zero;
        EnumChildWindows(parent,(h,l)=>{ var cl=new StringBuilder(128); GetClassNameW(h,cl,128);
            if(cl.ToString()=="Chrome_RenderWidgetHostHWND"){hit=h;return false;} return true; }, IntPtr.Zero);
        return hit; }
    static IntPtr Lp(int x,int y){ return (IntPtr)((y<<16)|(x&0xFFFF)); }
    /** 完整的一次点击：down + up。冒泡 click 由 Chromium 合成 */
    public static void Click(IntPtr h,int x,int y){ PostMessage(h,0x0201,(IntPtr)1,Lp(x,y));
        System.Threading.Thread.Sleep(150); PostMessage(h,0x0202,IntPtr.Zero,Lp(x,y)); }
}
'@
[CA]::Aware()
$p = Get-Process -Name 'xhy' | Select-Object -First 1
$pid32 = [uint32]$p.Id
$note = [CA]::Find($pid32,'便签')
$r = [CA]::Render($note)
if ($r -eq [IntPtr]::Zero) { "无 webview 子窗口"; exit 1 }

$w = Split-Path -Parent $PSScriptRoot   # 仓库根，不写死本机路径
function Shot([string]$tag) {
  & "$w\tools\capture-windows.ps1" -OutDir "$w\.ca\$tag" -ProcessName 'xhy' 2>&1 | Out-Null
  $f = Get-ChildItem "$w\.ca\$tag" -Filter '*便签*' | Select-Object -First 1
  if ($f) { return $f.FullName }
  return $null
}

"1) 点 ⋯ 开弹层"
[CA]::Click($r, $AnchorX, $AnchorY)
Start-Sleep -Milliseconds 900
$open = Shot 'open'
"   -> $open"

"2) 点弹层外面（正文区 $OutsideX,$OutsideY）"
[CA]::Click($r, $OutsideX, $OutsideY)
Start-Sleep -Milliseconds 900
$closed = Shot 'closed'
"   -> $closed"

# 用像素差粗略判断弹层是否消失：弹层是白底，正文是黄底
function Whiteness([string]$path) {
  Add-Type -AssemblyName System.Drawing
  $bmp = [System.Drawing.Bitmap]::FromFile($path)
  # 弹层大致占据 x 250-380, y 30-200（物理）
  $n = 0; $white = 0
  for ($x = 255; $x -lt 375; $x += 3) {
    for ($y = 35; $y -lt 195; $y += 3) {
      $c = $bmp.GetPixel($x, $y)
      $n++
      if ($c.R -gt 235 -and $c.G -gt 235 -and $c.B -gt 235) { $white++ }
    }
  }
  $bmp.Dispose()
  return [math]::Round($white / $n, 3)
}
$wo = Whiteness $open
$wc = Whiteness $closed
"`n弹层区域白色占比：开=$wo  点外面后=$wc"
if ($wo -gt 0.12 -and $wc -lt ($wo * 0.5)) { "`n✅ 点外面成功关闭弹层" }
elseif ($wo -le 0.12) { "`n⚠️ 第一步就没开出弹层（坐标可能偏了）" }
else { "`n❌ 弹层未关闭" }
