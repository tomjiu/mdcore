#!/usr/bin/env python3
"""创建 PR #408"""
import json, subprocess, os

TOKEN = os.environ['GH_TOKEN']

body = """**提交者**：Agent-3ivu

## 交付概要

PR for #408。分支 `fix/408-stamp-signature-gate`（基于 main @ 65f0834，零落后），1 commit，2 文件 +233/-4（crates/pdf/src/stamp.rs + tests/pdfh_stamp.rs）。

## 做了什么（对照 #408 验收）

| #408 任务 | 交付 | 证据 |
|---|---|---|
| 第一步 加签名检测 | ✅ 三路判定 `doc_declares_signature`：① catalog `/Perms /DocMDP` 在位（直接字典/间接引用两形态）；② `/AcroForm /SigFlags` bit0 置位；③ 字段表 `/FT /Sig`（`/Kids` 递归，限深 16 防环）——兜底：部分生成器不写 SigFlags（`fixtures/signed_sample.pdf` 即是）。宽拒窄放：误拒由显式放行兜底；误放（真签名档静默毁签）不可逆 | stamp.rs `doc_declares_signature` / `field_tree_has_sig` / `as_dict` |
| 第二步 API 面 | ✅ `StampOptions::allow_break_signature: bool`（默认 `false`——fail-closed），doc 注明「放行后签名必然失效（如实发生）」+ 桌面确认框引擎侧锚点语义 | stamp.rs `StampOptions` |
| 第三步 桌面提示 | ⚠️ **N/A（如实）**：stamp 当前**零调用方**（`git grep stamp:: -- crates/ mang-shell/` 无命中）——桌面无盖章入口可挂弹窗。`SignedDocument` 的 Display 文案（「文档已签名（盖章被拒——全量重写将毁签…）」）即未来 UI 提示的引擎侧锚点；接线轮挂菜单时直接消费此错误 | 见下方「未做的」 |
| 验收 1 默认拒 | ✅ | `signed_pdf_stamp_is_rejected`（pyhanko 真结构签名夹具） |
| 验收 2 显式放行可盖 | ✅ 产物含 `/XObject`、字节增长（签名确实失效——如实） | `signed_pdf_stamp_succeeds_when_break_explicit` |
| 验收 3 未签名正常 | ✅ 既有全套 small.pdf 用例不重复（13 过含原有 11） | `pdfh_stamp.rs` 全套 |
| 验收 5 R2 样张 | ✅ `fixtures/signed_sample.pdf`（pyhanko 真 PKCS#7/CMS + X.509，pdfi_signature 同款夹具） | 同上 |

### 结构对齐（不改既有语义）

- **门序**更新为：S3-L7 输入 → 参数校验 → S5 加密双拒 → **#408 签名门** → 写面（open 家族入口统一不变）
- `StampError::SignedDocument` 新变体 + Display（窄面，不污染 CommandError——同枚举纪律）
- organize.rs 的「声明毁签」路径与 writer_lopdf 的「增量保签」路径**均未触碰**（对比写进 doc 注释）

### AGENTS 0.0.5 合规（结构体加字段）

`StampOptions` 加字段后全仓字面量构造点 = 3 处，全部同步：impl Default（新字段）+ stamp.rs 内联测试 + pdfh_stamp.rs:105 均改 `..Default::default()` 形态；其余 9 处调用全为 `StampOptions::default()` 不受影响。grep 证据在本地已跑。

## 实测证据（本机 Linux，rustc 1.99.0 = 仓钉版本）

```
cargo test -p mdc-pdf --features engine-lopdf
  pdfh_stamp.rs:      13 passed / 0 failed（含新增 2 条验收）
  stamp 内联单测:      3 passed（含新增三路分支判定 + 控制组 /FT /Tx 不误伤）
  全套件 22 个 Running 全部 0 failed

cargo fmt -p mdc-pdf -- --check        → exit 0（fmt 门禁绿）

bash compliance/scan.sh                → FAIL：main 继承既有项
  （hit-paths=2 与 main @ 65f0834 完全一致——本 PR 不触 packaging/，
  零新增命中；既有项 = #359/#362，PR #412 在途修复中）
```

## 未做的 / 有偏离的

1. **验收 4（桌面弹窗）未做**：stamp 零调用方，无 UI 入口可挂（issue 第三步的前置是「用户点盖章」这个入口本身还不存在）。已在 issue 侧如实登记，建议接线轮（stamp 菜单接入时）补弹窗——错误类型与文案已备好。
2. engine-pdfium 面未跑（本机无原生 pdfium 库）——本 PR 只动 lopdf 面，pdfium 渲染面无接触。

## 需要 Lead 裁定的

无（桌面弹窗后置是唯一偏离，属结构性 N/A 而非决策项；如 Lead 要求本 PR 内带 UI，请指出入口设计）。"""

r = subprocess.run(['curl', '-s', '-X', 'POST',
    '-H', f'Authorization: token {TOKEN}',
    '-H', 'Accept: application/vnd.github+json',
    'https://api.github.com/repos/mmqz/mdcore/pulls',
    '-d', json.dumps({
        'title': 'feat(#408): stamp 签名门——已签名档盖章 fail-closed 拒 + allow_break_signature 显式放行',
        'head': 'fix/408-stamp-signature-gate',
        'base': 'main',
        'body': body,
    })], capture_output=True, text=True)
d = json.loads(r.stdout)
print('PR #' + str(d.get('number')), '|', d.get('html_url'))
print('state:', d.get('state'), '| base:', d.get('base', {}).get('ref'))
if d.get('number') is None:
    print('ERROR:', d.get('message'))
