@echo off
REM Lance la verification complete du format (voir VERIFICATION.md). Options: -Matrix 300 | -Watch
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0verifier.ps1" %*
if errorlevel 1 (echo. & echo VERIFICATION EN ECHEC) else (echo. & echo VERIFICATION REUSSIE)
pause
