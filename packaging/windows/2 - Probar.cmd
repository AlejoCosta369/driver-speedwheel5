@echo off
chcp 65001 >nul
title Speed Wheel 5 Pro - prueba de vibracion
if not exist "%~dp0sw5.exe" (
  echo No se encuentra sw5.exe junto a este script.
  echo Usa los scripts de la carpeta del paquete descomprimido ^(SpeedWheel5-driver-...^),
  echo no los de packaging\windows del codigo fuente.
  echo.
  pause
  exit /b 1
)
echo Con el volante conectado, agarralo: van a sonar 7 efectos de vibracion (~16 s).
echo.
"%~dp0sw5.exe" status
echo.
"%~dp0sw5.exe" ffb-test all
echo.
pause
