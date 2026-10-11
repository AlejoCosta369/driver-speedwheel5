@echo off
chcp 65001 >nul
title Speed Wheel 5 Pro - instalar driver
echo Instalando el driver de vibracion del Speed Wheel 5 Pro...
echo Windows va a pedir permiso de administrador: acepta el aviso.
echo.
"%~dp0sw5.exe" install
echo.
pause
