; ShellDeck NSIS Installer Script
; Requires NSIS 3.x (makensis)

!include "MUI2.nsh"
!include "FileFunc.nsh"

; ---------------------------------------------------------------------------
; Configuration
; ---------------------------------------------------------------------------
!define APP_NAME "Inklura Desktop"
; Keep the registry identity so upgrades reuse the existing install and uninstaller.
!define APP_ID "ShellDeck"
!define APP_EXE "shelldeck.exe"
!define APP_PUBLISHER "FAVRE BENJAMIN (WEB-DESIGN 29)"
!define APP_URL "https://github.com/benfavre/shelldeck"
!define APP_DESCRIPTION "GPU-accelerated terminal and SSH companion"

; Version is passed via /DVERSION=x.y.z on the makensis command line
!ifndef VERSION
    !define VERSION "0.0.0"
!endif

Name "${APP_NAME} ${VERSION}"
OutFile "dist\ShellDeck-windows-x86_64-setup.exe"
InstallDir "$PROGRAMFILES64\${APP_NAME}"
InstallDirRegKey HKLM "Software\${APP_ID}" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

; ---------------------------------------------------------------------------
; Modern UI pages
; ---------------------------------------------------------------------------
!define MUI_ABORTWARNING

; Use custom icon if available
!if /FileExists "packaging\icons\shelldeck.ico"
    !define MUI_ICON "packaging\icons\shelldeck.ico"
    !define MUI_UNICON "packaging\icons\shelldeck.ico"
!endif

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ---------------------------------------------------------------------------
; Install section
; ---------------------------------------------------------------------------
Section "Install"
    SetOutPath "$INSTDIR"

    ; Main binary
    File "target\release\shelldeck.exe"

    ; Versioned Chromium payload: never overwrite a helper/DLL in use by an
    ; existing session. The new main binary discovers this matching version.
    SetOutPath "$INSTDIR\browser\${VERSION}"
    File /r "target\release\browser\${VERSION}\*"
    SetOutPath "$INSTDIR"

    ; Icon (if available)
    !if /FileExists "packaging\icons\shelldeck.ico"
        File /oname=shelldeck.ico "packaging\icons\shelldeck.ico"
    !endif

    ; Create uninstaller
    WriteUninstaller "$INSTDIR\Uninstall.exe"

    ; Retire only the old product's shortcuts; keep the shared install identity.
    Delete "$SMPROGRAMS\ShellDeck\ShellDeck.lnk"
    Delete "$SMPROGRAMS\ShellDeck\ShellDeck Agents.lnk"
    Delete "$SMPROGRAMS\ShellDeck\Uninstall.lnk"
    RMDir "$SMPROGRAMS\ShellDeck"
    Delete "$DESKTOP\ShellDeck.lnk"

    ; Start menu shortcuts
    CreateDirectory "$SMPROGRAMS\${APP_NAME}"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" \
        "" "$INSTDIR\shelldeck.ico"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME} Agents.lnk" "$INSTDIR\${APP_EXE}" \
        "--agents" "$INSTDIR\shelldeck.ico"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk" "$INSTDIR\Uninstall.exe"

    ; Explorer "Send to" entry: appends the selected files after --share.
    ; They are staged in Share for review; nothing uploads automatically.
    CreateShortcut "$SENDTO\Inklura Share.lnk" "$INSTDIR\${APP_EXE}" \
        "--share" "$INSTDIR\shelldeck.ico"

    ; Desktop shortcut
    CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" \
        "" "$INSTDIR\shelldeck.ico"

    ; Registry: Add/Remove Programs
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "DisplayName" "${APP_NAME}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "DisplayIcon" "$INSTDIR\shelldeck.ico"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "Publisher" "${APP_PUBLISHER}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "URLInfoAbout" "${APP_URL}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "DisplayVersion" "${VERSION}"
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "NoModify" 1
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "NoRepair" 1

    ; Compute installed size
    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    IntFmt $0 "0x%08X" $0
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}" \
        "EstimatedSize" $0

    ; Store install dir
    WriteRegStr HKLM "Software\${APP_ID}" "InstallDir" "$INSTDIR"

    ; Register the shelldeck:// URL protocol so deep links open the app.
    WriteRegStr HKCR "shelldeck" "" "URL:Inklura Desktop Protocol"
    WriteRegStr HKCR "shelldeck" "URL Protocol" ""
    WriteRegStr HKCR "shelldeck\DefaultIcon" "" "$INSTDIR\${APP_EXE},0"
    WriteRegStr HKCR "shelldeck\shell\open\command" "" '"$INSTDIR\${APP_EXE}" "%1"'
SectionEnd

; ---------------------------------------------------------------------------
; Uninstall section
; ---------------------------------------------------------------------------
Section "Uninstall"
    ; Remove files
    Delete "$INSTDIR\${APP_EXE}"
    Delete "$INSTDIR\shelldeck.ico"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir /r "$INSTDIR\browser"
    RMDir "$INSTDIR"

    ; Remove shortcuts
    Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
    Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME} Agents.lnk"
    Delete "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk"
    RMDir "$SMPROGRAMS\${APP_NAME}"
    Delete "$DESKTOP\${APP_NAME}.lnk"
    Delete "$SENDTO\Inklura Share.lnk"

    ; Remove registry
    DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"
    DeleteRegKey HKLM "Software\${APP_ID}"

    ; Remove the shelldeck:// URL protocol registration.
    DeleteRegKey HKCR "shelldeck"
SectionEnd
