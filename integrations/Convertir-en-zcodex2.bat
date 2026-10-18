@echo off
REM Glisse un fichier .gcode (PrusaSlicer, OrcaSlicer, Cura, Simplify3D, IdeaMaker...) sur ce fichier:
REM il cree le .zcodex2 correspondant a cote du G-code, pour la Zortrax M200 Plus.
REM Autre materiau: Convertir-en-zcodex2.bat fichier.gcode --material Z-ABS
setlocal
if "%~1"=="" goto usage
"%~dp0bin\ztool.exe" to-zcodex2 %*
if errorlevel 1 goto failed
echo.
echo Termine.
goto end
:usage
echo Glisse un fichier .gcode sur ce script.
goto end
:failed
echo.
echo La conversion a echoue. Lis le message ci-dessus.
:end
pause
