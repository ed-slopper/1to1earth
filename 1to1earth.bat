@echo off
rem 1to1earth: play the 1to1earth instance (Horizon with the 1to1earth mod built in).
rem First time in a new world: the mod adds its datapack; save and quit, open the world again,
rem and you're put on the Earth. Esc > Earth Map picks a place; /earth tp <latitude> <longitude> too.
"%LOCALAPPDATA%\Programs\Horizon Launcher\horizon-launcher.exe" play --instance tellus --world Earth
if errorlevel 1 pause
