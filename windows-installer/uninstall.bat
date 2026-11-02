@echo off
setlocal
set "INSTALL_DIR=%LOCALAPPDATA%\Programs\ZPL Converter"

del "%USERPROFILE%\Desktop\ZPL Converter.lnk" 2>nul
rmdir /S /Q "%INSTALL_DIR%"
echo ZPL Converter was removed.
endlocal
