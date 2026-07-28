param(
  [string]$BrandDir = (Join-Path $PSScriptRoot "..\..\src\assets\brand")
)

Add-Type -AssemblyName System.Drawing

$outputDir = $PSScriptRoot
$iconPath = Join-Path $BrandDir "icon-src.png"
$mascotPath = Join-Path $BrandDir "kotone-cutout.png"

function New-Canvas([int]$width, [int]$height) {
  $bitmap = [System.Drawing.Bitmap]::new(
    $width,
    $height,
    [System.Drawing.Imaging.PixelFormat]::Format24bppRgb
  )
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
  $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $graphics.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::ClearTypeGridFit
  return @{ Bitmap = $bitmap; Graphics = $graphics }
}

function Add-Gradient($graphics, [int]$width, [int]$height) {
  $rect = [System.Drawing.Rectangle]::new(0, 0, $width, $height)
  $brush = [System.Drawing.Drawing2D.LinearGradientBrush]::new(
    $rect,
    [System.Drawing.Color]::FromArgb(7, 9, 27),
    [System.Drawing.Color]::FromArgb(30, 8, 45),
    115
  )
  $graphics.FillRectangle($brush, $rect)
  $brush.Dispose()
}

function Add-NeonLines($graphics, [int]$width, [int]$height) {
  $cyan = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(125, 0, 229, 255), 1.4)
  $pink = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(115, 255, 30, 180), 1.2)
  for ($i = -$height; $i -lt $width + $height; $i += 28) {
    $graphics.DrawLine($cyan, $i, $height, $i + $height, 0)
  }
  for ($i = -$height; $i -lt $width + $height; $i += 52) {
    $graphics.DrawLine($pink, $i, 0, $i + $height, $height)
  }
  $cyan.Dispose()
  $pink.Dispose()
}

function Save-Bitmap($canvas, [string]$path) {
  $canvas.Bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Bmp)
  $canvas.Graphics.Dispose()
  $canvas.Bitmap.Dispose()
}

# NSIS Modern UI welcome / finish page artwork (recommended exact size: 164 x 314).
$sidebar = New-Canvas 164 314
Add-Gradient $sidebar.Graphics 164 314
Add-NeonLines $sidebar.Graphics 164 314

$titleFont = [System.Drawing.Font]::new("Segoe UI", 17, [System.Drawing.FontStyle]::Bold)
$labelFont = [System.Drawing.Font]::new("Segoe UI", 7.5, [System.Drawing.FontStyle]::Bold)
$cnFont = [System.Drawing.Font]::new("Microsoft YaHei UI", 7.5, [System.Drawing.FontStyle]::Regular)
$white = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White)
$cyanBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(0, 229, 255))
$muted = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(185, 205, 220))

$sidebar.Graphics.DrawString("KOTONE", $titleFont, $white, 12, 11)
$sidebar.Graphics.DrawString("VOICE  //  READY", $labelFont, $cyanBrush, 14, 39)

$mascot = [System.Drawing.Image]::FromFile($mascotPath)
# The current RepoChan delivery is a full-body thumbs-up pose. Crop below the
# knees so the face and hand stay readable in MUI2's narrow 164 px sidebar.
$source = [System.Drawing.Rectangle]::new(300, 15, 390, 760)
$dest = [System.Drawing.Rectangle]::new(15, 53, 133, 259)
$sidebar.Graphics.DrawImage($mascot, $dest, $source, [System.Drawing.GraphicsUnit]::Pixel)
$mascot.Dispose()

$footerRect = [System.Drawing.RectangleF]::new(10, 286, 145, 20)
$footerBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(190, 7, 9, 27))
$sidebar.Graphics.FillRectangle($footerBrush, $footerRect)
$sidebar.Graphics.DrawString("LOCAL VOICE  //  GAME READY", $cnFont, $muted, 12, 289)

Save-Bitmap $sidebar (Join-Path $outputDir "sidebar.bmp")

$titleFont.Dispose()
$labelFont.Dispose()
$cnFont.Dispose()
$white.Dispose()
$cyanBrush.Dispose()
$muted.Dispose()
$footerBrush.Dispose()

# NSIS steps-page header artwork (recommended exact size: 150 x 57).
$header = New-Canvas 150 57
Add-Gradient $header.Graphics 150 57
Add-NeonLines $header.Graphics 150 57

$icon = [System.Drawing.Image]::FromFile($iconPath)
$header.Graphics.DrawImage($icon, [System.Drawing.Rectangle]::new(4, 4, 49, 49))
$icon.Dispose()

$headerTitle = [System.Drawing.Font]::new("Segoe UI", 14, [System.Drawing.FontStyle]::Bold)
$headerSub = [System.Drawing.Font]::new("Segoe UI", 6.5, [System.Drawing.FontStyle]::Bold)
$headerWhite = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White)
$headerCyan = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(0, 229, 255))
$header.Graphics.DrawString("KOTONE", $headerTitle, $headerWhite, 55, 7)
$header.Graphics.DrawString("VOICE INPUT", $headerSub, $headerCyan, 57, 34)

Save-Bitmap $header (Join-Path $outputDir "header.bmp")

$headerTitle.Dispose()
$headerSub.Dispose()
$headerWhite.Dispose()
$headerCyan.Dispose()

Write-Host "Generated NSIS artwork in $outputDir"
