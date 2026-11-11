; NSIS 安装器模板（#228 R4 路径面 + #143 行为面：HKCU 用户级 + 礼貌类型注册）
; 注入（packaging/nsis/build-installer.py 渲染，残留占位符/不一致即失败
;   ——链路可验证，断链 1 修复）：
;   {{VERSION}}      根 Cargo.toml [workspace.package].version（=
;                    mdcore::VERSION 单源，r24 锁背书；断链 1）
;   {{OUT_FILE}}     安装包产物绝对路径（build/nsis/mdcore-setup-<ver>.exe）
;   {{PAYLOAD_EXE}}  mdcore.exe 绝对路径（= mang-shell.exe staging 重命名）
;   {{PAYLOAD_DLL}}  pdfium.dll 绝对路径（定位序取件位 staging）
; 路径面铁律：NSIS 的 File/OutFile 按脚本目录而非调用 CWD 解析（探针
;   实证：脚本在 X 目录、cwd=Y，File "a/f" 找 X/a/f、OutFile 落 X/——
;   BUILD-VERIFY §3）；旧模板 "..\.." 反跳正是断链 3 的根源。故本模板
;   一律用构建器注入的绝对路径，任何调用目录/平台零歧义。
; 显示名 mang（A4 定名 2026-10-27，packaging/README 定名记录）；可执行名/
;   安装目录用自研命名域 mdcore（TRADEMARK_RULES §1）；APPID 恒
;   dev.mdcore.editor（r24_packaging.rs 门锁锚）。
;
; ── #143 安装行为面（ElyOffice packaging/windows/installer.nsi + types.sh
;    语义；其行为由 tests/scripts/windows-types.sh 钉死，我方实机验证
;    清单见 BUILD-VERIFY §3 与 checklist D2）──────────────────────────
;   1. 全程 HKCU 用户级：RequestExecutionLevel user（免管理员）、
;      InstallDir=$LOCALAPPDATA\Programs、卸载键/类型注册全在 HKCU。
;      旧 HKLM 机器级安装不在本模板迁移面（0.x 无发行用户；旧装机用
;      旧版卸载器自行清理）。
;   2. 礼貌类型注册：自有 progid（mdcore.md）只进「打开方式」列表
;      （.md\OpenWithProgids 值），绝不动用户已有默认处理程序；.md
;      默认值与「新建」菜单仅在无主时写入（默认空 / 默认值指向不存在
;      的类 / 已归自己——Ely types.sh 的 handlerless 判定）。
;   3. 写不进即失败：类型面逐写检查，首败键记名（$TypeFailKey）→
;      弹窗 + Abort（安装失败，非静默继续）。
;   4. 卸载所有权：progid 的 open 命令仍指向本安装目录才清理类型面
;      （换目录重装后，旧卸载器不动新安装的登记）；文件面只删本安装
;      写入的载荷，目录非空则留（用户自放文件不删）。
;   5. SHChangeNotify(SHCNE_ASSOCCHANGED)：类型面变更后通知 Explorer。
; 载荷语义：pdfium.dll 与 mdcore.exe 同目录安装 = 定位序 exe-dir 候选
;   （#223/#224）；安装后自检复用 --check-pdfium（退出码 0=已绑定 /
;   3=未绑定）。
; WebView2 面已移除（断链 2 处置 = 面移除）：主线壳 mang-shell 走
;   GPU 自绘渲染（DirectX 后端），无 WebView 运行时依赖。

!include "LogicLib.nsh"

!define APPEXEC "mdcore"
!define APPNAME "mang"
!define APPVERSION "{{VERSION}}"
!define APPID "dev.mdcore.editor"
; 自有 progid：独立命名域（mdcore.md），与任何既有「MarkdownFile」类
; 互不相扰——礼貌注册的前提是不借用别人的类名。
!define PROGID "mdcore.md"
; HKCU 下的 per-user 类注册面（Vista+ 与 HKLM 面合并进 HKCR 生效视图）
!define CLASSES "Software\Classes"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}"

