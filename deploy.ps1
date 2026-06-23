cargo build --release
if ($LASTEXITCODE -eq 0) {
    Copy-Item "target\release\gi-utils.exe" -Destination "E:\Program\GI-Utils\" -Force
    Write-Host "-> copied to E:\Program\GI-Utils\" -ForegroundColor Green
}
