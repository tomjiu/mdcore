# LO 裁剪实验重做轮 REPORT（2026-11-12）

> 产物指纹：lo-trimmed.tgz SHA256 `c4c273eb80806ad9ea09a1d948c66774166fc07aa04c41c75a39295afee8beac`（57.6MB）· lo-trimmed.tar.xz `1898b18013d01ff087bdfcdef617ebd61ee142a3213228c443cf0c79a346a157`（41.2MB）

> 回填缺口：`docs/architecture-audit/libreoffice-trim-analysis.md` 头注
> 「任务书引用的 981.3MB/377.9MB/86.1MiB 实验数据未在仓内找到对应
> 产物或脚本，按调查纪律标记未复核、待重做」——本轮以沙箱可复跑实验
> 重做替代，全部数据本仓可溯源。

## 0. 结论速览

| 口径 | 基线（Debian 13 全量安装树副本） | 裁剪后 | 降幅 |
|---|---|---|---|
| 未压缩树 | 294 MB | **174 MB** | −41% |
| tar.gz | 101.3 MB | **57.6 MB** | −43% |
| tar.xz（-9） | — | **41.2 MB** | — |

门口径：mdcore 真实 LOK 消费面两测试（`lok_smoke` 全链 26 步含
paintTile/saveAs/重开校验 + `export_pdf` 全文/页范围/单页）在裁剪终态
**全绿**（`final-gate-smoke.log` / `final-gate-pdf.log`）。

环境：Debian 13, LibreOffice 25.2.3.2（libreoffice-core 4:25.2.3-2+deb13u9
全家桶 27 包）；沙箱无 root，副本 staging 至
`/home/z/my-project/tmp/lo-trim/lo-trim-live`（整树复制自含
fundamentalrc 相对引用，`MDC_LOK_HOME` 指路即自洽）。

## 1. 三轮裁剪明细（每轮每项独立门裁决）

### R1a：share 资源类整批（`batches.jsonl`）

13 项一次删成：gallery(14M) template(9M) wizards(5M) basic(5M) xslt(5M)
palette(2M) wordbook labels tipoftheday uno_packages glade toolbarmode
theme_definitions。**294 → 257 MB**。

### R1b：program 非模块库逐库 bisect（`items.jsonl`）

55 候选逐库门裁决：**31 删成 / 1 必经回滚（`libloglo.so`）**。粗批教训
在档：34 库整批删门红（粗批混必经库），细化后定位唯一必经 = libloglo。
**257 → 212 MB**。

可删代表（门绿）：libcalc/libsc/libscui/libsd/libcui（calc/impress/
UI 模块内核）、libchart*、libabp/libacc/libanalysis/libanimcore/libbib、
libdb*/libsdbt（数据库面）、librpt*（报表）、libOGLTrans/
libPresentationMinimizer（演示面）、libflash/libwpft*（外部格式导入
面）、libcairocanvas/libvclcanvas（备选渲染后端——skia 主后端在场）、
libemb*（OLE 嵌入面）、writerfilter 旧库（libsw_writerfilter 新位保留）。

### R2：soffice.cfg 非 Writer 模块 UI 配置 + 图标包（`round2.log`）

21 项全删成：scalc/simpress/sdraw/smath/sabpilot/sbibliography/schart/
scanner/spropctrlr/StartModule/BasicIDE/db* 六面/sglobal/sweb/swxform
的模块 UI cfg + images_colibre*.zip 四资产包 + wizard。**212 → 193 MB**。
（LOK 无头面不吃 UI 配置与图标主题；postUnoCommand 走 UNO 分发不经 .cfg。）

### R3：残留明细面（`round3.log`）

11 项全删成：libscfilt（calc 过滤器）、libdbu（库 UI）、
liblocaledata_others（41 非 CORE locale）、libvbaobj/libvbaswobj（VBA
兼容对象）、libsm（公式内核）、unopkg/unopkg.bin/unoinfo（扩展管理
CLI）、xpdfimport（PDF 导入——mdcore PDF 面走自有 pdfium）、wizards。
**193 → 174 MB**。

## 2. 不可再删面（必经库实测清单）

