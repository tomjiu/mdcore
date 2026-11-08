//! #413 —— DOCX 修订标记写面（`w:ins` / `w:del` 输出）。
//!
//! ## 定位：redline 的硬前提
//!
//! office-io 读面早已提取修订标记（`w:ins` = 当前生效文本 +
//! [`crate::docx`] RunData::revision_inserted 标记；`w:delText` =
//! 已删除内容不提取 + [`crate::io::WarningKind::RevisionDelete`]
//! 计数——#17/#343 在档），但**写面完全缺失**：文档 diff 引擎
//! （#414）产出 redline 时，插入块/删除块无从落成 Word 修订标记。
//! 本模块补写面：[`crate::docx::DocxWriter`] 的修订段发射方法
//! （在本模块对 DocxWriter 的分文件 impl——[`crate::style`] /
//! [`crate::numbering`] 同款 impl 桥先例）。
//!
//! ## 读写对称（往返闭环）
//!
//! ```ignore
//! let mut w = DocxWriter::new();
//! w.heading(1, "第二章 报价构成")?;
//! w.insert_run_revised("新增：质保期延至 5 年", &RevisionMark::new("评审人A", "2026-11-07T00:00:00Z"));
//! w.delete_run_revised("原报价有效期 30 天", &RevisionMark::new("评审人A", "2026-11-07T00:00:00Z"));
//! let bytes = w.save_to_vec()?;
//! // 读回（读面 #17 语义）：
//! // - ins 文本在读回段落 + run 带 revision_inserted = true
//! // - del 文本不在读回文本 + WarningKind::RevisionDelete ≥ 1
//! ```
//!
//! 往返验收 = 写出的必须能被自己的读面读回（issue 验收 3），锁在
//! `tests/docx_revision_write.rs`。
//!
//! ## 与 issue API 草图的三处偏差（如实登记）
//!
//! 1. **`RevisionMark` 无 `kind` 字段**——kind 由方法面承载
//!    （[`DocxWriter::insert_run_revised`] / [`DocxWriter::delete_run_revised`]），
//!    字段化会引入「`delete_run_revised` + `kind=Insert`」的不一致态
//!    （调用方与校验面双份真相）；
//! 2. **`RevisionMark` 无 `id` 字段**——docx-rs 0.4.22 `Insert`/`Delete`
//!    无公开 `w:id` 面（HistoryId 全局计数器在 XML 序列化时自动注入，
//!    源级一手核验：documents/elements/insert.rs `generate()`）；携带
//!    不生效的字段 = 谎言 API。验收「id 正确保留」按产物 XML 断言
//!    `w:id` 在场且每个标记唯一（tests/docx_revision_write.rs）；
//! 3. **`date` 为 `String`（ISO 8601）而非 `chrono::DateTime<Utc>`**——
//!    office-io 依赖闭包 = calamine/serde_json/zip/docx-rs（零 chrono，
//!    依赖纪律），OOXML `w:date` 本就是 W3CDTF 字符串面（schema
//!    ST_DateTime），字符串承载无损。**#586 补面**：docx-rs `date()`
//!    原始字节拼入 XML 属性（无第二层转义——研究仓 round47 源级证据），
//!    含 `"` 的串可闭合属性注入伪造属性——发射前经
//!    [`validate_revision_date`] 共享边界把关（语法白名单，非法
//!    拒入粘性错误漏斗，save 零产物）。
//!
//! ## 边界（issue「不做」清单 + 本模块补充）
//!
//! - 不做 `w:rPrChange`/`w:pPrChange`（格式变更修订——另开 issue）；
//! - 不做 `w:moveFrom`/`w:moveTo`（移动修订可用 del+ins 降级——issue
//!   明列）；
//! - 富文本修订**已扩面**（#556：[`DocxWriter::insert_runs_revised`]/
//!   [`DocxWriter::delete_runs_revised`] 多 run + rPr 保留）；**新边界**
//!   = `link`/`anchor`/`footnote`/`field` 载荷——引擎修订容器 child 面
//!   （docx-rs 0.4.22 `InsertChild`/`DeleteChild` 仅 Run/Delete/Comment）
//!   无对应形态，V13 粘性拒（save 漏斗零产物，不伪称支持）。

use crate::docx::{spec_run_property, DocxWriter, RunSpec};

