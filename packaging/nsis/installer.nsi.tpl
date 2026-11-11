; NSIS 安装器模板（#228 R4 重构：宿主壳 = mang-shell 主线，GPU 自绘渲染/DirectX 后端）
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
; 载荷语义：pdfium.dll 与 mdcore.exe 同目录安装 = 定位序 exe-dir 候选
;   （#223/#224）；安装后自检复用 --check-pdfium（退出码 0=已绑定 /
;   3=未绑定）。
; WebView2 面已移除（断链 2 处置 = 面移除）：主线壳 mang-shell 走
;   GPU 自绘渲染（DirectX 后端），无 WebView 运行时依赖；旧 tao+wry 壳（mdshell-native）
;   为 Linux GTK 面（windows-check.yml 排除注释同源），不随 Windows
;   安装包分发。处置记录见 BUILD-VERIFY §3。
; 注册表面（#143：HKCU 用户级 + 礼貌类型注册，ElyOffice
;   windows-types.sh 钉死行为对照）：
;   · 全程 HKCU（per-user 安装免管理员）——InstallDir/卸载键/类型注册
;     全在 HKCU；写 HKLM\Software\Classes 或 HKCR 会因权限在非管理员
;     会话直接失败，per-user 类型注册唯一正确落位 = HKCU\Software\Classes。
;   · 礼貌关联：.md 默认只在**无现有处理程序或已指向自己**时设置；
;     「新建」菜单随接管同走；他人登记不动（保留用户选择）。
;   · 「默认应用」面（不接管）：RegisteredApplications + Capabilities
;     只把应用**列进** Windows 设置的默认应用列表，接不接管由用户选择
;     ——Ely 假 scheme 面同构。
;   · 写不进注册表即安装失败（IfErrors → Abort），不静默继续。
;   · 卸载只清自己写入的键；.md 默认值仅当仍指向本程序 progid 才删；
;     载荷按文件删（换文件夹后旧卸载不删新装——卸载器 $INSTDIR =
;     自身所在目录，天然只作用旧安装位）。

!define APPEXEC "mdcore"
!define APPNAME "mang"
!define APPVERSION "{{VERSION}}"
!define APPID "dev.mdcore.editor"
; 类型 progid：mdcore 自研命名域（TRADEMARK_RULES §1 同源；旧模板裸
; "MarkdownFile" 与系统/他人 progid 撞名，卸载语义未定义——#143 换自有 ID）。
!define PROGID "dev.mdcore.editor.md"

Name "${APPNAME} ${APPVERSION}"
OutFile "{{OUT_FILE}}"
Unicode true
; per-user 落位（#143）：$LOCALAPPDATA\Programs = 用户级安装惯例位
; （VSCode 用户装/Chrome 同款），免管理员可写；$PROGRAMFILES64 需提权
; 与 HKCU 面矛盾，弃用。
InstallDir "$LOCALAPPDATA\Programs\${APPEXEC}"
InstallDirRegKey HKCU "Software\${APPID}" "InstallDir"
RequestExecutionLevel user

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "${APPNAME} (required)"
  SectionIn RO
  SetOutPath "$INSTDIR"
  ; 载荷（构建器注入的绝对路径——见模板头「路径面铁律」）
  File "{{PAYLOAD_EXE}}"
  File "{{PAYLOAD_DLL}}"

  ; 注册表面：开始菜单 + 默认程序（checklist D2）。
  ; shell 上下文显式钉 current（#143：per-user 安装写当前用户开始
  ; 菜单；NSIS 默认即 current，显式声明防回归）。
  SetShellVarContext current
  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\${APPEXEC}.exe"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Uninstall ${APPNAME}.lnk" "$INSTDIR\uninstall.exe"

  ; 软件面（全 HKCU，#143）：写失败即安装失败（不静默继续）。
  ClearErrors
  WriteRegStr HKCU "Software\${APPID}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}" \
                 "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}" \
                 "DisplayVersion" "${APPVERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}" \
                 "UninstallString" "$INSTDIR\uninstall.exe"
  IfErrors reg_write_failed
  Goto reg_write_done
