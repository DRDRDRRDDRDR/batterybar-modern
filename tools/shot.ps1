# 电池条实机验收截图脚本
# 用法: pwsh -File tools\shot.ps1 <输出文件名> [放大倍数]
# 原理: 枚举 Shell_TrayWnd 的 BBM_Bar 子窗口，用 PrintWindow(PW_RENDERFULLCONTENT)
#       直接抓窗口内容（不受屏幕坐标/DPI 虚拟化影响），再放大便于肉眼检查像素。

param(
  [string]$Name = "shot_bar.png",
  [int]$Zoom = 6
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

if (-not ("ShotWin" -as [type])) {
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class ShotWin {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
'@
}

Add-Type -AssemblyName System.Drawing

function Get-BbmWindow([string]$cls) {
  $tb = [ShotWin]::FindWindow("Shell_TrayWnd", $null)
  if ($tb -eq [IntPtr]::Zero) { throw "Shell_TrayWnd not found" }
  $script:found = [IntPtr]::Zero
  [ShotWin]::EnumChildWindows($tb, {
    param($h, $l)
    $sb = New-Object System.Text.StringBuilder 256
    [ShotWin]::GetClassName($h, $sb, 256) | Out-Null
    if ($sb.ToString() -eq $cls) { $script:found = $h }
    $true
  }, [IntPtr]::Zero) | Out-Null
  return $script:found
}

function Save-WindowShot([IntPtr]$h, [string]$file, [int]$zoom) {
  $r = New-Object ShotWin+RECT
  [ShotWin]::GetWindowRect($h, [ref]$r) | Out-Null
  $w = $r.Right - $r.Left
  $ht = $r.Bottom - $r.Top
  if ($w -le 0 -or $ht -le 0) { throw "bad rect $w x $ht" }
  $bmp = New-Object System.Drawing.Bitmap($w, $ht)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $g.GetHdc()
  [ShotWin]::PrintWindow($h, $hdc, 2) | Out-Null
  $g.ReleaseHdc($hdc); $g.Dispose()
  $dst = New-Object System.Drawing.Bitmap(($w * $zoom), ($ht * $zoom))
  $g2 = [System.Drawing.Graphics]::FromImage($dst)
  $g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
  $g2.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
  $g2.DrawImage($bmp, 0, 0, ($w * $zoom), ($ht * $zoom))
  $out = Join-Path $root $file
  $dst.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose(); $g2.Dispose(); $dst.Dispose()
  return "$out  ($w x $ht @ ${zoom}x)"
}

$bar = Get-BbmWindow "BBM_Bar"
if ($bar -eq [IntPtr]::Zero) { throw "BBM_Bar not found" }
Save-WindowShot $bar $Name $Zoom
