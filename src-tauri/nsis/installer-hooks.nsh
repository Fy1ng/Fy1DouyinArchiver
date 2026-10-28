!include LogicLib.nsh

${StrTrimNewLines}
${UnStrTrimNewLines}

Var EmilyCoordinationState
Var EmilyGuardActive
Var EmilySessionNonce
Var EmilyInstallerPid
Var EmilyInstallerCreatedAt
Var EmilyDataDisposition
Var EmilyControlExecutable
Var EmilyGuardExecutable
Var EmilyCandidateManifest
Var EmilyRepairTakeoverActive
Var EmilyCommitted

!macro NSIS_HOOK_PREUNINSTALL
  ; The pinned installer template owns the authenticated uninstall lifecycle.
!macroend

!macro EMILY_DECLARE_FUNCTIONS
Function EmilyInspectCoordinationState
  CreateDirectory "${EMILY_COORDINATION_DIR}"
  InitPluginsDir
  File /oname=$PLUGINSDIR\emily-installer-control.exe "${MAINBINARYSRCPATH}"
  StrCpy $EmilyControlExecutable "$PLUGINSDIR\emily-installer-control.exe"
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control inspect --coordination-dir "${EMILY_COORDINATION_DIR}"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  ${StrTrimNewLines} $EmilyCoordinationState "$1"
  ${If} $EmilyCoordinationState == "repair_required"
    StrCpy $EmilyRepairTakeoverActive 1
    StrCpy $UpdateMode 1
  ${ElseIf} $EmilyCoordinationState != "clean"
  ${AndIf} $EmilyCoordinationState != "rolled_back"
    MessageBox MB_ICONSTOP|MB_OK "无法确认安装恢复状态：$EmilyCoordinationState"
    Abort
  ${EndIf}
FunctionEnd

Function un.EmilyInspectCoordinationState
  CreateDirectory "${EMILY_COORDINATION_DIR}"
  ${If} ${FileExists} "${EMILY_COORDINATION_DIR}\uninstall-controller.exe"
    StrCpy $EmilyControlExecutable "${EMILY_COORDINATION_DIR}\uninstall-controller.exe"
  ${Else}
    StrCpy $EmilyControlExecutable "$INSTDIR\${MAINBINARYNAME}.exe"
  ${EndIf}
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control inspect --coordination-dir "${EMILY_COORDINATION_DIR}"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  ${UnStrTrimNewLines} $EmilyCoordinationState "$1"
FunctionEnd

Function EmilyReadSession
  FileOpen $0 "${EMILY_COORDINATION_DIR}\installer-session-v1.args" r
  IfErrors 0 +3
    MessageBox MB_ICONSTOP|MB_OK "无法读取卸载保护会话。"
    Abort
  FileRead $0 $EmilySessionNonce
  FileRead $0 $EmilyInstallerPid
  FileRead $0 $EmilyInstallerCreatedAt
  FileRead $0 $EmilyControlExecutable
  FileRead $0 $EmilyGuardExecutable
  FileClose $0
  ${StrTrimNewLines} $EmilySessionNonce "$EmilySessionNonce"
  ${StrTrimNewLines} $EmilyInstallerPid "$EmilyInstallerPid"
  ${StrTrimNewLines} $EmilyInstallerCreatedAt "$EmilyInstallerCreatedAt"
  ${StrTrimNewLines} $EmilyControlExecutable "$EmilyControlExecutable"
  ${StrTrimNewLines} $EmilyGuardExecutable "$EmilyGuardExecutable"
FunctionEnd

