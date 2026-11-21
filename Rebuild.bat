@echo off
rem Package the mod (.hmod), install it into the 1to1earth instance and build it.
cd /d "%~dp0"
cargo-horizon-mod horizon-mod build || goto :fail
copy /y target\horizon-mod\1to1earth.hmod "%APPDATA%\horizon\instances\tellus\horizon-native-mods\" >nul
"%LOCALAPPDATA%\Programs\Horizon Launcher\horizon-launcher.exe" native-build --instance tellus --trust-native-code
pause
exit /b
:fail
echo Packaging failed: cargo-horizon-mod is Horizon's packaging tool (cargo build -p cargo-horizon-mod in a Horizon checkout).
pause
