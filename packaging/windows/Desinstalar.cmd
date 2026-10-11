@echo off
chcp 65001 >nul
title Speed Wheel 5 Pro - desinstalar driver
echo Se desinstala el driver Rust y se restaura el driver anterior.
echo Windows va a pedir permiso de administrador: acepta el aviso.
echo.
"%~dp0sw5.exe" uninstall
echo.
pause