Function EmilyRepairTakeover
  ${If} $EmilyRepairTakeoverActive != 1
    Return
  ${EndIf}
  InitPluginsDir
  File /oname=$PLUGINSDIR\emily-installer-control.exe "${MAINBINARYSRCPATH}"
  File /oname=$PLUGINSDIR\emily-candidate-manifest.json "${EMILY_PACKAGE_MANIFEST_PATH}"
  System::Call 'kernel32::GetCurrentProcessId()i.r0'
  StrCpy $EmilyCandidateManifest "$PLUGINSDIR\emily-candidate-manifest.json"
  nsExec::ExecToStack '"$PLUGINSDIR\emily-installer-control.exe" --installer-control takeover --coordination-dir "${EMILY_COORDINATION_DIR}" --install-dir "$INSTDIR" --main-executable "$INSTDIR\${MAINBINARYNAME}.exe" --package-manifest "$EmilyCandidateManifest" --package-manifest-sha256 "${EMILY_PACKAGE_MANIFEST_SHA256}" --action repair --installer-pid "$0"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  Call EmilyReadSession
  StrCpy $EmilyGuardActive 1
  StrCpy $EmilyCommitted 1
FunctionEnd

Function EmilyCompleteRepair
  ${If} $EmilyRepairTakeoverActive != 1
    Return
  ${EndIf}
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control complete-repair --coordination-dir "${EMILY_COORDINATION_DIR}" --session-nonce "$EmilySessionNonce" --installer-pid "$EmilyInstallerPid" --installer-created-at-100ns "$EmilyInstallerCreatedAt" --package-manifest "$EmilyCandidateManifest" --package-manifest-sha256 "${EMILY_PACKAGE_MANIFEST_SHA256}"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  Sleep 100
  Delete "$EmilyControlExecutable"
  StrCpy $EmilyGuardActive 0
  StrCpy $EmilyCommitted 0
FunctionEnd

Function un.EmilyFail
  Pop $1
  Pop $0
  MessageBox MB_ICONSTOP|MB_OK "$1"
  Abort
FunctionEnd

Function un.EmilyReadSession
  FileOpen $0 "${EMILY_COORDINATION_DIR}\installer-session-v1.args" r
  IfErrors 0 +3
    MessageBox MB_ICONSTOP|MB_OK "无法读取卸载保护会话。"
    Abort
  FileRead $0 $EmilySessionNonce
  FileRead $0 $EmilyInstallerPid
  FileRead $0 $EmilyInstallerCreatedAt
  FileRead $0 $EmilyControlExecutable
  FileRead $0 $EmilyGuardExecutable
  FileClose $0
  ${UnStrTrimNewLines} $EmilySessionNonce "$EmilySessionNonce"
  ${UnStrTrimNewLines} $EmilyInstallerPid "$EmilyInstallerPid"
  ${UnStrTrimNewLines} $EmilyInstallerCreatedAt "$EmilyInstallerCreatedAt"
  ${UnStrTrimNewLines} $EmilyControlExecutable "$EmilyControlExecutable"
  ${UnStrTrimNewLines} $EmilyGuardExecutable "$EmilyGuardExecutable"
  ${If} $EmilySessionNonce == ""
  ${OrIf} $EmilyInstallerPid == ""
  ${OrIf} $EmilyInstallerCreatedAt == ""
  ${OrIf} $EmilyControlExecutable == ""
  ${OrIf} $EmilyGuardExecutable == ""
    MessageBox MB_ICONSTOP|MB_OK "卸载保护会话格式无效。"
    Abort
  ${EndIf}
FunctionEnd

Function un.EmilyPrepareGuard
  ${If} $UpdateMode = 1
    Return
  ${EndIf}
  StrCpy $EmilyDataDisposition "retain"
  ${If} $DeleteAppDataCheckboxState = 1
    StrCpy $EmilyDataDisposition "delete"
  ${EndIf}
  StrCpy $EmilyCommitted 0
  System::Call 'kernel32::GetCurrentProcessId()i.r0'
  nsExec::ExecToStack '"$INSTDIR\${MAINBINARYNAME}.exe" --installer-control prepare --coordination-dir "${EMILY_COORDINATION_DIR}" --bootstrap-dir "${EMILY_BOOTSTRAP_DIR}" --install-dir "$INSTDIR" --main-executable "$INSTDIR\${MAINBINARYNAME}.exe" --package-manifest "$INSTDIR\${EMILY_PACKAGE_MANIFEST_FILE}" --package-manifest-sha256 "${EMILY_PACKAGE_MANIFEST_SHA256}" --data-disposition "$EmilyDataDisposition" --installer-pid "$0"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  Call un.EmilyReadSession
  StrCpy $EmilyGuardActive 1
