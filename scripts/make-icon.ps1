# Génère le logo REZ (1024x1024 PNG) -> scripts/rez-logo.png
# Puis: npx tauri icon scripts/rez-logo.png  (crée icon.ico + PNG dans src-tauri/icons)
Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'
$out = Join-Path $PSScriptRoot 'rez-logo.png'
$S = 1024
$bmp = New-Object System.Drawing.Bitmap $S, $S
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = 'AntiAlias'
$g.TextRenderingHint = 'AntiAlias'
$g.Clear([System.Drawing.Color]::Transparent)

function C($a, $r, $gg, $b) { [System.Drawing.Color]::FromArgb($a, $r, $gg, $b) }
function RR($x, $y, $w, $h, $r) {
  $p = New-Object System.Drawing.Drawing2D.GraphicsPath
  $d = $r * 2
  $p.AddArc($x, $y, $d, $d, 180, 90)
  $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
  $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
  $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
  $p.CloseFigure()
  return $p
}

# fond verre sombre
$bg = RR 24 24 976 976 236
$lin = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Point 0, 0), (New-Object System.Drawing.Point $S, $S), (C 255 26 20 48), (C 255 7 8 16)
$g.FillPath($lin, $bg)
$g.SetClip($bg)

# lueurs douces
$e1 = New-Object System.Drawing.Drawing2D.GraphicsPath; $e1.AddEllipse(-120, 120, 900, 820)
$pg1 = New-Object System.Drawing.Drawing2D.PathGradientBrush $e1
$pg1.CenterColor = C 150 139 92 246; $pg1.SurroundColors = @((C 0 139 92 246))
$g.FillPath($pg1, $e1)
$e2 = New-Object System.Drawing.Drawing2D.GraphicsPath; $e2.AddEllipse(380, 420, 760, 700)
$pg2 = New-Object System.Drawing.Drawing2D.PathGradientBrush $e2
$pg2.CenterColor = C 120 34 211 238; $pg2.SurroundColors = @((C 0 34 211 238))
$g.FillPath($pg2, $e2)

# reflet verre (haut)
$hl = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Point 0, 24), (New-Object System.Drawing.Point 0, 600), (C 40 255 255 255), (C 0 255 255 255)
$g.FillPath($hl, (RR 24 24 976 500 236))

# île dynamique (pilule)
$pill = RR 292 130 440 112 56
$g.FillPath((New-Object System.Drawing.SolidBrush (C 255 4 4 8)), $pill)
$g.DrawPath((New-Object System.Drawing.Pen (C 90 255 255 255), 4), $pill)
$g.FillEllipse((New-Object System.Drawing.SolidBrush (C 255 34 211 238)), 640, 166, 40, 40)
$g.FillEllipse((New-Object System.Drawing.SolidBrush (C 70 34 211 238)), 626, 152, 68, 68)

# lettre R avec halo
$fam = New-Object System.Drawing.FontFamily 'Segoe UI'
$txt = New-Object System.Drawing.Drawing2D.GraphicsPath
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = 'Center'; $sf.LineAlignment = 'Center'
$txt.AddString('R', $fam, [int][System.Drawing.FontStyle]::Bold, 600, (New-Object System.Drawing.RectangleF 0, 250, $S, 720), $sf)
foreach ($i in 8..1) {
  $pen = New-Object System.Drawing.Pen (C (6 * (9 - $i)) 160 120 255), ($i * 9)
  $pen.LineJoin = 'Round'
  $g.DrawPath($pen, $txt)
}
$tb = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Point 0, 300), (New-Object System.Drawing.Point 0, 900), (C 255 255 255 255), (C 255 196 181 253)
$g.FillPath($tb, $txt)

# bordure
$g.ResetClip()
$g.DrawPath((New-Object System.Drawing.Pen (C 80 255 255 255), 6), $bg)

$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Host "Logo -> $out"
