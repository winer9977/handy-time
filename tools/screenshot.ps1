# 截取整个虚拟屏幕到 PNG（用于验证桌面挂件真实效果）
param([string]$Out = (Join-Path (Split-Path -Parent $PSScriptRoot) 'shot.png'))
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size)
$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"saved $Out  ($($b.Width)x$($b.Height))"
