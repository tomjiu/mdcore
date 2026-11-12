# LOK 引擎边界与体积：ElyOffice / dsh-libreoffice-kit 实测对照

> 关联：#69（Windows LOK 链路）| PR #161
> 结论先行：**从 ElyOffice 已无可抄的编译期裁剪**；差距在打包格式与运行时裁剪。
> 本文所有数字均为本机实测，非公开资料转述。

## 1. 三个参照对象的实测体积

| 树 | 未压缩 | zip(deflate) | 7z(LZMA2) |
|---|---|---|---|
| 全量 `instdir` | 981.3 MB / 5,705 文件 | 305 MiB | — |
| 精简（去 64 国词典/外格式/Python） | 432.5 MB / 3,501 文件 | **158.4 MiB** | — |
| 引擎（再去 122 个未加载 DLL） | 377.9 MB / 3,388 文件 | — | **86.1 MiB** |

**同一棵树，zip 与 LZMA2 差 45%。** ElyOffice ElyDoc 的 Windows 安装包 117.6 MiB
就是 LZMA 安装器；按同口径我们 86.1 MiB 已经更小。

## 2. ElyOffice：没有可再抄的编译期裁剪

对比本机 `autogen.lastrun` 与 ElyOffice `scripts/build-libreoffice.sh`：

| 项 | ElyOffice | 我们 |
|---|---|---|
| `--enable-mergelibs` | ✓ | ✓ |
| `--without-java/junit/help/doxygen` | ✓ | ✓ |
| `--disable-odk/online-update/extension-integration` | ✓ | ✓ |
| `--disable-scripting-beanshell/javascript` | ✓ | ✓ |
| `--disable-firebird-sdbc/postgresql-sdbc` | ✓ | ✓ |
| `--disable-ext-nlpsolver/ext-wiki-publisher` | ✓ | ✓ |
| `--disable-report-builder/sdremote(+bluetooth)` | ✓ | ✓ |
| `--disable-poppler/lotuswordpro/gpgmepp` | ✓ | ✓ |
| `--with-galleries=no --with-theme=colibre --with-fonts` | ✓ | ✓ |
| `--disable-python/coinmp/lpsolve --with-myspell-dicts` | ✓ | ✓ |
| `--with-lang=`（无界面本地化） | ✓ | 我们 `=en-US` |
| `--disable-cli --without-dotnet --with-visual-studio=2022` | ✓（Cygwin 分支） | ✓ |

**差异只有 `--with-lang`。** 且 LO 没有 `--disable-writer/calc/impress`，
`--with-main-module=writer` 是 Emscripten/WASM 专用——原生 Windows 做不出
"Writer-only 发行版"，这是构建系统事实，不是选择。

ElyOffice 的后处理只有两步：只留 `dict-en`、删 Lightproof（Python 语法检查）。
我们已等价（只留 dict-en；Python 已整树移除）。

> 所以「从 ElyOffice 剪切多少」的答案是 **0**：它保留 Writer+Calc+Impress
> 三套应用，我们若只做 Writer 语义，省下的不是 configure 开关，而是
> **打包时选择不发布哪些模块**——那是 payload 裁剪，不是构建裁剪。

## 3. dsh-libreoffice-kit：真正可借的六件事

源码实测（`engine/native/configure.mjs`、`scripts/slim-native.mjs`、
`scripts/minimize-ui-resources.py`、`engine/native/worker.cxx`）。

### 3.1 构建期：静态化 + LTO + Oz（我们完全没有）

```
--disable-dynamic-loading          # 组件静态链进 worker，不再一堆 DLL
--enable-customtarget-components
--enable-lto                       # 链接时优化
--optimization=Oz  (clang-cl)      # Windows 用 clang-cl 而非 MSVC
--disable-pch --disable-symbols --disable-debug
--disable-nss --with-tls=openssl --disable-gpgmepp
--enable-pdfium --disable-pdfimport
--without-fonts --without-myspell-dicts
--with-templates=no --with-theme=no
--disable-extensions --disable-database-connectivity --disable-scripting
--disable-ldap --disable-libcmis --without-webdav --disable-curl --disable-breakpad
```

**这是体积的真正来源**：DLL 数量下降、代码被 LTO+Oz 压小、组件不注册。
代价是**必须全量重编**（本机约 10 小时）+ clang-cl 工具链。

### 3.2 打包期：删除清单（可直接抄，无需重编）

把它的清单套到我们树上实测：

| 项 | 我们树上占用 |
|---|---|
| `share/gallery` + `template` + `wizards` + `tipoftheday` + `xslt` | 20.4 MB |
| `share/basic` + `Scripts` + `presets/basic` | 4.6 MB |
| `program/` 23 个启动器（soffice.bin/各种 .exe/unopkg…） | 6.3 MB |
| `program/shlxthdl` + `shell`（Windows 外壳集成） | 1.4 MB |
| 被排除 DLL 23 个（clucene/ucp*/libcrypto/libssl/reg_*/pdfimportlo…） | 20.3 MB |
| `share/config/images*.zip` | 2.8 MB |
| **合计** | **53.4 MB** |

