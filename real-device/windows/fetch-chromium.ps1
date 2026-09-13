# 下载 Chrome for Testing (win64) 到外部盘 vendor/chromium-win（不进 git）。
param()
$env:FB_LOCAL = (Resolve-Path (Join-Path $PSScriptRoot "..\..\")).Path
$outDir = Join-Path $env:FB_LOCAL "vendor\chromium-win"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null

$json = Invoke-RestMethod "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json"
$dl = $json.channels.Stable.downloads.chrome | Where-Object { $_.platform -eq "win64" } | Select-Object -First 1
$version = $json.channels.Stable.version
$zip = Join-Path $outDir "chrome-$version.zip"
Write-Host "下载 $($dl.url) -> $outDir"
Invoke-WebRequest -Uri $dl.url -OutFile $zip
Expand-Archive -Path $zip -DestinationPath $outDir -Force
Remove-Item $zip
Write-Host "完成：$outDir (chrome.exe 由 env.ps1 自动发现)"
