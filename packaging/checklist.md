# 分发合规核对单（R24-c；发布前逐项打勾，未全勾不得出包）

> 每次正式发布（含公测）逐项核验。勾选记录随发布 tag 归档
> （`packaging/release_<version>.md`）。门禁失败的项禁止跳过——
> 阻塞项走 TRADEMARK_RULES / COMPLIANCE 的对应流程。

## A. 命名与商标（TRADEMARK_RULES）

> **定名状态（2026-10-27）**：正式显示名 = **mang**（中文「芒」）。
> 所有者决议：**不注册域名、不承诺商用**——本软件仅以该名作显示名。
> 因此本节的「三件套一致」与「五步商标检索」按**非商用显示名**口径处理，
> 具体状态见下表；**若转为商用/公开发行，必须按原口径重跑 A1/A2**。

- [x] A1 显示名已定：`mang` / 芒（三件套中「域名」项按所有者决议**豁免**，
      「仓库组织名」维持 `mmqz`；豁免理由记录于 packaging/README.md §定名记录）
- [ ] A2 商标检索五步 —— **降级为风险登记**（记录：
      `compliance_release/trademark_search_2026-10-27.md`；
      机器化项已完成：crates.io/npm/GitHub/域名初筛；
      ⚠️ **CNIPA 9/42、WIPO、common-law 未查**——商用前必须补）
- [x] A3 定名后 denylist 近似判定零命中（`mang` 与 denylist 全部参考对象词
      均不近似；判定依据 = compliance/denylist/sources.txt 四源字集；
      CI 上 `scan.sh --all` 仍需复跑）
- [x] A4 `{{APP_NAME}}` 占位符已全量替换为 `mang`（5 个模板/脚本 + 门禁断言同步改写），
      packaging/README.md 变量表已登记决议
- [ ] A5 法律声明区归属句 —— **待 TRADEMARK_RULES §2.5 到位后执行**
      （该文件当前不在仓库中，见 README 流程缺口标注）

## B. 版本与产物

- [ ] B1 版本单源：`根 Cargo.toml [workspace.package].version` 为唯一
      手写点；`r24_packaging.rs` 一致性锁绿
- [ ] B2 发布产物版本 = `mdcore::VERSION`（deb control / NSIS / Info.plist /
      AppImage 元数据四处注入，无手抄）
- [ ] B3 tag 纪律：`v<VERSION>`，与 Cargo.lock 一起冻结（`--locked` 构建）
- [ ] B4 三平台安装 + 首次启动 + 卸载冒烟（R24-d 执行：AppImage/deb、
      NSIS、.app）

## C. 依赖与合规（COMPLIANCE.md §4 常驻门禁）

- [ ] C1 `bash compliance/scan.sh` 产品面 0 命中（含 packaging/）
- [ ] C2 `cargo test --workspace` 全绿（语义锁零回归）
- [ ] C3 SBOM 重跑（宿主壳启用 wry/tao 后依赖面变化），
      THIRD-PARTY-NOTICES 版本一致、全白名单
- [ ] C4 wasm32 `cargo check` 通过
- [ ] C5 图标/资产经 ASSET_CHECK 登记（来源可证，无第三方侵权素材）

## D. 平台注册面

- [ ] D1 Linux：`.desktop` 入口 + `text/markdown` mime 关联 +
      `x-scheme-handler/mdcore` scheme 注册（update-desktop-database /
      update-mime-database 刷新验证）
- [ ] D2 Windows：NSIS 安装/卸载注册表面（#143 起 HKCU 用户级：免管理员，
      InstallDir=$LOCALAPPDATA\Programs；礼貌类型注册——.md 只进「打开
      方式」列表，默认与「新建」仅在无主时写入，写不进即安装失败；
      开始菜单快捷方式 + 安装后 pdfium 部署自检 `--check-pdfium` 冒烟
      ——#228 起安装包捆载 pdfium.dll 并复用 #224 自检；WebView2 面已随
      主线壳 mang-shell 的 GPU 自绘（DirectX 后端）架构移除。实机验证
      面见 BUILD-VERIFY §3 复核清单。跟进件加「默认应用」面：应用列进
      Windows 设置默认应用列表（RegisteredApplications + Capabilities，
      跟进 PR 登记）；结构面钉子 =
      `packaging/tests/test_ns_polite_registration.py`（20 项，Linux/Win
      双平台），编译面 = makensis 3.11（BUILD-VERIFY §3 口径））
- [ ] D2a R24-d 预装关联验证（#143 登记五条，真机窗口执行）：
      ① 装前设 `.md` = 别的程序 → 安装后 .md 关联**不动**（礼貌不接管）
      ② 干净机安装 → .md 无主时归本程序且「新建」菜单含空 Markdown 项
        （悬空 progid 情形同判无主）
      ③ 安装后 Windows「设置 → 默认应用」列表可见 mang（Capabilities
        面）且「打开方式」菜单含 mang（OpenWithProgids 面）
      ④ 换文件夹重装 → 运行旧目录 uninstall.exe 只清旧目录与新登记无涉
        （open 命令所有权闸）
      ⑤ 卸载 → .md 默认值/「新建」仅当仍归本程序时清理；「默认应用」
        列表登记随卸载消失（RegisteredApplications 值删除）
- [ ] D3 macOS：Info.plist 文档类型（.md）+ URL scheme（mdcore://）声明，
      Gatekeeper 冒烟（未签名时的用户引导文案）

## E. 隐私与安全

- [ ] E1 自定义协议面只服务本地 bundle（mdcore://app 无网络面——
      CONTRACT §2，发布版 webview 网络策略复核）
- [ ] E2 无遥测/无自动上传（v0 产品语义；如后续引入须 ADR + 明示）
- [ ] E3 崩溃转储/日志本地化（不外发）

## F. 发布物料

- [ ] F1 Release notes（版本 + 变更 + 已知问题）
- [ ] F2 校验和（SHA-256 per artifact）随发布公布
- [ ] F3（可选）签名/公证：macOS notarization、Windows Authenticode
- [ ] F4 EULA/许可文本（闭源 EULA 起草 = ADR-C1 挂起项，发布前完成）
