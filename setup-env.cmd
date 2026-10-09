@echo off
set "MINGW_BIN=C:\Users\fschmidt\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
set "CARGO_BIN=%USERPROFILE%\.cargo\bin"

set "GIT_BIN=C:\Users\fschmidt\AppData\Local\Programs\Git\cmd"
set "GH_BIN=C:\Users\fschmidt\AppData\Local\Microsoft\WinGet\Packages\GitHub.cli_Microsoft.Winget.Source_8wekyb3d8bbwe\bin"

set "PATH=%CARGO_BIN%;%MINGW_BIN%;%GIT_BIN%;%GH_BIN%;%PATH%"

echo Rust and MinGW environment loaded successfully!
rustc --version
cargo --version
gcc --version
