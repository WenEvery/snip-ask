; =============================================================================
; SnipAsk（框选 AI）· NSIS 卸载清理钩子
; =============================================================================
; 由 src-tauri/tauri.conf.json -> bundle > windows > nsis > installerHooks 引入。
;
; Tauri 2.12 模板在 `!include MUI2.nsh` 等基础头之后、任何 Var / !define 之前
; include 本文件；`!insertmacro NSIS_HOOK_POSTUNINSTALL` 位于 Section Uninstall 尾部
; （此时 $UpdateMode / $DeleteAppDataCheckboxState / ${PRODUCTNAME} / ${MANUPRODUCTKEY}
; 等均已定义）。因此：
;   - 本文件只定义 !macro，不定义 Var（避免与模板冲突）；
;   - 宏体里的 ${PRODUCTNAME} 等 define 在宏插入点才展开，不能在本文件顶层直接引用。
;
; 卸载行为矩阵（变量由 Tauri 模板声明并在 un.onInit / 卸载确认页中赋值）：
;   $UpdateMode = 1（覆盖安装/更新，卸载器由新安装器以 /UPDATE 调起）
;       → 一律保留用户数据（绝不删除）
;   静默卸载 /S（${Silent}）
;       → 无条件彻底清理（用户明确要求静默全卸载）
;   交互卸载 且 勾选「删除应用数据」复选框（$DeleteAppDataCheckboxState = 1）
;       → 彻底清理
;   交互卸载 且 未勾选复选框
;       → MessageBox 明确询问；默认按钮为「是」（Enter=清理），Esc=否（保留）
;   被动安装 /P（$PassiveMode = 1）时卸载确认页被跳过，$DeleteAppDataCheckboxState
;   为空，走「未勾选 → MessageBox」分支，用户仍有明确选择。
;
; 清理范围（全部为本产品专属路径，绝不触碰其它目录）：
;   %APPDATA%\SnipAsk              配置/历史/日志/提示词/后端（platform/paths.rs data_dir）
;   %LOCALAPPDATA%\SnipAsk         模型/缓存/运行时；currentUser 模式下默认也是安装目录
;   %APPDATA%\com.snipask.app      WebView2（BUNDLEID）漫游数据
;   %LOCALAPPDATA%\com.snipask.app WebView2（BUNDLEID）本地数据（EBWebView）
;   注册表 HKCU ...\Run\SnipAsk、HKCU\Software\<厂商>\<产品>（安装位置/安装语言）、
;         HKCU ...\Uninstall\SnipAsk
; 目录用 RMDir /r 递归删除；被占用文件用 /REBOOTOK 兜底（重启后删除）。
; =============================================================================

; MUI2 已包含 LogicLib；显式引入以防本文件被单独 include（LogicLib 自带 include guard）。
!include LogicLib.nsh

; -----------------------------------------------------------------------------
; 实际清理逻辑。只在 NSIS_HOOK_POSTUNINSTALL 展开处插入，故可使用模板 define。
; 路径名 "SnipAsk" 与 src-tauri/src/platform/paths.rs 中的固定目录名保持一致
; （不随 productName 变化）；WebView2 目录使用模板的 ${BUNDLEID}（com.snipask.app）。
; -----------------------------------------------------------------------------
!macro SnipAskPurgeUserData
  ; 固定为当前用户视角，确保 $APPDATA / $LOCALAPPDATA 指向本用户的目录。
  SetShellVarContext current

  ; currentUser 模式下默认安装目录就是 $LOCALAPPDATA\SnipAsk（与数据根同名），
  ; 卸载器进程的当前目录可能仍停在该目录里；Windows 不允许删除进程的当前目录，
  ; 因此先把 $OUTDIR / 当前目录切到 $TEMP，再递归删除，避免 RMDir 只落 /REBOOTOK
  ; 甚至静默失败。
  SetOutPath "$TEMP"

  DetailPrint "SnipAsk: removing local user data (models, cache, config, history) ..."

  ; 配置 / 历史 / 日志 / 提示词 / 后端（含 backends\vulkan 等运行时组件）
  RMDir /r /REBOOTOK "$APPDATA\SnipAsk"
  ; 模型 / 缓存 / 运行时（含已下载的 gguf 模型）；currentUser 安装时这里也是 $INSTDIR
  RMDir /r /REBOOTOK "$LOCALAPPDATA\SnipAsk"
  ; WebView2 用户数据（Tauri BUNDLEID 目录）
  RMDir /r /REBOOTOK "$APPDATA\${BUNDLEID}"
  RMDir /r /REBOOTOK "$LOCALAPPDATA\${BUNDLEID}"

  ; 自启动值（模板在非更新模式已删；这里兜底，幂等）
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"

  ; 安装器写入的「安装位置 / 安装语言」（模板「删除应用数据」分支同款清理；
  ; 仅在本次确实要删除数据时执行，更新模式下不会走到这里）
  DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
  DeleteRegKey HKCU "${MANUPRODUCTKEY}"
  DeleteRegKey /ifempty HKCU "${MANUKEY}"

  ; 卸载注册项（模板 Section Uninstall 已删，幂等兜底）
  DeleteRegKey HKCU "${UNINSTKEY}"

  DetailPrint "SnipAsk: local user data removal finished."
!macroend

; -----------------------------------------------------------------------------
; Tauri 卸载器钩子：Section Uninstall 中所有文件/快捷方式/注册表清理之后执行。
; -----------------------------------------------------------------------------
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode = 1
    ; 覆盖安装/更新：卸载器由新版本安装器以 /UPDATE 调起，必须保留用户数据。
    DetailPrint "SnipAsk: update mode, user data is kept."
  ${ElseIf} ${Silent}
    ; 静默卸载 /S：无条件彻底清理。
    DetailPrint "SnipAsk: silent uninstall, removing user data."
    !insertmacro SnipAskPurgeUserData
  ${ElseIf} $DeleteAppDataCheckboxState = 1
    ; 交互卸载且用户勾选了「删除应用数据」。
    DetailPrint "SnipAsk: delete app data checkbox selected."
    !insertmacro SnipAskPurgeUserData
  ${Else}
    ; 交互卸载且未勾选复选框：明确询问，默认按钮为「是」。
    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON1 "是否同时删除本地数据（已下载的模型、配置、历史记录）？$\r$\nDelete local data (downloaded models, settings, history) as well?" /SD IDNO IDYES snipask_purge_yes IDNO snipask_purge_no
    snipask_purge_yes:
      !insertmacro SnipAskPurgeUserData
      Goto snipask_purge_done
    snipask_purge_no:
      DetailPrint "SnipAsk: user data is kept."
    snipask_purge_done:
  ${EndIf}
!macroend