/// 修订标记元数据（`w:ins`/`w:del` 的 `w:author` + `w:date` 承载）。
///
/// `author` 落 OOXML 属性（XML 转义由 docx-rs `author()` 构造器内聚
/// ——escape_owned 源级在档）；`date` 经 [`validate_revision_date`]
/// 共享边界把关（#586：docx-rs `date()` 无第二层转义，语法白名单
/// 拒注入面字符）；`w:id` 由 docx-rs HistoryId 自动分配（见模块文档
/// 偏差 2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionMark {
    /// 修订作者（`w:author`——Word 审阅窗格显示名）。
    pub author: String,
    /// 修订时间（`w:date`——W3CDTF / ISO 8601 完整形态，如
    /// `2026-11-07T08:30:00Z`；#586 起发射前校验，非法拒入
    /// 粘性错误漏斗——**空串拒绝**：「不关心时间」用
    /// [`RevisionMark::default`] 的 epoch 兜底，不用空串占位）。
    pub date: String,
}

impl RevisionMark {
    /// 构造（author 直入 / date 发射前经共享边界校验——构造期不拒，
    /// 违规在 [`DocxWriter::insert_run_revised`] /
    /// [`DocxWriter::delete_run_revised`] 落粘性错误，save 零产物）。
    pub fn new(author: impl Into<String>, date: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            date: date.into(),
        }
    }
}

impl Default for RevisionMark {
    /// docx-rs 同款缺省（`unnamed` / epoch）——可诊断的兜底，不是
    /// 隐藏的自动行为。
    fn default() -> Self {
        Self {
            author: "unnamed".to_owned(),
            date: "1970-01-01T00:00:00Z".to_owned(),
        }
    }
}

impl DocxWriter {
    /// 发射**插入修订段**：`<w:p><w:ins w:id w:author w:date><w:r>
    /// <w:t>text</w:t></w:r></w:ins></w:p>`。
    ///
    /// 语义（与读面 #17 对称）：`w:ins` 内 = **当前生效文本**——读回
    /// 时正常提取且 run 带 `revision_inserted = true`（Word 显示为
    /// 插入修订，带下划线着色）。
    ///
    /// #586：`mark.date` 发射前经 [`validate_revision_date`] 共享边界
    /// 把关——非法拒入粘性错误（save 漏斗零产物，V7 同纪律）。
    ///
    /// r4-t2 计数：1 段 + 1 run（与 [`DocxWriter::page_break`] 同为
    /// 非 RunSpec 发射面——按发射面诚实计数，走 save 漏斗上限）。
    pub fn insert_run_revised(&mut self, text: &str, mark: &RevisionMark) {
        // #586：共享日期边界（ins/del × 纯文本/富文本四调用点同源）。
        if let Err(msg) = validate_revision_date(&mark.date) {
            self.record_validation_error(format!(
                "修订日期非法（insert_run_revised：author={:?} date={:?}）：{msg}",
                mark.author, mark.date
            ));
        }
        let run = docx_rs::Run::new().add_text(text);
        let ins = docx_rs::Insert::new(run)
            .author(&mark.author)
            .date(&mark.date);
        let p = docx_rs::Paragraph::new().add_insert(ins);
        let docx = std::mem::take(self.docx_mut());
        *self.docx_mut() = docx.add_paragraph(p);
        self.count_raw_emission(1, 1);
    }

    /// 发射**删除修订段**：`<w:p><w:del w:id w:author w:date><w:r>
    /// <w:delText>text</w:delText></w:r></w:del></w:p>`。
    ///
    /// 语义（与读面 #17 对称）：`w:delText` = **已删除内容**——读回
    /// 不提取、逐段登记 `WarningKind::RevisionDelete`（Word 显示为
    /// 删除线修订）。注意 run 内是 `w:delText` 而非 `w:t`（OOXML
    /// 删除修订的承载约定——`w:t` 在 `w:del` 内是非法形态）。
    ///
    /// #586：`mark.date` 发射前经 [`validate_revision_date`] 共享边界
    /// 把关——与 [`Self::insert_run_revised`] 同源同语义。
    ///
    /// r4-t2 计数：1 段 + 1 run（同 [`DocxWriter::insert_run_revised`]）。
    pub fn delete_run_revised(&mut self, text: &str, mark: &RevisionMark) {
        // #586：共享日期边界（首错落账——两法互不覆盖）。
        if let Err(msg) = validate_revision_date(&mark.date) {
            self.record_validation_error(format!(
                "修订日期非法（delete_run_revised：author={:?} date={:?}）：{msg}",
                mark.author, mark.date
            ));
        }
        let run = docx_rs::Run::new().add_delete_text(text);
        let del = docx_rs::Delete::new()
            .add_run(run)
            .author(&mark.author)
            .date(&mark.date);
        let p = docx_rs::Paragraph::new().add_delete(del);
        let docx = std::mem::take(self.docx_mut());
        *self.docx_mut() = docx.add_paragraph(p);
        self.count_raw_emission(1, 1);
    }

