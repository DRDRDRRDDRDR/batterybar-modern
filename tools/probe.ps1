# 像素探针：按行扫描电池条窗口的实际颜色，定位渲染层次是否正确
# 用法: powershell -ExecutionPolicy Bypass -File tools\probe.ps1 [x0] [x1] [y0] [y1]
param(
  [int]$X0 = 0, [int]$X1 = 81, [int]$Y0 = 0, [int]$Y1 = 46
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

if (-not ("ProbeWin" -as [type])) {
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class ProbeWin {
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

$tb = [ProbeWin]::FindWindow("Shell_TrayWnd", $null)
$script:h = [IntPtr]::Zero
[ProbeWin]::EnumChildWindows($tb, {
  param($w, $l)
  $sb = New-Object System.Text.StringBuilder 256
  [ProbeWin]::GetClassName($w, $sb, 256) | Out-Null
  if ($sb.ToString() -eq "BBM_Bar") { $script:h = $w }
  $true
}, [IntPtr]::Zero) | Out-Null

$r = New-Object ProbeWin+RECT
[ProbeWin]::GetWindowRect($script:h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[ProbeWin]::PrintWindow($script:h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc); $g.Dispose()

Write-Output "window ${w}x${ht}  (BGR hex, R=$([char]27)[31mred[0m G=green B=blue)"
$hdr = "y  ".PadRight(4)
for ($x = $X0; $x -lt $X1; $x++) { $hdr += ("{0,-9}" -f $x) }
Write-Output $hdr
for ($y = $Y0; $y -lt $Y1; $y++) {
  $line = ("{0,-4}" -f $y)
  for ($x = $X0; $x -lt $X1; $x++) {
    $c = $bmp.GetPixel($x, $y)
    $line += ("{0,-9}" -f ("{0:X2}{1:X2}{2:X2}" -f $c.B, $c.G, $c.R))
  }
  Write-Output $line
}
$bmp.Dispose()
