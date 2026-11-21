@echo off
rem Tellus for Horizon: play the Tellus instance (Horizon with the Tellus mod built in).
rem First time in a new world: Tellus adds its datapack; save and quit, open the world again,
rem and you're put on the Earth. /earth tp <latitude> <longitude> goes anywhere.
"%LOCALAPPDATA%\Programs\Horizon Launcher\horizon-launcher.exe" play --instance tellus --world Earth
if errorlevel 1 pause