Name "${APPNAME} ${APPVERSION}"
OutFile "{{OUT_FILE}}"
Unicode true
; #143（Ely 参照）：安装器 UI 自身按 PerMonitorV2 声明——HiDPI 下安装
; 向导不糊（应用本体 DPI 面由 exe manifest 自理，与此无关）。
ManifestDPIAwareness PerMonitorV2
; #143（Ely 参照）：/SOLID lzma——载荷整包固态压缩（pdfium.dll 数 MB
; 级，收益实际）。
SetCompressor /SOLID lzma
; #143：per-user 安装三件——免管理员 / 用户级程序目录 / HKCU 记忆位。
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APPEXEC}"
InstallDirRegKey HKCU "Software\${APPID}" "InstallDir"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Var Cmd
Var TypeFailKey
Var Tmp0
Var Tmp1

; 检查式写值（类型面专用）：失败即记名，只记第一个失败键——Ely types.sh
; 的 blame 语义（后续组以 $TypeFailKey 为闸，首败后续不脏写）。
!macro TypePut KEY NAME VALUE
  ClearErrors
  WriteRegStr HKCU "${KEY}" "${NAME}" "${VALUE}"
  ${If} ${Errors}
  ${AndIf} $TypeFailKey == ""
    StrCpy $TypeFailKey "${KEY}"
  ${EndIf}
!macroend

Section "${APPNAME} (required)"
  SectionIn RO
  ; #143：开始菜单等 shell 目录按当前用户解析（用户级安装配套）。
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  ; 载荷（构建器注入的绝对路径——见模板头「路径面铁律」）
  File "{{PAYLOAD_EXE}}"
  File "{{PAYLOAD_DLL}}"
  ; 卸载器先落位：后续任何一步失败（类型面 Abort），本次安装都可从
  ; 「应用与功能」完整移除（Ely 同序：文件 → 卸载器 → 注册面 → 类型面）。
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; 注册表面：安装目录记忆 + 卸载项（Add/Remove 面板；#143 起 HKCU）
  WriteRegStr HKCU "Software\${APPID}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${APPVERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APPEXEC}.exe,0"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1

  ; 开始菜单（checklist D2）
  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\${APPEXEC}.exe"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk" "$INSTDIR\uninstall.exe"

  ; .md 类型登记（#143 礼貌语义，详见 RegisterTypes 头注）
  Call RegisterTypes
  ${If} $TypeFailKey != ""
    MessageBox MB_ICONSTOP "文件类型登记失败：写不进 $TypeFailKey。$\n$\n安装中止（未完成的登记不会半途而废）。$\n文件已复制到 $INSTDIR；可重跑安装器重试，或用其中的卸载器完整移除。"
    Abort
  ${EndIf}

  ; 安装后部署自检（#224 能力复用）：pdfium 定位序逐层探针 + 绑定走查。
  ; 退出码合同：0 = 已绑定 / 3 = 未绑定（mang-shell main.rs 与
  ; mdc-pdf deploy_check 同源）。失败不回滚安装（Markdown/Word 面不依赖
  ; pdfium）——弹窗给复跑命令，报告全文进安装日志（详情页可见）。
  Call DeploySmoke
SectionEnd

