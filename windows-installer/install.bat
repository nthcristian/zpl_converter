@echo off
setlocal
set "INSTALL_DIR=%LOCALAPPDATA%\Programs\ZPL Converter"

echo Installing ZPL Converter to "%INSTALL_DIR%"...
if not exist "%INSTALL_DIR%" mkdir "%INSTALL_DIR%"
copy /Y "%~dp0zpl-converter.exe" "%INSTALL_DIR%\zpl-converter.exe" >nul
copy /Y "%~dp0uninstall.bat" "%INSTALL_DIR%\uninstall.bat" >nul

powershell -NoProfile -ExecutionPolicy Bypass -Command "$shell = New-Object -ComObject WScript.Shell; $shortcut = $shell.CreateShortcut([Environment]::GetFolderPath('Desktop') + '\ZPL Converter.lnk'); $shortcut.TargetPath = '%INSTALL_DIR%\zpl-converter.exe'; $shortcut.WorkingDirectory = '%INSTALL_DIR%'; $shortcut.Save()"

echo Installed successfully.
start "" "%INSTALL_DIR%\zpl-converter.exe"
endlocal