377.9 − 53.4 = **324.5 MB → LZMA ≈ 74 MiB**。

### 3.3 UI 资源最小化：比猜测强的方法论

`share/config/soffice.cfg` 在我们树上 **32.4 MB**，是 mergedlo/ICU/fonts 之后最大的单项。
dsh 的 `minimize-ui-resources.py` 不猜，用**证据驱动**：

1. 拿到完整 cfg；
2. 按"目录优先、再文件"贪心尝试删除；
3. 每次删除后**重新对新进程、新 profile、真实六格式语料做一次转换**；
4. 用 Poppler 抽出 PDF 的**页面几何 + 文字 + 词框**，算 SHA-256 比对；
5. 无变化才保留删除，反复迭代到稳定。

**这和我们验证 tile 逐字节相同的思路一致**，但目标换成了 UI 配置。可以直接借鉴。

### 3.4 删除前的静态自检（防假绿的关键一招）

`slim-native.mjs` 在删每个组件 DLL 之前断言：

```js
assert(!services.rdb.includes(`/${name}`),
  `Removed component remains registered: ${name}; reconfigure Core`);
```

即：**若该 DLL 仍登记在 `program/services/services.rdb`，拒绝删除**——
否则 LO 启动时加载到空实现，表现为"能开、某功能静默失效"。
我们之前是"删完跑测试"，漏了这一步；**应当补上这个前置断言**。

### 3.5 worker 形态：一次性转换进程，不是长活会话

我们：长活 JSON-lines 会话 worker（为交互编辑设计）。
它：`worker.cxx` 一次进程一次转换，参数即契约：

```
--input-path --output-path --format(pdf|...) --font-file(可多次)
--profile --scratch --max-output-bytes --recalculate(xlsx/ods)
operation = convert | render-images
```

自带安全约束，条条值得抄：
- 输出路径**必须绝对**，且**已存在即拒**（防覆盖用户文件）；
- 输出格式**白名单**校验；
- **`--max-output-bytes` 上限**（防解压炸弹/无限输出）；
- **`--font-file` 显式注册字体**——把字体策略交给调用方，构建期就能 `--without-fonts`。

## 4. 两条路线：体积 / 工作量 / 保真风险

| | A：保留 LO Writer 核心 + 裁剪运行时 | B：自建受控 DOCX 模型 + 自建排版 |
|---|---|---|
| 现在体积 | 86.1 MiB（LZMA，已实测） | 仅 mdcore 应用 ~12–23 MB |
| 可做到 | ~50–74 MiB（dsh 清单 + 重编 LTO/静态/无字体） | 本质上是自己写排版引擎 |
| 开发量 | 打包 + 构建参数 + 验证（周级） | 分页/表格/字体度量/编号/域…（年级） |
| 保真风险 | **低**：LO Writer 是 DOCX 排版的参考实现之一 | **高**：所有边缘行为要自己兜 |
| 编辑闭环 | LOK 交互编辑可做但难控（已有 117 个补丁在垫） | 天然受控，AI 与人工同一状态 |
| 未知内容 | 靠 LO 兜，不会"编辑一个段落毁掉整文档" | 必须显式拒绝，否则静默降级 |

**建议不是二选一，而是分工**：

- **编辑层**：mdcore 自有模型（受控 DOCX 子集），人工与 AI 共享同一状态——这是 B 的地盘，
  且我们已有基础（`docx_doc` 文本投影编辑、部分表格格编辑、保真保存）。
- **排版/输出层**：LO Writer 内核，按 dsh 形态做**一次性转换 worker**
  （input → PDF / render-images），不承担交互编辑职责 → 可以裁到极致（无 UI 配置、
  无字体、无词典、无脚本），并且可以用 `--max-output-bytes`、拒绝覆盖等硬约束。
- **预览层**：用同一个 worker 的 `render-images` 出页图，验证"编辑态与输出态一致"。

这样 `share/config` 32.4 MB、`fonts` 53 MB、`dict-en` 25.5 MB **全部可以不要**
（字体、词典改由调用方按需提供），体积落到 **~50 MiB** 量级，且不需要提前解决
"LOK 交互编辑"这个大坑。

> 若坚持 LOK 交互编辑（当前产品形态），则 dsh 的 `--without-fonts/--with-templates=no`
> 等裁剪不能照抄，体积停在 74–86 MiB 区间。

## 5. 立即可做（不重编，按收益排序）

1. **改 LZMA 分发**：158.4 → 86.1 MiB（已实测，零风险）。
2. **套 dsh 删除清单**：再省 53.4 MB（需按第 3.4 节补 services.rdb 前置断言 + 全链回归）。
3. **UI 配置最小化**：32.4 MB 里能砍多少要用第 3.3 节方法实测，预计一半以上。
4. **字体/词典策略**：若采用"排版输出与编辑分离"，53 + 25.5 MB 可整体移除。

需要重编才有的收益（LTO/Oz/静态链接/无 NSS）留作独立构建实验，按
`--optimization=Oz --enable-lto --disable-dynamic-loading` 三件套先跑一个对照构建。