; ── .md 礼貌类型注册（#143；Ely types.sh 的 .md 单类型手写版）────────
; 不变量：①不动用户已有默认（含 HKLM 机器级登记——HKCR 合并视图读到
; 有主即跳过）；②「打开方式」注册只加自己的值名，别家登记原样；
; ③「新建」菜单仅随无主接管出现（NullFile = 空文件模板，零载荷——
; Markdown 空文件即惯例，无需实体模板资产）；④逐写检查，写不进即失败。
Function RegisterTypes
  StrCpy $TypeFailKey ""
  StrCpy $Cmd '"$INSTDIR\${APPEXEC}.exe" "%1"'

  ; ① 自有 progid（独立命名域 mdcore.md）
  !insertmacro TypePut "${CLASSES}\${PROGID}" "" "mang Markdown Document"
  !insertmacro TypePut "${CLASSES}\${PROGID}\DefaultIcon" "" "$INSTDIR\${APPEXEC}.exe,0"
  ; open 命令带内嵌引号，走变量传值（NSIS 单引号串含双引号）
  ClearErrors
  WriteRegStr HKCU "${CLASSES}\${PROGID}\shell\open\command" "" $Cmd
  ${If} ${Errors}
  ${AndIf} $TypeFailKey == ""
    StrCpy $TypeFailKey "${CLASSES}\${PROGID}\shell\open\command"
  ${EndIf}

  ; ② 「打开方式」列表 + 应用登记（FriendlyAppName 面；不动默认处理程序）
  !insertmacro TypePut "${CLASSES}\Applications\${APPEXEC}.exe" "FriendlyAppName" "${APPNAME}"
  ${If} $TypeFailKey == ""
    ; 值名 = 自有 progid：与他应用在 OpenWithProgids 里的值名互不相扰
    !insertmacro TypePut "${CLASSES}\.md\OpenWithProgids" "${PROGID}" ""
    !insertmacro TypePut "${CLASSES}\Applications\${APPEXEC}.exe\SupportedTypes" ".md" ""
  ${EndIf}

  ; ③ 默认值 + 「新建」：仅当 .md 无主（HKCR = HKCU/HKLM 合并生效视图）
  ${If} $TypeFailKey == ""
    ClearErrors
    ReadRegStr $Tmp0 HKCR ".md" ""
    ${If} $Tmp0 != ""
      ; 非空默认值还须命名的类真的存在（有子键）才算有主——
      ; 默认值指向已删除的类 = 谁也没在处理（Ely types.sh 同判）
      EnumRegKey $Tmp1 HKCR "$Tmp0" 0
    ${EndIf}
    ${If} $Tmp0 == ""
    ${OrIf} ${Errors}
    ${OrIf} $Tmp0 == "${PROGID}"
      !insertmacro TypePut "${CLASSES}\.md" "" "${PROGID}"
      !insertmacro TypePut "${CLASSES}\.md\ShellNew" "NullFile" ""
    ${EndIf}
  ${EndIf}

  ; ④ 通知 Explorer 关联已变（SHCNE_ASSOCCHANGED）
  System::Call "shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)"
FunctionEnd

Function DeploySmoke
  DetailPrint "部署自检：${APPEXEC}.exe --check-pdfium ..."
  nsExec::ExecToLog '"$INSTDIR\${APPEXEC}.exe" --check-pdfium'
  Pop $0
  ; 退出码 0 = pdfium 已绑定；其余（含 3=未绑定、负值=无法启动）走告警。
  IntCmp $0 0 smoke_ok smoke_fail smoke_fail
smoke_fail:
  MessageBox MB_ICONEXCLAMATION "部署自检未通过（退出码 $0）。$\n$\nPDF 面暂不可用（Markdown / Word 面不受影响）。$\n修复指引：在命令行运行安装目录（$INSTDIR）下的 ${APPEXEC}.exe --check-pdfium，查看逐层探针与修复指引。"
smoke_ok:
  DetailPrint "部署自检退出码：$0"
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  StrCpy $Cmd '"$INSTDIR\${APPEXEC}.exe" "%1"'

  ; ── 类型面：所有权验证后清理（#143；Ely ElyTypesUninstall 语义）────
  ; progid 的 open 命令仍指向「本安装目录」才动类型面——换目录重装后，
  ; 旧卸载器不删新安装的登记（「旧卸载不删新的」）。
  ReadRegStr $Tmp0 HKCU "${CLASSES}\${PROGID}\shell\open\command" ""
  ${If} $Tmp0 == $Cmd
    ; 「打开方式」登记（只删自己的值名；别家的原样）
    DeleteRegValue HKCU "${CLASSES}\.md\OpenWithProgids" "${PROGID}"
    ; 默认值与「新建」：仅当仍归自己（用户事后改选别的默认 → 不动）
    ReadRegStr $Tmp0 HKCU "${CLASSES}\.md" ""
    ${If} $Tmp0 == "${PROGID}"
      DeleteRegValue HKCU "${CLASSES}\.md\ShellNew" "NullFile"
      DeleteRegKey /ifempty HKCU "${CLASSES}\.md\ShellNew"
      DeleteRegValue HKCU "${CLASSES}\.md" ""
    ${EndIf}
    ; 应用登记与 progid 最后删（此时已无登记再引用它）
    DeleteRegKey HKCU "${CLASSES}\Applications\${APPEXEC}.exe"
    DeleteRegKey HKCU "${CLASSES}\${PROGID}"
    System::Call "shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)"
  ${EndIf}

  ; ── 文件面：只删本安装写入的载荷；目录非空则留（用户自放文件不删）──
  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"
  Delete "$INSTDIR\${APPEXEC}.exe"
  Delete "$INSTDIR\pdfium.dll"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  ; ── 注册面：卸载键 + 安装目录记忆（HKCU）────────────────────────────
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\${APPID}"
SectionEnd