FunctionEnd

Function un.EmilyAssertGuardAlive
  ${If} $UpdateMode = 1
    Return
  ${EndIf}
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control assert-alive --coordination-dir "${EMILY_COORDINATION_DIR}" --session-nonce "$EmilySessionNonce" --installer-pid "$EmilyInstallerPid" --installer-created-at-100ns "$EmilyInstallerCreatedAt"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
FunctionEnd

Function un.EmilyWaitGuardReady
  Call un.EmilyAssertGuardAlive
FunctionEnd

Function un.EmilyCommitGuard
  ${If} $UpdateMode = 1
    Return
  ${EndIf}
  Call un.EmilyAssertGuardAlive
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control commit --coordination-dir "${EMILY_COORDINATION_DIR}" --session-nonce "$EmilySessionNonce" --installer-pid "$EmilyInstallerPid" --installer-created-at-100ns "$EmilyInstallerCreatedAt"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  StrCpy $EmilyCommitted 1
FunctionEnd

Function un.EmilyDeleteManagedData
  ${If} $UpdateMode = 1
    Return
  ${EndIf}
  Call un.EmilyAssertGuardAlive
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control delete-managed-data --coordination-dir "${EMILY_COORDINATION_DIR}" --session-nonce "$EmilySessionNonce" --installer-pid "$EmilyInstallerPid" --installer-created-at-100ns "$EmilyInstallerCreatedAt"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
FunctionEnd

Function un.EmilyCompleteGuard
  ${If} $UpdateMode = 1
    Return
  ${EndIf}
  Call un.EmilyAssertGuardAlive
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control complete-uninstall --coordination-dir "${EMILY_COORDINATION_DIR}" --session-nonce "$EmilySessionNonce" --installer-pid "$EmilyInstallerPid" --installer-created-at-100ns "$EmilyInstallerCreatedAt"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  Sleep 100
  Delete "$EmilyControlExecutable"
  StrCpy $EmilyGuardActive 0
FunctionEnd

Function un.EmilyAbortGuard
  ${If} $EmilyGuardActive = 1
    nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control abort --coordination-dir "${EMILY_COORDINATION_DIR}" --session-nonce "$EmilySessionNonce" --installer-pid "$EmilyInstallerPid" --installer-created-at-100ns "$EmilyInstallerCreatedAt"'
    Pop $0
    Pop $1
    ${If} $EmilyCommitted != 1
      Sleep 100
      Delete "$EmilyControlExecutable"
    ${EndIf}
    StrCpy $EmilyGuardActive 0
  ${EndIf}
FunctionEnd

Function un.EmilyRepairTakeover
  ${If} $EmilyCoordinationState != "repair_required"
    Return
  ${EndIf}
  System::Call 'kernel32::GetCurrentProcessId()i.r0'
  nsExec::ExecToStack '"$EmilyControlExecutable" --installer-control takeover --coordination-dir "${EMILY_COORDINATION_DIR}" --install-dir "$INSTDIR" --main-executable "$INSTDIR\${MAINBINARYNAME}.exe" --package-manifest "$INSTDIR\${EMILY_PACKAGE_MANIFEST_FILE}" --package-manifest-sha256 "${EMILY_PACKAGE_MANIFEST_SHA256}" --action continue-uninstall --installer-pid "$0"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "$1"
    Abort
  ${EndIf}
  Call un.EmilyReadSession
  StrCpy $UpdateMode 0
  StrCpy $EmilyGuardActive 1
  StrCpy $EmilyCommitted 1
FunctionEnd

Function un.onUninstFailed
  Call un.EmilyAbortGuard
FunctionEnd
!macroend
