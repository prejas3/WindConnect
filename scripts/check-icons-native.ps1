param([string]$Output = (Join-Path $env:TEMP 'windconnect-wn93-native'))
$ErrorActionPreference = 'Stop'
# CCS/1 CHECK:actual Win32 ICO selection at every embedded size; no installed app, shell settings or user clipboard changed.
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class IconProof {
  [StructLayout(LayoutKind.Sequential)] public struct IconInfo { public bool icon; public uint x,y; public IntPtr mask,color; }
  [StructLayout(LayoutKind.Sequential)] public struct Bitmap { public int type,width,height,stride; public ushort planes,bits; public IntPtr data; }
  [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] public static extern IntPtr LoadImageW(IntPtr instance,string name,uint type,int x,int y,uint flags);
  [DllImport("user32.dll",EntryPoint="LoadImageW",SetLastError=true)] public static extern IntPtr LoadImageResource(IntPtr instance,IntPtr name,uint type,int x,int y,uint flags);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] public static extern IntPtr LoadLibraryExW(string file,IntPtr reserved,uint flags);
  [DllImport("kernel32.dll")] public static extern bool FreeLibrary(IntPtr module);
  [DllImport("user32.dll")] public static extern bool GetIconInfo(IntPtr icon,out IconInfo info);
  [DllImport("user32.dll")] public static extern bool DestroyIcon(IntPtr icon);
  [DllImport("gdi32.dll",CharSet=CharSet.Unicode)] public static extern int GetObjectW(IntPtr handle,int size,out Bitmap bitmap);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr handle);
}
'@
$file = (Resolve-Path (Join-Path $PSScriptRoot '../apps/windows/src-tauri/icons/icon.ico')).Path
$bytes = [IO.File]::ReadAllBytes($file)
$count = [BitConverter]::ToUInt16($bytes,4)
$expected = @(16,20,24,32,40,48,64,96,128,256)
if ($count -ne $expected.Count) { throw 'ICO image count differs' }
New-Item -ItemType Directory -Path $Output -Force | Out-Null
$exe = (Resolve-Path (Join-Path $PSScriptRoot '../target/release/windconnect-windows.exe')).Path
$module = [IconProof]::LoadLibraryExW($exe,[IntPtr]::Zero,0x22)
if ($module -eq [IntPtr]::Zero) { throw 'cannot read EXE icon resource' }
try { foreach ($source in @('native','exe')) { foreach ($size in $expected) {
  $handle = if ($source -eq 'native') { [IconProof]::LoadImageW([IntPtr]::Zero,$file,1,$size,$size,0x10) } else { [IconProof]::LoadImageResource($module,[IntPtr]32512,1,$size,$size,0) }
  if ($handle -eq [IntPtr]::Zero) { throw "Win32 cannot load ICO $size" }
  $info = New-Object IconProof+IconInfo
  try {
    if (![IconProof]::GetIconInfo($handle,[ref]$info)) { throw 'GetIconInfo failed' }
    $bitmap = New-Object IconProof+Bitmap
    if (![IconProof]::GetObjectW($info.color,[Runtime.InteropServices.Marshal]::SizeOf($bitmap),[ref]$bitmap)) { throw 'GetObject failed' }
    if ($bitmap.width -ne $size -or $bitmap.height -ne $size -or $bitmap.bits -ne 32) { throw "bad native ICO dimensions/depth $size" }
    $icon = [Drawing.Icon]::FromHandle($handle)
    $image = $icon.ToBitmap()
    try { $image.Save((Join-Path $Output "$source-$size.png"),[Drawing.Imaging.ImageFormat]::Png) } finally { $image.Dispose(); $icon.Dispose() }
  } finally {
    if ($info.color -ne [IntPtr]::Zero) { [void][IconProof]::DeleteObject($info.color) }
    if ($info.mask -ne [IntPtr]::Zero) { [void][IconProof]::DeleteObject($info.mask) }
    [void][IconProof]::DestroyIcon($handle)
  }
} } } finally { [void][IconProof]::FreeLibrary($module) }
foreach ($size in $expected) {
  if ((Get-FileHash (Join-Path $Output "native-$size.png")).Hash -ne (Get-FileHash (Join-Path $Output "exe-$size.png")).Hash) { throw "EXE icon differs from ICO at $size" }
}
"PASS: Win32 loaded all $count ICO and EXE resource sizes identically as 32-bit native icons; $Output"
