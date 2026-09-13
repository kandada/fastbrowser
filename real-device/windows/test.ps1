# Windows：跑真实浏览器集成测试（bundled/chromium）并清理孤儿进程。
param()
# 1) 环境（大资源在外部盘）
. (Join-Path $PSScriptRoot "env.ps1")

Set-Location (Join-Path $PSScriptRoot "..\..")   # fastbrowser 内核仓库

# 2) 全特性真实浏览器测试（无头 + 有头 + 异步）
cargo test --features "engine-cdp,async-core" --test chromium_integration --test chromium_headed_integration --test async_core_e2e
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 3) 清理孤儿测试 Chrome（Windows：按命令行含 fastbrowser-test-chrome 匹配）
Get-CimInstance Win32_Process -Filter "Name like 'chrome%'" | Where-Object { $_.CommandLine -like "*fastbrowser-test-chrome*" -or $_.CommandLine -like "*fastbrowser-cft-*" } | ForEach-Object {
    Write-Host "收割孤儿: $($_.ProcessId)"
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
}
Write-Host "done."
