# Refresh PATH in the current PowerShell session
$mingwBin = "C:\Users\fschmidt\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
$cargoBin = "$env:USERPROFILE\.cargo\bin"

$gitBin = "C:\Users\fschmidt\AppData\Local\Programs\Git\cmd"
$ghBin = "C:\Users\fschmidt\AppData\Local\Microsoft\WinGet\Packages\GitHub.cli_Microsoft.Winget.Source_8wekyb3d8bbwe\bin"

if ($env:PATH -notlike "*$cargoBin*") {
    $env:PATH = "$cargoBin;$env:PATH"
}
if ($env:PATH -notlike "*$mingwBin*") {
    $env:PATH = "$mingwBin;$env:PATH"
}
if ($env:PATH -notlike "*$gitBin*") {
    $env:PATH = "$gitBin;$env:PATH"
}
if ($env:PATH -notlike "*$ghBin*") {
    $env:PATH = "$ghBin;$env:PATH"
}

Write-Host "Rust and MinGW environment loaded successfully!" -ForegroundColor Green
Write-Host "rustc: $(rustc --version)"
Write-Host "cargo: $(cargo --version)"
Write-Host "gcc:   $(& gcc --version | Select-Object -First 1)"
