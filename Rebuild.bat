@echo off
rem Rebuild the Tellus mod into the Tellus instance after changing its source.
cd /d "%~dp0"
"%LOCALAPPDATA%\Programs\Horizon Launcher\horizon-launcher.exe" native-build . --instance tellus --trust-native-code
pause