reg_write_failed:
  MessageBox MB_ICONSTOP \
    "注册表写入失败（HKCU——常见于策略限制或配置单元损坏）。$\n$\n安装无法完成，未对系统做任何关联改动。"
  Abort
reg_write_done:

  ; .md 礼貌关联（#143，Ely 行为对照）：
  ;   先读 HKCR 合成视图（HKCU+HKLM 生效面）——空或已指向自己 →
  ;   接管（幂等重写，覆盖升级/换目录场景）；他人非空关联 → 不动。
  ReadRegStr $0 HKCR ".md" ""
  StrCmp $0 "" md_take 0
  StrCmp $0 "${PROGID}" md_take md_keep
md_take:
  ClearErrors
  WriteRegStr HKCU "Software\Classes\.md" "" "${PROGID}"
  WriteRegStr HKCU "Software\Classes\${PROGID}" "" "Markdown 文档（mang）"
  WriteRegStr HKCU "Software\Classes\${PROGID}\DefaultIcon" "" \
                 "$INSTDIR\${APPEXEC}.exe,0"
  WriteRegStr HKCU "Software\Classes\${PROGID}\shell\open\command" "" \
                 '"$INSTDIR\${APPEXEC}.exe" "%1"'
  ; 「新建」菜单随接管走（空 .md 新建项；他人接管的「新建」不动）
  WriteRegStr HKCU "Software\Classes\${PROGID}\ShellNew" "NullFile" ""
  IfErrors reg_write_failed
  Goto md_cap
md_keep:
  DetailPrint ".md 已有关联（$0）——保留用户选择，不接管（#143 礼貌注册）"
md_cap:
  ; 「默认应用」面（不接管）：应用进 Windows 设置的默认应用列表，
  ; 用户可自行选默认——Ely 假 scheme 进「默认应用」同构。
  ClearErrors
  WriteRegStr HKCU "Software\${APPID}\Capabilities" "ApplicationName" "${APPNAME}"
  WriteRegStr HKCU "Software\${APPID}\Capabilities" "ApplicationDescription" \
                 "mang —— PDF / DOCX / Markdown 文档编辑器"
  WriteRegStr HKCU "Software\${APPID}\Capabilities\FileAssociations" ".md" "${PROGID}"
  WriteRegStr HKCU "Software\RegisteredApplications" "${APPNAME}" \
                 "Software\${APPID}\Capabilities"
  IfErrors reg_write_failed

  ; 安装后部署自检（#224 能力复用）：pdfium 定位序逐层探针 + 绑定走查。
  ; 退出码合同：0 = 已绑定 / 3 = 未绑定（mang-shell main.rs 与
  ; mdc-pdf deploy_check 同源）。失败不回滚安装（Markdown/Word 面不依赖
  ; pdfium）——弹窗给复跑命令，报告全文进安装日志（详情页可见）。
  Call DeploySmoke

  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

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
  ; #143：卸载只清自己写入的面——
  ;   · 载荷按文件删 + 非递归收目录（RMDir 不带 /r：$INSTDIR 只在
  ;     已空时移除；用户放进同目录的文件与子目录一律不动；卸载器
  ;     $INSTDIR = 自身所在目录，换文件夹后新装不在其下，旧卸载
  ;     不删新的）。
  ;   · .md 默认值仅当仍指向本程序 progid 才清理（他人已接管 → 不动；
  ;     旧模板 RMDir /r $INSTDIR + HKCR MarkdownFile 裸删两缺陷弃用）。
  SetShellVarContext current
  Delete "$INSTDIR\${APPEXEC}.exe"
  Delete "$INSTDIR\pdfium.dll"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  RMDir /r "$SMPROGRAMS\${APPNAME}"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPID}"
  DeleteRegKey HKCU "Software\${APPID}"
  DeleteRegValue HKCU "Software\RegisteredApplications" "${APPNAME}"
  DeleteRegKey HKCU "Software\Classes\${PROGID}"
  ReadRegStr $0 HKCU "Software\Classes\.md" ""
  StrCmp $0 "${PROGID}" 0 md_keep_un
  DeleteRegKey HKCU "Software\Classes\.md"
md_keep_un:
SectionEnd
