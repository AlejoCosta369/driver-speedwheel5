@echo off
chcp 65001 >nul
title Speed Wheel 5 Pro - prueba de vibracion
echo Con el volante conectado, agarralo: van a sonar 7 efectos de vibracion (~16 s).
echo.
"%~dp0sw5.exe" status
echo.
"%~dp0sw5.exe" ffb-test all
echo.
pause
