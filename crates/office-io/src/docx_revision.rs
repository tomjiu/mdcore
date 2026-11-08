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
//!    ST_DateTime），字符串承载无损。
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
/// `author`/`date` 直落 OOXML 属性（XML 转义由 docx-rs 构造器内聚）；
/// `w:id` 由 docx-rs HistoryId 自动分配（见模块文档偏差 2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionMark {
    /// 修订作者（`w:author`——Word 审阅窗格显示名）。
    pub author: String,
    /// 修订时间（`w:date`——W3CDTF / ISO 8601 字符串，如
    /// `2026-11-07T08:30:00Z`）。
    pub date: String,
}

impl RevisionMark {
    /// 构造（author/date 直入——空串合法但 Word 显示面为空，
    /// 调用方自证；缺省面见 [`RevisionMark::default`]）。
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
    /// r4-t2 计数：1 段 + 1 run（与 [`DocxWriter::page_break`] 同为
    /// 非 RunSpec 发射面——按发射面诚实计数，走 save 漏斗上限）。
    pub fn insert_run_revised(&mut self, text: &str, mark: &RevisionMark) {
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
    /// r4-t2 计数：1 段 + 1 run（同 [`DocxWriter::insert_run_revised`]）。
    pub fn delete_run_revised(&mut self, text: &str, mark: &RevisionMark) {
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
}
