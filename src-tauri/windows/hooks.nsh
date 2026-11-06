; Otodo NSIS hooks
; 开机自启：放在完成页，与「运行应用」「创建桌面快捷方式」一起勾选
; 通过重定义 MUI_PAGE_FINISH，给完成页加上 SHOW/LEAVE 回调

!include "nsDialogs.nsh"
!include "LogicLib.nsh"

!ifmacrodef MUI_PAGE_FINISH
  !macroundef MUI_PAGE_FINISH
!endif
!macro MUI_PAGE_FINISH
  !verbose push
  !verbose ${MUI_VERBOSE}
  !define MUI_PAGE_CUSTOMFUNCTION_SHOW OtodoFinishShow
  !define MUI_PAGE_CUSTOMFUNCTION_LEAVE OtodoFinishLeave
  !insertmacro MUI_PAGE_INIT
  !insertmacro MUI_PAGEDECLARATION_FINISH
  !verbose pop
!macroend

Var OtodoAutostartCheckbox

!macro OtodoWriteAutostart
  ; 与 tauri-plugin-autostart / auto-launch 一致：HKCU Run，键名 = Cargo package name
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "otodo" '"$INSTDIR\otodo.exe"'
  DetailPrint "已启用开机自启"
!macroend

!macro OtodoClearAutostart
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "otodo"
  DetailPrint "未启用开机自启"
!macroend

!macro OtodoMarkAutostartHandled
  CreateDirectory "$LOCALAPPDATA\com.otodo.desktop"
  FileOpen $0 "$LOCALAPPDATA\com.otodo.desktop\autostart_initialized" w
  FileWrite $0 "installer"
  FileClose $0
!macroend

Function OtodoFinishShow
  ; 与上面两项同一套 nsDialogs 复选框，不要关视觉样式，否则会变成经典灰底勾
  ${NSD_CreateCheckbox} 120u 130u 195u 10u "开机自启"
  Pop $OtodoAutostartCheckbox
  SendMessage $OtodoAutostartCheckbox ${BM_SETCHECK} ${BST_CHECKED} 0
  ${NSD_SetFocus} $mui.FinishPage.Run
FunctionEnd

Function OtodoFinishLeave
  ${If} $OtodoAutostartCheckbox <> 0
    ${NSD_GetState} $OtodoAutostartCheckbox $0
    ${If} $0 = ${BST_CHECKED}
      !insertmacro OtodoWriteAutostart
    ${Else}
      !insertmacro OtodoClearAutostart
    ${EndIf}
  ${EndIf}
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  ; 静默/被动安装会跳过完成页，沿用原来的默认：开启自启
  ${If} ${Silent}
    !insertmacro OtodoWriteAutostart
  ${Else}
    ClearErrors
    ${GetOptions} $CMDLINE "/P" $R0
    ${IfNot} ${Errors}
      !insertmacro OtodoWriteAutostart
    ${EndIf}
  ${EndIf}
  !insertmacro OtodoMarkAutostartHandled
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "otodo"
!macroend
