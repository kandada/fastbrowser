# Windows 真机测试环境：大资源重定向到外部盘 fastbrowser_local。
# 用法：  powershell -File real-device/windows/env.ps1
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$env:FB_LOCAL = (Resolve-Path (Join-Path $scriptDir "..\..\")).Path   # fastbrowser_local
$env:CARGO_TARGET_DIR = Join-Path $env:FB_LOCAL "target"
$env:VENDOR_DIR = Join-Path $env:FB_LOCAL "vendor"
$env:CHROME_PATH = Get-ChildItem -Path (Join-Path $env:FB_LOCAL "vendor\chromium-win") -Recurse -Filter "chrome.exe" -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
$env:REAL_DEVICE_REPORTS = Join-Path $env:FB_LOCAL "real-device-reports"
New-Item -ItemType Directory -Force -Path $env:REAL_DEVICE_REPORTS | Out-Null
Write-Host "FB_LOCAL=$env:FB_LOCAL"
Write-Host "CHROME_PATH=$env:CHROME_PATH"