    /// 发射**插入修订段（富文本多 run，#556）**：`<w:p><w:ins …>
    /// <w:r rPr>…</w:r>×N</w:ins></w:p>`——每个 [`RunSpec`] 一个
    /// run，rPr 经 [`crate::docx::spec_run_property`] 单点翻译保留
    /// （加粗/斜体/删除线/下画线/等宽/字符样式）。
    ///
    /// **单元素等价**：`insert_runs_revised(&[RunSpec::text(x)])` 与
    /// [`DocxWriter::insert_run_revised`](x) 产物逐字节一致
    /// （`Insert::new(run)` ≡ `new_with_empty().add_run(run)`——
    /// tests/docx_revision_write.rs 锁定，含 id 归一化对照）。
    ///
    /// **边界（V13 粘性拒——save 漏斗零产物）**：`link`/`anchor`/
    /// `footnote`/`field` 载荷的 RunSpec 不能经引擎修订容器合法承载
    /// （见模块文档）；带载荷调用即落粘性错误，文本面照常发射但文档
    /// 永不产出（V7 同语义——写自洽不产出半合法文档）。
    ///
    /// **行为锁**：零 run = 空容器（`<w:ins …/>` 合法最小形，Word
    /// 显示空插入标记，计数 1 段 + 0 run）；`\n` 由引擎 `add_text`
    /// 剥除（与普通 `paragraph()` 路径同源——不拆 w:br）。
    ///
    /// r4-t2 计数与输入校验：1 段 + N run（`count_specs` 复用 V12，不放宽
    /// 上限）。
    pub fn insert_runs_revised(&mut self, runs: &[RunSpec], mark: &RevisionMark) {
        self.note_unshippable_revision_payloads(runs, "insert_runs_revised");
        let mut ins = docx_rs::Insert::new_with_empty()
            .author(&mark.author)
            .date(&mark.date);
        for spec in runs {
            let mut run = docx_rs::Run::new().add_text(spec.text.as_str());
            run.run_property = spec_run_property(spec);
            ins = ins.add_run(run);
        }
        let p = docx_rs::Paragraph::new().add_insert(ins);
        let docx = std::mem::take(self.docx_mut());
        *self.docx_mut() = docx.add_paragraph(p);
        self.count_specs(1, runs);
    }

    /// 发射**删除修订段（富文本多 run，#556）**：`<w:p><w:del …>
    /// <w:r rPr><w:delText>…</w:delText></w:r>×N</w:del></w:p>`——
    /// 每个 run 用 `w:delText` 承载（OOXML 删除修订约定，`w:t` 在
    /// `w:del` 内非法）+ rPr 逐 run 保留（与
    /// [`DocxWriter::insert_runs_revised`] 同翻译单点）。
    ///
    /// 边界（V13）/行为锁（零 run 空容器、`\n` 剥除）/计数（1 段 +
    /// N run）均与 [`DocxWriter::insert_runs_revised`] 同面；单元素
    /// 与 [`DocxWriter::delete_run_revised`] 等价（测试锁定）。
    pub fn delete_runs_revised(&mut self, runs: &[RunSpec], mark: &RevisionMark) {
        self.note_unshippable_revision_payloads(runs, "delete_runs_revised");
        let mut del = docx_rs::Delete::new().author(&mark.author).date(&mark.date);
        for spec in runs {
            let mut run = docx_rs::Run::new().add_delete_text(spec.text.as_str());
            run.run_property = spec_run_property(spec);
            del = del.add_run(run);
        }
        let p = docx_rs::Paragraph::new().add_delete(del);
        let docx = std::mem::take(self.docx_mut());
        *self.docx_mut() = docx.add_paragraph(p);
        self.count_specs(1, runs);
    }