| 库 | 体积 | 角色 |
|---|---|---|
| libmergedlo.so | 83 MB | 合并内核（UNO/SAL/VCL/Writer 核心入口全在内） |
| libswlo.so | 20 MB | Writer 模型/排版引擎 |
| libskialo.so | 7 MB | 渲染后端（paintTile 像素面） |
| libpdfiumlo.so | 4 MB | LO 内嵌 PDF 导出面（saveAs pdf 走此） |
| libmswordlo.so | 3 MB | DOCX 二进制导入（WW8） |
| libswuilo.so / libsw_writerfilterlo.so | 3+3 MB | Writer UI 桥 / OOXML 过滤器 |
| liblocaledata_euro.so | 4 MB | 欧面 locale（zh/en 数字日期格式必需） |
| libloglo.so | <1 MB | 日志面（删之 worker boot 即败——R1b 唯一必经教训） |

动态链接硬约束：这些库经 program/services.rdb 与 fundamentalrc 引用链
解析，删任何一个 → `documentLoad` 起即红。**未尝试**：libmergedlo 内部
strip（--strip-all 仅剥符号不动代码）、objcopy 分段——那是分发层打包
手段（strip 后预估再省 15-25%，未实测不下结论）。

## 3. 与任务书口径及 DSHKit/ElyOffice 路线的对照

- 任务书「377.9MB 删 DLL」对应 Windows 树（DLL 面更碎），本实验为
  Linux so 面口径，**不可直接换算**——量级结论一致：删非 Writer 模块
  面（calc/impress/draw/base/math + 资产）是最大单笔收益。
- 本实验 174MB 未压 / 57.6MB tar.gz，**落在 trim-analysis §4 预估带
  （250-350MB 未压 / 90-120MB 压缩）之下**：预估带按「保留完整
  LibreOffice-kit 双向兼容面」取宽，本轮按 mdcore 实际消费面（Writer
  编辑渲染 + docx/pdf saveAs）裁得更狠——印证「按消费面裁比按模块
  名单裁收益高」的路线判断。
- DSHKit 的「自动回滚删除失败项」不属实（其在档结论）——本实验的
  逐项门裁决 + 红即回滚机制即替代实现（`lo_trim_bisect.sh`）。

## 4. 复跑指引

```bash
# 0) 环境前置：Debian 13 + libreoffice 全家桶（apt install libreoffice）
# 1) staging 副本（294MB）
cp -a /usr/lib/libreoffice <工作区>/lo-baseline
# 2) 基线门（必须两门全绿才可信后续）
bash scripts/lok/lo_trim_gate.sh <工作区>/lo-baseline <工作区>/work
# 3) 三轮裁剪（顺序执行，每项独立门裁决、红即回滚）
bash scripts/lok/lo_trim_experiment.sh   # R1a 粗批（B3/B4 红属预期，脚本自回滚）
bash scripts/lok/lo_trim_bisect.sh       # R1b 逐库
bash scripts/lok/lo_trim_round2.sh       # R2 UI 配置面
bash scripts/lok/lo_trim_round3.sh       # R3 残留明细面
# 4) 终验 + 压缩口径
bash scripts/lok/lo_trim_gate.sh <工作区>/lo-trim-live <工作区>/work
tar czf trimmed.tgz -C <工作区> lo-trim-live && ls -l trimmed.tgz
```

脚本四件入仓 `scripts/lok/`：`lo_trim_gate.sh`（门）、
`lo_trim_experiment.sh`（R1a 驱动）、`lo_trim_bisect.sh`（逐库裁决）、
`lo_trim_round2.sh` / `lo_trim_round3.sh`（R2/R3 驱动）。

## 5. 本轮未做（如实）

- 未做 7z 口径（沙箱无 7z/p7zip；xz -9 = 41.2MB 已给下界参照）。
- 未做 strip/objcopy 分发层实验（见 §2 尾注）。
- 未做 Windows DLL 面复刻（无 Windows LO 安装树；任务书数据仍标
  未复核，仅 Linux 面已实测）。
- 未做「裁剪树捆绑进 deb/AppImage 分发」——那是 A 路线决策件，
  本轮只产出「裁到什么程度门不破」的实证边界。
