# 从真分辨率桌面截图里裁出指定窗口区域，便于逐像素检查
param(
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][int]$X, [Parameter(Mandatory)][int]$Y,
    [Parameter(Mandatory)][int]$W, [Parameter(Mandatory)][int]$H,
    [Parameter(Mandatory)][string]$Out
)
Add-Type -AssemblyName System.Drawing
$full = [System.Drawing.Image]::FromFile($Source)
$bmp = New-Object System.Drawing.Bitmap $W, $H
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.DrawImage($full, (New-Object System.Drawing.Rectangle 0,0,$W,$H),
                 (New-Object System.Drawing.Rectangle $X,$Y,$W,$H), [System.Drawing.GraphicsUnit]::Pixel)
$g.Dispose(); $full.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"cropped $W x $H @($X,$Y) -> $Out"
