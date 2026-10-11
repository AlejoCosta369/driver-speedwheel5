@echo off
chcp 65001 >nul
title Speed Wheel 5 Pro - instalar driver
if not exist "%~dp0sw5.exe" (
  echo No se encuentra sw5.exe junto a este script.
  echo Usa los scripts de la carpeta del paquete descomprimido ^(SpeedWheel5-driver-...^),
  echo no los de packaging\windows del codigo fuente.
  echo.
  pause
  exit /b 1
)
echo Instalando el driver de vibracion del Speed Wheel 5 Pro...
echo Windows va a pedir permiso de administrador: acepta el aviso.
echo.
"%~dp0sw5.exe" install
echo.
pause