    /// V13 边界检查（私有——两富文本发射面共用）：修订容器不能合法
    /// 承载的 RunSpec 载荷登记粘性错误（首错留存，save 漏斗拒）。
    /// anchor 优先于 link 报告（V8 优先序镜像——同 Some 时 anchor 赢）。
    fn note_unshippable_revision_payloads(&mut self, runs: &[RunSpec], face: &str) {
        for (i, spec) in runs.iter().enumerate() {
            let kind = if spec.anchor.is_some() {
                "anchor"
            } else if spec.link.is_some() {
                "link"
            } else if spec.footnote.is_some() {
                "footnote"
            } else if spec.field.is_some() {
                "field"
            } else {
                continue;
            };
            self.note_validation_error(format!(
                "V13: {face} RunSpec[{i}] 带 {kind} 载荷——docx-rs 0.4.22 \
                 修订容器 child 面（InsertChild/DeleteChild）仅 Run/Delete/Comment，
                 {kind} 不能合法承载（修订容器富文本边界，save 漏斗零产物）"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// #586：修订日期共享校验边界（纯文本 ins/del 与富文本 ins/del 四
// 调用点同源把关——富文本面 [#556/#580] 合入时在同 impl 内加调用
// 即可，无需复制逻辑）。
// ---------------------------------------------------------------------------

/// #586：修订日期 W3CDTF 语法级校验（共享边界）。
///
/// **威胁模型**：`w:date` 经 docx-rs `date()` 原始字节拼入 XML 属性
/// （无第二层转义——研究仓 round47 源级证据，主 f36b40e 生产写面
/// 实证：`2026-…Z" injected="yes` 产出 `w:date="…Z" injected="yes"
/// 伪造属性）。语法白名单从源头拒绝（属性面字符集 ∈ 安全面）。
///
/// 接受（**无损保留原文**——大小写/时区形态原样落 `w:date`）：
///
/// ```text
/// YYYY-MM-DDThh:mm:ss[.f+][Z|z|±HH:MM]
/// ```
///
/// - 年 4 位数字（`0000`-`9999`——ISO 8601:2019 容忍 `0000`，
///   XSD 1.0 不允许——按模块声明的双标准取宽容面）；
/// - 月 `01`-`12` / 日 `01`-`31` / 时 `00`-`23` / 分 `00`-`59` /
///   秒 `00`-`60`（`60` = 闰秒，XSD 惯例）；
/// - 小数秒：`.` + ≥1 位数字（不封顶——纳秒面无损）；
/// - 时区：缺省（XSD 合法）| `Z`/`z` | `±HH:MM`（偏移时 `00`-`23`、
///   分 `00`-`59`）；
/// - `T` 分隔大小写均收（ISO 8601 容忍 `t`，保留原样）。
///
/// 拒绝（→ 粘性错误 + save 零产物）：
///
/// - **空串**（显式选择：W3CDTF 无空形态——「不关心时间」面用
///   [`RevisionMark::default`] 的 epoch 兜底，不用空串占位）；
/// - XML 属性面危险字符（`"` `'` `<` `>` `&`、控制字符、空白、
///   非 ASCII——注入面 + Word 解析面双拒；结构校验逐字节把关，
///   任何未预期字符必落拒绝）；
/// - 结构错位（长度不足 / 分隔位错 / 非数字位）与域越界
///   （月 `13`、时 `24`、秒 `61`、偏移 `+25:00`…）。
///
/// 边界如实登记：**语法级**校验（非日历级——`02-30`/`02-31` 等月末
/// 语义不在本层，由消费端 Word/LO 承载；office-io 依赖闭包零
/// chrono，语法白名单是无重型依赖下的安全承载）。
pub(crate) fn validate_revision_date(date: &str) -> Result<(), String> {
    let b = date.as_bytes();
    // 完整形态固定前缀长度：YYYY-MM-DDThh:mm:ss = 19 字节。
    if b.len() < 19 {
        return Err(format!(
            "长度 {} < 19（W3CDTF 完整形态 YYYY-MM-DDThh:mm:ss）",
            b.len()
        ));
    }
    // 数字位（年 4 + 月 2 + 日 2 + 时 2 + 分 2 + 秒 2 = 14 位——逐位
    // is_ascii_digit，多字节 UTF-8 的字节必非数字位落拒；错误消息用
    // 字节而非 str 切片（非字符边界切片会 panic——防御面）。
    for (label, start, width) in [
        ("年", 0usize, 4usize),
        ("月", 5, 2),
        ("日", 8, 2),
        ("时", 11, 2),
        ("分", 14, 2),
        ("秒", 17, 2),
    ] {
        for k in 0..width {
            let pos = start + k;
            if !b[pos].is_ascii_digit() {
                return Err(format!(
                    "{}域第 {} 字节非数字（字节 0x{:02X}——结构/字符白名单违规）",
                    label, pos, b[pos]
                ));
            }
        }
    }
    // 分隔位：- - T : :（T 大小写均收）。
    if b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return Err("分隔符错位（须 YYYY-MM-DDThh:mm:ss）".into());
    }
    if b[10] != b'T' && b[10] != b't' {
        return Err("日期时间分隔须为 T/t（位置 10）".into());
    }
    // 域越界（语法级——日上限 31，月末语义由消费端承载）。
    let two = |i: usize| (b[i] - b'0') as u32 * 10 + (b[i + 1] - b'0') as u32;
    let month = two(5);
    let day = two(8);
    let hour = two(11);
    let minute = two(14);
    let second = two(17);
    if !(1..=12).contains(&month) {
        return Err(format!("月域 {month:02} 超界（须 01-12）"));
    }
    if !(1..=31).contains(&day) {
        return Err(format!("日域 {day:02} 超界（须 01-31）"));
    }
    if hour > 23 {
        return Err(format!("时域 {hour:02} 超界（须 00-23）"));
    }
    if minute > 59 {
        return Err(format!("分域 {minute:02} 超界（须 00-59）"));
    }
    if second > 60 {
        return Err(format!("秒域 {second:02} 超界（须 00-60，60 = 闰秒）"));
    }
    // 尾部：小数秒和/或时区（逐字节把关——任何未预期字符拒绝）。
    let mut rest = &b[19..];
    if rest.first() == Some(&b'.') {
        let frac_len = rest[1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if frac_len == 0 {
            return Err("小数秒须至少 1 位数字（`.` 后无数字）".into());
        }
        rest = &rest[1 + frac_len..];
    }
    match rest.first() {
        None => Ok(()), // 时区缺省（XSD 合法）。
        Some(b'Z') | Some(b'z') => {
            if rest.len() != 1 {
                return Err(format!("Z 后尾随 {} 字节（须恰为 Z）", rest.len() - 1));
            }
            Ok(())
        }
        Some(b'+') | Some(b'-') => {
            if rest.len() != 6 || rest[3] != b':' {
                return Err("时区偏移须为 ±HH:MM（恰 6 字节）".into());
            }
            if !rest[1].is_ascii_digit()
                || !rest[2].is_ascii_digit()
                || !rest[4].is_ascii_digit()
                || !rest[5].is_ascii_digit()
            {
                return Err("时区偏移 HH/MM 须为数字".into());
            }
            let tz_h = (rest[1] - b'0') as u32 * 10 + (rest[2] - b'0') as u32;
            let tz_m = (rest[4] - b'0') as u32 * 10 + (rest[5] - b'0') as u32;
            if tz_h > 23 {
                return Err(format!("时区偏移时 {tz_h:02} 超界（须 00-23）"));
            }
            if tz_m > 59 {
                return Err(format!("时区偏移分 {tz_m:02} 超界（须 00-59）"));
            }
            Ok(())
        }
        Some(other) => Err(format!(
            "尾部非法字符 {:?}（仅允许小数秒/时区 Z|±HH:MM）",
            *other as char
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_construction_and_default() {
        let m = RevisionMark::new("评审人A", "2026-11-07T08:30:00Z");
        assert_eq!(m.author, "评审人A");
        assert_eq!(m.date, "2026-11-07T08:30:00Z");
        // docx-rs 同款缺省（可诊断兜底）。
        assert_eq!(
            RevisionMark::default(),
            RevisionMark::new("unnamed", "1970-01-01T00:00:00Z")
        );
    }

    // ── #586：共享日期边界（语法白名单——合法面） ──────────────

    #[test]
    fn revision_date_accepts_w3cdtf_forms() {
        // UTC（既有夹具同款）。
        assert!(validate_revision_date("2026-11-07T08:30:00Z").is_ok());
        // 时区偏移（正负两侧）。
        assert!(validate_revision_date("2026-11-07T08:30:00+08:00").is_ok());
        assert!(validate_revision_date("2026-11-07T08:30:00-05:30").is_ok());
        // 小数秒（毫秒/微秒/纳秒——不封顶无损）。
        assert!(validate_revision_date("2026-11-07T08:30:00.5Z").is_ok());
        assert!(validate_revision_date("2026-11-07T08:30:00.123456Z").is_ok());
        assert!(validate_revision_date("2026-11-07T08:30:00.123456789+08:00").is_ok());
        // 时区缺省（XSD 合法）。
        assert!(validate_revision_date("2026-11-07T08:30:00").is_ok());
        // 小写 t/z（ISO 8601 容忍——保留原样）。
        assert!(validate_revision_date("2026-11-07t08:30:00z").is_ok());
        // 域边界：秒 60（闰秒）/ 日 31 / 时 23 / 偏移 23:59。
        assert!(validate_revision_date("2026-12-31T23:59:60Z").is_ok());
        assert!(validate_revision_date("2024-01-31T00:00:00Z").is_ok());
        assert!(validate_revision_date("2026-01-01T23:00:00+23:59").is_ok());
        // 年 0000（ISO 8601:2019 容忍——模块声明双标准取宽容面）。
        assert!(validate_revision_date("0000-01-01T00:00:00Z").is_ok());
    }

    #[test]
    fn revision_date_rejects_injection_and_structural_violations() {
        // 注入面（issue 实证载荷 + 属性面危险字符族）。
        let payload = "2026-11-08T00:00:00Z\" injected=\"yes";
        assert!(
            validate_revision_date(payload).is_err(),
            "属性注入载荷必须拒"
        );
        for bad in [
            "2026-11-08T00:00:00Z<x/>",
            "2026-11-08T00:00:00Z&amp;",
            "2026-11-08T00:00:00Z'",
            "2026-11-08T00:00:00\nZ",
            "2026-11-08T00:00:00 Z",
            "2026-11-08T00:00:00\tZ",
        ] {
            assert!(
                validate_revision_date(bad).is_err(),
                "危险字符面须拒：{bad:?}"
            );
        }
        // 空（显式选择——见 RevisionMark 文档）。
        assert!(validate_revision_date("").is_err(), "空串拒绝");
        // 结构错位。
        for bad in [
            "2026-11-08",                // 日期面
            "2026-11-08 00:00:00Z",      // 空格代 T
            "2026-11-08T00:00Z",         // 缺秒
            "26-11-08T00:00:00Z",        // 年 2 位
            "2026/11/08T00:00:00Z",      // 斜杠分隔
            "2026-13-08T00:00:00Z",      // 月 13
            "2026-00-08T00:00:00Z",      // 月 00
            "2026-11-32T00:00:00Z",      // 日 32
            "2026-11-00T00:00:00Z",      // 日 00
            "2026-11-08T24:00:00Z",      // 时 24
            "2026-11-08T00:60:00Z",      // 分 60
            "2026-11-08T00:00:61Z",      // 秒 61
            "2026-11-08T00:00:00.Z",     // 小数秒无数字
            "2026-11-08T00:00:00+8:00",  // 偏移时 1 位
            "2026-11-08T00:00:00+25:00", // 偏移时 25
            "2026-11-08T00:00:00+08:60", // 偏移分 60
            "2026-11-08T00:00:00ZZ",     // 双 Z
            "2026-11-08T00:00:00Z ",     // 尾随空白
            "中文日期",                  // 非 ASCII
        ] {
            assert!(
                validate_revision_date(bad).is_err(),
                "结构/域违规须拒：{bad:?}"
            );
        }
    }

    /// 粘性首错语义：第一个违规落账，后续（含合法发射）不覆盖。
    #[test]
    fn revision_date_sticky_first_error_wins() {
        let mut w = DocxWriter::new();
        let bad = RevisionMark::new("甲", "2026-11-08T00:00:00Z\" injected=\"yes");
        w.insert_run_revised("a", &bad);
        // 合法发射不覆盖粘性错误。
        w.insert_run_revised("b", &RevisionMark::default());
        let err = w.save_to_vec().expect_err("粘性错误必须令 save 恒拒");
        let msg = err.to_string();
        assert!(
            msg.contains("insert_run_revised"),
            "错误含接口上下文：{msg}"
        );
        assert!(msg.contains("date"), "错误含字段上下文：{msg}");
        assert!(
            !msg.contains("delete_run_revised"),
            "首错（ins 面）不被后错覆盖的语义面——错误唯一起点：{msg}"
        );
    }

    /// del 面同源把关（接口上下文区分 ins/del）。
    #[test]
    fn revision_date_delete_face_rejects() {
        let mut w = DocxWriter::new();
        let bad = RevisionMark::new("甲", "2026-11-08T00:00:00<x>");
        w.delete_run_revised("a", &bad);
        let err = w.save_to_vec().expect_err("del 面非法 date 必拒");
        assert!(
            err.to_string().contains("delete_run_revised"),
            "del 面错误含接口上下文"
        );
    }
}
