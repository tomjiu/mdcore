#!/usr/bin/env python3
"""开 #485 D5 残差跟进 issue + PR #487 偏离报告评论"""
import json, subprocess, os, sys

TOKEN = os.environ['GH_TOKEN']
H = ['Authorization: token ' + TOKEN, 'Accept: application/vnd.github+json']
DRY = '--go' not in sys.argv

def api(path, method='GET', data=None):
    cmd = ['curl', '-s', '-X', method, '-H', H[0], '-H', H[1],
        f'https://api.github.com/repos/mmqz/mdcore{path}']
    if data is not None:
        cmd += ['-d', json.dumps(data)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return json.loads(r.stdout)

issue_body = """# P3: 表内/文本框段脚注引用位读面标注——三层支持收口（#485 D5 残差 3 件的根面）

> 登记人：Agent-3ivu（#485 CI 红根因分析过程，2026-11-08）

## 现状（#485 落地后的残差）

`crates/office-io/src/docx.rs` 的脚注引用位第二趟（`scan_note_references`）**只标注 body 顶层段**（`DocumentChild::Paragraph` 分支）。引用位落在以下位置的脚注不标注：

```
① 表内段（w:tbl → w:tr → w:tc → w:p）    —— scan 归表块索引但表块不消费（静默）
② 文本框内段（w:txbxContent）             —— scan 显式跳过（in_txbx）
③ mc:Fallback 复制区                      —— scan 显式跳过（合理——复制区）
```

`crates/mdcore-wasm/src/docx_import.rs`（#485）按 issue 契约**孤儿定义照发**（验收 1 点名），于是这三种位置的脚注在 md 里表现为「定义块在场、无引用位」= 孤儿定义。姊妹面 `docx_export.rs`（#488 已合）按 GFM 语义**孤儿定义不发射** → 往返丢定义块。

## 用户可见后果（D5 残差实证）

health_check 六维门 `roundtrip_exact` 基线随 #485 定向更新（85→83 / 388→387），丢的 3 件全部是本 issue 的机理：

| 件 | 语料 | 引用位落点 |
|---|---|---|
| `form_footnotes.docx` | POI | 表/文本框内 |
| `table_footnotes.docx` | POI | 表内 |
| `text-extraction_phase0_extraction-probe-inspect.docx` | docx4j | `NOTE63_UNREFERENCED`（真孤儿） |

注：docx4j 那件是真孤儿（document.xml 全文无引用），与①②不同——真孤儿是文档自身形态，md 内容保留已是诚实行为，本 issue 不含它。

## 修复面（供接手人——三层，建议按序独立可合）

```
层 1（引擎，office-io）：scan_note_references 的块索引从 body 一维扩展为
     (块索引, 表内段文档序) 二维键；表提取路径（extract_table 的 cell 段）
     透传标注。文本框内段同构（text_box_blocks 路径）。
层 2（import，mdcore-wasm）：Block::Table 分支的 cell 文本尾接 " [^N]"
     （与 body 段同口径——trim_end + 标签查表）。
层 3（export，mdcore-wasm）：表格发射路径从 table_header_repeat 的
     Vec<Vec<String>> 扁平文本面升级为 cell 级 RunSpec 切分
     （splice_runs 同谓词）——GFM 单元格单行面与脚注标记的相容性
     需先裁定（单元格文本含 [^N] 时 GFM 渲染语义）。
收口：三层齐后 hc_poi_docx roundtrip_exact 回 85、hc_docx4j 回 388
     （form_footnotes/table_footnotes 两件恢复；基线回补 + 根因指向本 issue）。
```

## 关联

- #485（import 接入——发现来源）、#488（export GFM 消费——孤儿不发射的语义面）
- 查重：全局搜「表内」「footnote_refs」「引用位」无同题 open issue"""

pr_body = """## Review 补充（Agent-3ivu——CI 红根因 + 修复交付，2026-11-08）

### CI 红根因（不是平台差异）

本地裸分支 7 连绿 vs CI 红（D5 82<85）的表象差异 = **pull_request 事件检出合并提交**（#485 import × #490 export 组合）；裸分支不含 #490 的 export 脚注消费。合并语义下本地复现成功后拆出两缺陷一边界：

1. **分离空格翻倍**（POI footnotes.docx）：import 尾接 ` [^N]` → export 切分标记后文本 run 保留分离空格 → 二轮 import 再尾接 → 双空格。修：尾接前 `trim_end` + 定向回归锁。
2. **定义块位置漂移**（docx4j 2010_w14_mcIgnorable）：定义块原在页脚尾接之前；md 无页脚概念，往返后页脚行降为正文段，二轮导入把定义块顶到页脚行后。修：定义块发射到**全文档最末尾**（定点稳定）——与 PR 描述的「正文之后、页脚尾接之前」有设计偏离，理由如上（往返稳定性）。
3. **边界（基线 85→83 / 388→387 的全部残差）**：表内/文本框引用位读面不标注（#485 登记边界）+ issue 验收 1 明令孤儿定义照发 + export 按 GFM 语义孤儿不发射 → 3 件往返丢定义。**非放宽**：md 投影信息量严格增长（脚注内容此前静默丢弃），其余五维与逐件 class 零变化，fixed_point 因缺陷①②修复保持 111 不降。跟进 issue 已开（表内/文本框三层支持）。

### 处置

- rebase 到含 #490 的最新 main（原基线 da29d0b 落后 50 commits，import/export 语义不闭环）
- `3f79125`：两缺陷修 + 基线定向更新 + 回归锁
- 本地实测（Linux / Rust 1.99.0 钉版）：`--lib` 140 全绿；health_check_docx 双节全绿；fmt CLEAN

### 需要 Lead 裁定的

1. 定义块位置改为「全文档最末尾」（偏离原描述「正文之后页脚之前」）——GFM 惯例与往返稳定性兼得，请追认
2. D5 基线 85→83 / 388→387 的语义演进口径（残差全部归因表内/文本框边界，跟进 issue 在档）"""

if DRY:
    print("[dry] 将开 issue:", issue_body[:80])
    print("[dry] 将评论 PR #487:", pr_body[:80])
else:
    r = api('/issues', 'POST', {
        'title': 'P3: 表内/文本框段脚注引用位读面标注——三层支持收口（#485 D5 残差 3 件的根面）',
        'body': issue_body,
        'labels': ['P3', 'area/office', 'type/wiring']})
    n = r.get('number')
    print(f"issue #{n} 已开: {r.get('html_url')}")
    if n:
        c = api('/issues/487/comments', 'POST', {'body': pr_body.replace('跟进 issue 已开（表内/文本框三层支持）', f'跟进 issue #{n}（表内/文本框三层支持）')})
        print(f"PR #487 评论: {'OK' if 'id' in c or 'number' in c else c}")
