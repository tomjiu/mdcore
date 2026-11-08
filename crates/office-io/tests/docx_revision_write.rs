//! #413 —— DOCX 修订标记写面验收（w:ins/w:del 输出 + 读回往返）。
//!
//! 语义基线（读写对称——读面 #17 在档语义）：
//! - `w:ins` 内文本 = **当前生效文本**：读回提取 + run 带
//!   `revision_inserted = true`（Word 显示为插入修订）；
//! - `w:delText` = **已删除内容**：读回不提取 + 逐段登记
//!   `WarningKind::RevisionDelete`（Word 显示为删除线修订）。
//!
//! 验收对照（issue #413）：
//! 1. 写入 w:ins，Word 显示为插入修订 → XML 结构断言（w:ins 包裹
//!    w:r/w:t + author/date/id 属性）+ LO 跨消费者转换探针（ignore
//!    门控——需 LO 环境，本仓 CI 无 LO，与 g1_lo_probe 同纪律）；
//! 2. 写入 w:del，Word 显示删除线 → 同上（w:delText 承载）；
//! 3. 自己的读面能读回（往返一致）→ 常开断言（本文件主体）；
//! 4. author/date/id 正确保留 → XML 属性断言（id 由 docx-rs
//!    HistoryId 自动分配——断言在场 + 唯一，见模块文档偏差 2）。
//!
//! 偏差登记：issue 验收 1/2 的「Word 打开显示」在无 Word 环境下以
//! ① OOXML 结构逐字段断言 ② 本沙箱 LibreOffice headless 转换存活
//! （探针）双证据代偿，如实登记。
//!
//! #556 富文本多 run 扩面验收（追加面）：
//! 1. 多 run 粗体/斜体样例 XML：w:ins/w:del 内逐 run rPr 保留（
//!    w:b/w:i 在场）+ 文本保序 + author/date/id 属性；
//! 2. 删除容器 w:delText 承载（零 w:t）+ rPr 保留；
//! 3. 读面读回：多 run 拼接提取 + revision_inserted 标记（含
//!    bold/italic 读回）+ 删除面零提取 + RevisionDelete 诊断；
//! 4. 单元素 RunSpec::text 与原纯文本方法 XML 等价（id 归一）；
//! 5. 零 run / 空文本 / 多行文本（\n 引擎剥除）行为锁定；
//! 6. V13 边界：link/anchor/footnote/field 载荷粘性拒（save 零产物）。

use mdc_office_io::docx::{
    read_docx_from_bytes_with_options, Block, DocxWriter, FieldSpec, RunSpec,
};
use mdc_office_io::docx_revision::RevisionMark;
use mdc_office_io::io::{ReadOptions, WarningKind};

const INS_TEXT: &str = "新增：质保期延长至 5 年";
const DEL_TEXT: &str = "原报价有效期 30 天";
const AUTHOR: &str = "评审人A";
const DATE: &str = "2026-11-07T08:30:00Z";

fn mark() -> RevisionMark {
    RevisionMark::new(AUTHOR, DATE)
}

/// 三段修订文档：普通段 + 插入修订段 + 删除修订段（混合形态——
/// redline 的最小真实切片）。
fn mixed_revision_docx() -> Vec<u8> {
    let mut w = DocxWriter::new();
    w.heading(1, "第二章 报价构成").unwrap();
    w.paragraph_text("报价以最新澄清文件为准。");
    w.insert_run_revised(INS_TEXT, &mark());
    w.delete_run_revised(DEL_TEXT, &mark());
    w.save_to_vec().expect("修订文档应可保存")
}

/// 读 zip 内 document.xml 原文（XML 结构断言面——zip 为产品依赖）。
fn document_xml(bytes: &[u8]) -> String {
    use std::io::Read as _;
    let cursor = std::io::Cursor::new(bytes);
    let mut za = zip::ZipArchive::new(cursor).expect("产物应为合法 zip");
    let mut zf = za.by_name("word/document.xml").expect("document.xml 在场");
    let mut s = String::new();
    zf.read_to_string(&mut s).unwrap();
    s
}

// ----------------------------------------------------------- 往返读回

#[test]
fn ins_roundtrip_extracted_with_flag() {
    // 验收 3（ins 面）：写出的 w:ins 被自己的读面读回——文本提取 +
    // run 级 revision_inserted 标记（读面 #17 语义对称）。
    let bytes = mixed_revision_docx();
    let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true })
        .expect("修订文档应可读回");

    let mut flagged = String::new();
    let mut ins_runs = 0usize;
    for b in &doc.blocks {
        if let Block::Paragraph(p) = b {
            for r in &p.runs {
                if r.revision_inserted {
                    ins_runs += 1;
                    flagged.push_str(&r.text);
                }
            }
        }
    }
    assert_eq!(flagged, INS_TEXT, "ins 文本完整提取且带标记");
    assert!(
        ins_runs >= 1,
        "至少一个 revision_inserted run（实测 {ins_runs}）"
    );
    // 普通段零误标。
    let normal_flagged = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| p.runs.iter())
        .filter(|r| !r.revision_inserted && r.text.contains("报价"))
        .count();
    assert!(normal_flagged >= 1, "普通段文本正常提取");
}

#[test]
fn del_roundtrip_skipped_with_warning() {
    // 验收 3（del 面）：delText 不在读回文本 + RevisionDelete 登记可见。
    let bytes = mixed_revision_docx();
    let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true })
        .expect("修订文档应可读回");

    let all_text: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.text()),
            _ => None,
        })
        .collect();
    assert!(
        all_text.iter().all(|t| !t.contains(DEL_TEXT)),
        "已删除文本零提取: {all_text:?}"
    );
    let rev_warnings = doc
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::RevisionDelete)
        .count();
    assert!(
        rev_warnings >= 1,
        "RevisionDelete warning ≥ 1（实测 {rev_warnings}）"
    );
}

#[test]
fn plain_paragraphs_untouched_by_revision_emission() {
    // 控制组：修订段发射不影响普通段/标题的读回（混合文档里普通
    // 面语义不变——门是加法，不是负担）。
    let bytes = mixed_revision_docx();
    let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true }).unwrap();
    let all_text: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.text()),
            _ => None,
        })
        .collect();
    assert!(
        all_text
            .iter()
            .any(|t| t.contains("报价以最新澄清文件为准")),
        "普通段读回不变: {all_text:?}"
    );
}

// ------------------------------------------------------- XML 结构面

#[test]
fn ins_xml_shape_author_date_id() {
    // 验收 1 + 4：w:ins 包裹 w:r/w:t + w:author/w:date/w:id 属性在场。
    let xml = document_xml(&mixed_revision_docx());
    assert!(
        xml.contains("<w:ins "),
        "w:ins 元素在场（Word 插入修订承载）: 前 400 字符 {}",
        &xml[..xml.len().min(400)]
    );
    // author/date 属性（RevisionMark 直落——转义面由 docx-rs 内聚）。
    assert!(
        xml.contains("w:author=\"评审人A\""),
        "w:author 保留: {xml:?}"
    );
    assert!(
        xml.contains("w:date=\"2026-11-07T08:30:00Z\""),
        "w:date 保留（ISO 8601 直落）"
    );
    assert!(
        xml.contains("w:id="),
        "w:id 在场（docx-rs HistoryId 自动分配）"
    );
    // ins 文本在 w:t 承载（当前生效文本——非 delText）。
    let ins_pos = xml.find("<w:ins ").expect("ins 在场");
    let ins_end = xmlins_close(&xml, ins_pos);
    let ins_frag = &xml[ins_pos..ins_end];
    assert!(
        ins_frag.contains("<w:t") && ins_frag.contains(INS_TEXT),
        "ins 内为 w:t 承载当前生效文本: {ins_frag}"
    );
}

#[test]
fn del_xml_shape_deltext_not_wt() {
    // 验收 2 + 4：w:del 包裹 w:r/**w:delText**（非 w:t——OOXML 删除
    // 修订承载约定），author/date/id 同 ins。
    let xml = document_xml(&mixed_revision_docx());
    assert!(
        xml.contains("<w:del "),
        "w:del 元素在场（Word 删除线修订承载）"
    );
    let del_pos = xml.find("<w:del ").expect("del 在场");
    let del_end = xmlins_close(&xml, del_pos);
    let del_frag = &xml[del_pos..del_end];
    assert!(
        del_frag.contains("<w:delText"),
        "del 内为 w:delText 承载（w:t 在 w:del 内是非法形态）: {del_frag}"
    );
    assert!(
        del_frag.contains(DEL_TEXT),
        "删除文本在 delText 内: {del_frag}"
    );
    assert!(!del_frag.contains("<w:t>"), "del 容器内零 w:t: {del_frag}");
    assert!(
        del_frag.contains("w:author=\"评审人A\""),
        "del 的 author 保留"
    );
}

#[test]
fn ids_unique_across_marks() {
    // 验收 4（id 面）：多个修订标记的 w:id 互异（HistoryId 全局
    // 计数器——本断言锁「不撞号」，#415 的 MD 块 ID 撞号是另一面）。
    let mut w = DocxWriter::new();
    for i in 0..4 {
        w.insert_run_revised(&format!("新增条目 {i}"), &mark());
        w.delete_run_revised(&format!("删除条目 {i}"), &mark());
    }
    let xml = document_xml(&w.save_to_vec().unwrap());
    let ids: Vec<&str> = xml
        .split("<w:ins ")
        .skip(1)
        .filter_map(|frag| {
            let end = frag.find('>').unwrap_or(frag.len());
            frag[..end].split("w:id=\"").nth(1).map(|rest| {
                let id_end = rest.find('"').unwrap_or(rest.len());
                &rest[..id_end]
            })
        })
        .collect();
    assert_eq!(ids.len(), 4, "4 个 ins 标记全部带 id: {ids:?}");
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "id 互异: {ids:?}");
}

/// 从 `<w:ins `/`<w:del ` 起位找对应闭合标签（粗切——结构断言足够；
/// 不引 XML 解析依赖）。
fn xmlins_close(xml: &str, from: usize) -> usize {
    let tag = if xml[from..].starts_with("<w:ins ") {
        "</w:ins>"
    } else {
        "</w:del>"
    };
    xml[from..]
        .find(tag)
        .map(|p| from + p + tag.len())
        .unwrap_or(xml.len())
}

/// `w:id="…"` 归一为 `w:id="#"`（HistoryId 全局计数器跨文档递增
/// ——自动分配面非语义差异，等价对照前归一）。
fn normalize_ids(frag: &str) -> String {
    let mut parts = frag.split("w:id=\"");
    let mut out = parts.next().unwrap_or("").to_owned();
    for part in parts {
        out.push_str("w:id=\"#");
        if let Some(q) = part.find('"') {
            out.push_str(&part[q..]);
        }
    }
    out
}

// ------------------------------------------------- #556 富文本多 run 面

const MR_BOLD: &str = "加粗修订文本";
const MR_ITALIC: &str = "斜体补充说明";
const MR_PLAIN: &str = "普通收尾段";
const MR_DEL_BOLD: &str = "被删除的加粗句";
const MR_DEL_ITALIC: &str = "被删除的斜体句";

/// 加粗/斜体二位规格（其余域缺省——富文本最小形态）。
fn styled(text: &str, bold: bool, italic: bool) -> RunSpec {
    RunSpec {
        bold,
        italic,
        ..RunSpec::text(text)
    }
}

/// 首 `<w:{tag} ` 片段（并闭含）——多 run 断言的统一取片面。
fn first_frag<'a>(xml: &'a str, tag: &str) -> &'a str {
    let open = format!("<w:{tag} ");
    let pos = xml.find(&open).unwrap_or_else(|| panic!("w:{tag} 应在场"));
    &xml[pos..xmlins_close(xml, pos)]
}

#[test]
fn ins_multirun_xml_rpr_and_order() {
    // 验收 1（#556）：多 run 粗体/斜体样例的 XML——w:ins 中每个 run
    // 的 rPr 保留（w:b/w:i 在场）、三 run 保序、author/date/id 属性。
    let mut w = DocxWriter::new();
    w.insert_runs_revised(
        &[
            styled(MR_BOLD, true, false),
            styled(MR_ITALIC, false, true),
            RunSpec::text(MR_PLAIN),
        ],
        &mark(),
    );
    let xml = document_xml(&w.save_to_vec().unwrap());
    let frag = first_frag(&xml, "ins");
    assert!(frag.contains("<w:b />"), "加粗 run 的 w:b 保留: {frag}");
    assert!(frag.contains("<w:i />"), "斜体 run 的 w:i 保留: {frag}");
    // 保序（run 序 = spec 序——读回拼接依赖此序）。
    let (pb, pi, pp) = (
        frag.find(MR_BOLD).expect("加粗文本在场"),
        frag.find(MR_ITALIC).expect("斜体文本在场"),
        frag.find(MR_PLAIN).expect("普通文本在场"),
    );
    assert!(pb < pi && pi < pp, "三 run 保序: {frag}");
    // 每 spec 一 run（`<w:r>` 无属性形——引擎 open_run 无属性）。
    assert_eq!(frag.matches("<w:r>").count(), 3, "每 spec 一 run: {frag}");
    assert!(
        frag.contains("w:author=\"评审人A\"") && frag.contains(&format!("w:date=\"{DATE}\"")),
        "author/date 属性保留: {frag}"
    );
    assert!(frag.contains("w:id="), "w:id 在场（HistoryId 自动分配）");
}

#[test]
fn del_multirun_deltext_rpr_no_wt() {
    // 验收 2（#556）：删除容器多 run——每 run w:delText 承载（零
    // w:t，含带属性形）+ rPr 保留 + author/date/id 同 ins。
    let mut w = DocxWriter::new();
    w.delete_runs_revised(
        &[
            styled(MR_DEL_BOLD, true, false),
            styled(MR_DEL_ITALIC, false, true),
        ],
        &mark(),
    );
    let xml = document_xml(&w.save_to_vec().unwrap());
    let frag = first_frag(&xml, "del");
    assert_eq!(frag.matches("<w:r>").count(), 2, "每 spec 一 run: {frag}");
    assert!(
        frag.matches("<w:delText").count() >= 2,
        "每 run 一 delText 承载: {frag}"
    );
    assert!(
        !frag.contains("<w:t ") && !frag.contains("<w:t>"),
        "del 容器内零 w:t（含带属性形）: {frag}"
    );
    assert!(
        frag.contains("<w:b />") && frag.contains("<w:i />"),
        "rPr 保留: {frag}"
    );
    assert!(
        frag.contains(MR_DEL_BOLD) && frag.contains(MR_DEL_ITALIC),
        "删除文本在场: {frag}"
    );
    assert!(
        frag.contains("w:author=\"评审人A\"") && frag.contains("w:id="),
        "author/id 保留: {frag}"
    );
}

#[test]
fn multirun_roundtrip_read_back() {
    // 验收 3（#556）：多 run 读回——插入面逐 run 提取 + 拼接序 +
    // bold/italic 读回（rPr 保留的读侧证据）+ 删除面零提取 +
    // RevisionDelete 诊断。
    let mut w = DocxWriter::new();
    w.insert_runs_revised(
        &[
            styled(MR_BOLD, true, false),
            styled(MR_ITALIC, false, true),
            RunSpec::text(MR_PLAIN),
        ],
        &mark(),
    );
    w.delete_runs_revised(
        &[
            styled(MR_DEL_BOLD, true, false),
            styled(MR_DEL_ITALIC, false, true),
        ],
        &mark(),
    );
    let doc =
        read_docx_from_bytes_with_options(&w.save_to_vec().unwrap(), &ReadOptions { compat: true })
            .expect("多 run 修订文档应可读回");

    // 插入面：三 run 全带标记，拼接序 = 发射序，格式位读回。
    let flagged: Vec<&mdc_office_io::docx::RunData> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| p.runs.iter())
        .filter(|r| r.revision_inserted)
        .collect();
    assert_eq!(flagged.len(), 3, "每 spec 一读回 run: {flagged:?}");
    let joined: String = flagged.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        joined,
        format!("{MR_BOLD}{MR_ITALIC}{MR_PLAIN}"),
        "拼接序 = 发射序"
    );
    assert!(flagged[0].bold, "加粗位读回（rPr 保留的读侧证据）");
    assert!(flagged[1].italic, "斜体位读回");
    assert!(!flagged[2].bold && !flagged[2].italic, "普通 run 零误标");

    // 删除面：两文本零提取 + RevisionDelete ≥ 1。
    let all_text: Vec<String> = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.text()),
            _ => None,
        })
        .collect();
    assert!(
        all_text
            .iter()
            .all(|t| !t.contains(MR_DEL_BOLD) && !t.contains(MR_DEL_ITALIC)),
        "多 run 删除文本零提取: {all_text:?}"
    );
    let rev_warnings = doc
        .warnings
        .iter()
        .filter(|x| x.kind == WarningKind::RevisionDelete)
        .count();
    assert!(
        rev_warnings >= 1,
        "RevisionDelete ≥ 1（实测 {rev_warnings}）"
    );
}

#[test]
fn single_element_equivalence_with_plain_faces() {
    // 验收 4（#556）：单元素 RunSpec::text 与原纯文本方法行为等价
    // ——产物 XML 逐字节一致（w:id 归一后：HistoryId 全局计数器跨
    // 文档递增，属自动分配面非语义差异）。
    let mut a = DocxWriter::new();
    a.insert_run_revised("等价对照文本", &mark());
    let xml_a = document_xml(&a.save_to_vec().unwrap());
    let mut b = DocxWriter::new();
    b.insert_runs_revised(&[RunSpec::text("等价对照文本")], &mark());
    let xml_b = document_xml(&b.save_to_vec().unwrap());
    assert_eq!(
        normalize_ids(first_frag(&xml_a, "ins")),
        normalize_ids(first_frag(&xml_b, "ins")),
        "ins 单元素等价（id 归一）"
    );

    let mut c = DocxWriter::new();
    c.delete_run_revised("等价对照文本", &mark());
    let xml_c = document_xml(&c.save_to_vec().unwrap());
    let mut d = DocxWriter::new();
    d.delete_runs_revised(&[RunSpec::text("等价对照文本")], &mark());
    let xml_d = document_xml(&d.save_to_vec().unwrap());
    assert_eq!(
        normalize_ids(first_frag(&xml_c, "del")),
        normalize_ids(first_frag(&xml_d, "del")),
        "del 单元素等价（id 归一）"
    );
}

#[test]
fn zero_runs_empty_container_behavior() {
    // 验收 5a（#556）：零 run = 空容器（合法最小形）——文档可保存，
    // w:ins/w:del 元素在场且零 run，读回零标记零提取。
    let mut w = DocxWriter::new();
    w.insert_runs_revised(&[], &mark());
    w.delete_runs_revised(&[], &mark());
    let bytes = w.save_to_vec().expect("零 run 空容器应可保存");
    let xml = document_xml(&bytes);
    let ins = first_frag(&xml, "ins");
    let del = first_frag(&xml, "del");
    assert!(!ins.contains("<w:r>"), "ins 空容器零 run: {ins}");
    assert!(!del.contains("<w:r>"), "del 空容器零 run: {del}");
    assert!(
        ins.contains("w:author=\"评审人A\""),
        "空容器仍带元数据: {ins}"
    );

    let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true }).unwrap();
    let flagged = doc
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p),
            _ => None,
        })
        .flat_map(|p| p.runs.iter())
        .filter(|r| r.revision_inserted)
        .count();
    assert_eq!(flagged, 0, "空容器零标记 run");
}

#[test]
fn empty_and_multiline_text_engine_behavior() {
    // 验收 5b/5c（#556）：空文本 run 照常发射（空 w:t 合法形）；
    // 多行文本 `\n` 由引擎 add_text 剥除（与普通 paragraph() 路径
    // 同源——不拆 w:br，行为锁定非缺陷遮蔽）。
    let mut w = DocxWriter::new();
    w.insert_runs_revised(&[RunSpec::text("")], &mark());
    w.insert_runs_revised(&[RunSpec::text("第一行\n第二行")], &mark());
    let bytes = w.save_to_vec().expect("空文本/多行文本应可保存");
    let xml = document_xml(&bytes);
    assert!(
        !xml.contains("第一行\n第二行") && xml.contains("第一行第二行"),
        "\\n 剥除（引擎 replace 源行为，与 paragraph() 同源）: {}",
        &xml[..xml.len().min(600)]
    );
    // 零 w:br（修订容器不拆行——多行承载需调方先行拆 run）。
    assert!(!xml.contains("<w:br"), "修订发射不产生 w:br");

    // 空文本与多行均与原纯文本面等价（同一引擎路径锁定）。
    let mut a = DocxWriter::new();
    a.insert_run_revised("第一行\n第二行", &mark());
    let xml_a = document_xml(&a.save_to_vec().unwrap());
    let mut b = DocxWriter::new();
    b.insert_runs_revised(&[RunSpec::text("第一行\n第二行")], &mark());
    let xml_b = document_xml(&b.save_to_vec().unwrap());
    assert_eq!(
        normalize_ids(first_frag(&xml_a, "ins")),
        normalize_ids(first_frag(&xml_b, "ins")),
        "多行文本与纯文本面等价（引擎同源锁定）"
    );
}

#[test]
fn v13_payload_kinds_sticky_rejected() {
    // 验收 6（#556）：修订容器不能合法承载的四种载荷——V13 粘性拒
    // （save 漏斗零产物，错误信息带载荷种类）；控制组（纯格式 run）
    // 同面保存成功。ins/del 两发射面同拒。
    let cases: Vec<(&str, RunSpec)> = vec![
        (
            "link",
            RunSpec {
                link: Some("https://example.com/".into()),
                ..RunSpec::text("链接载荷")
            },
        ),
        (
            "anchor",
            RunSpec {
                anchor: Some("bm-556".into()),
                ..RunSpec::text("锚点载荷")
            },
        ),
        (
            "footnote",
            RunSpec {
                footnote: Some("脚注内容".into()),
                ..RunSpec::text("脚注载荷")
            },
        ),
        (
            "field",
            RunSpec {
                field: Some(FieldSpec::Page),
                ..RunSpec::text("页码域载荷")
            },
        ),
    ];
    for (kind, spec) in cases {
        for face in ["ins", "del"] {
            let mut w = DocxWriter::new();
            if face == "ins" {
                w.insert_runs_revised(&[spec.clone()], &mark());
            } else {
                w.delete_runs_revised(&[spec.clone()], &mark());
            }
            let err = w
                .save_to_vec()
                .expect_err(&format!("{face} 带 {kind} 载荷应被 V13 拒"));
            let msg = err.to_string();
            assert!(
                msg.contains("V13") && msg.contains(kind),
                "{face} 面 {kind} 载荷错误信息应带 V13 与种类: {msg}"
            );
        }
    }

    // 控制组：纯格式 run（含加粗/斜体）同面保存成功——边界只拒
    // 容器不能承载的载荷，不误伤富文本本体。
    let mut w = DocxWriter::new();
    w.insert_runs_revised(&[styled("纯格式控制组", true, true)], &mark());
    w.delete_runs_revised(&[styled("删除控制组", false, true)], &mark());
    assert!(w.save_to_vec().is_ok(), "纯格式 run 不受 V13 误伤");
}

// ------------------------------------------------- LO 跨消费者探针

#[test]
#[ignore = "探针：需 LO 环境显式跑（cargo test -- --ignored，与 g1_lo_probe 同纪律）"]
fn lo_headless_conversion_smoke() {
    // 跨消费者证据：LibreOffice headless 能解析修订文档并转换 PDF
    // （弱于 Word 实开，强于零外部验证——如实定位）。产物落
    // /tmp/rev413/，断言 PDF 非空。
    // #556 扩面：单 run 混合文档 + 多 run 富文本修订文档双探针。
    let bytes = mixed_revision_docx();
    std::fs::create_dir_all("/tmp/rev413").unwrap();
    let docx_path = "/tmp/rev413/revision.docx";
    std::fs::write(docx_path, &bytes).unwrap();
    let out = std::process::Command::new("soffice")
        .args([
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            "/tmp/rev413",
            docx_path,
        ])
        .output()
        .expect("soffice 应可执行");
    assert!(
        out.status.success(),
        "LO 转换应成功: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pdf = std::fs::read("/tmp/rev413/revision.pdf").expect("PDF 产物应在场");
    assert!(pdf.len() > 500, "PDF 非空（{} 字节）", pdf.len());
    println!("LO 探针通过：revision.pdf {} 字节", pdf.len());

    // #556：多 run 富文本修订（w:ins/w:del 各含多 run rPr）——LO
    // 跨消费者存活（富文本修订容器的真实解析面证据）。
    let mut w = DocxWriter::new();
    w.insert_runs_revised(
        &[
            styled(MR_BOLD, true, false),
            styled(MR_ITALIC, false, true),
            RunSpec::text(MR_PLAIN),
        ],
        &mark(),
    );
    w.delete_runs_revised(
        &[
            styled(MR_DEL_BOLD, true, false),
            styled(MR_DEL_ITALIC, false, true),
        ],
        &mark(),
    );
    let mr_path = "/tmp/rev413/revision_multirun.docx";
    std::fs::write(mr_path, &w.save_to_vec().unwrap()).unwrap();
    let out2 = std::process::Command::new("soffice")
        .args([
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            "/tmp/rev413",
            mr_path,
        ])
        .output()
        .expect("soffice 应可执行");
    assert!(
        out2.status.success(),
        "多 run 富文本修订 LO 转换应成功: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    let pdf2 = std::fs::read("/tmp/rev413/revision_multirun.pdf").expect("PDF 产物应在场");
    assert!(pdf2.len() > 500, "多 run PDF 非空（{} 字节）", pdf2.len());
    println!(
        "LO 探针通过（#556 多 run）：revision_multirun.pdf {} 字节",
        pdf2.len()
    );
}

// ------------------------------------------------- #586 日期共享边界

/// #586 验收 1：恶意 date（属性注入载荷）经生产写面 → save_to_vec 与
/// save_to_path 双拒且零产物（解包面此前实证：修复前 document.xml 含
/// `injected="yes"` 伪造属性——先红后绿在案）。
#[test]
fn malicious_date_rejected_with_no_artifact() {
    let payload = "2026-11-08T00:00:00Z\" injected=\"yes";
    let bad = RevisionMark::new(AUTHOR, payload);

    // ins + del 双面（共享边界四调用点的纯文本两翼）。
    let mut w = DocxWriter::new();
    w.insert_run_revised(INS_TEXT, &bad);
    w.delete_run_revised(DEL_TEXT, &bad);
    let err = w.save_to_vec().expect_err("注入载荷必须拒绝");
    let msg = err.to_string();
    assert!(
        msg.contains("insert_run_revised"),
        "错误含接口上下文：{msg}"
    );
    assert!(msg.contains("date"), "错误含字段上下文：{msg}");
    // 违规原值可诊断（{:?} 转义形态在场——含注入载荷本体）。
    assert!(msg.contains("injected"), "错误含违规原值（可诊断）：{msg}");

    // save_to_path 同漏斗 + 无目标产物（零产物语义）。
    let dir = std::env::temp_dir().join(format!("mdc-586-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("out.docx");
    let mut w2 = DocxWriter::new();
    w2.insert_run_revised(INS_TEXT, &bad);
    assert!(
        w2.save_to_path(target.to_str().unwrap()).is_err(),
        "save_to_path 同漏斗拒绝"
    );
    assert!(!target.exists(), "无目标产物（半合法文件零产出）");
    // 临时区无残留（写器内部不落盘——save_to_path 先 vec 后写）。
    let residue: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(residue.is_empty(), "目录零残留：{residue:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #586 验收 2：有效形态（UTC / 时区偏移 / 小数秒 / 缺省时区）无损
/// 保留——`w:date` 属性逐字节等于原文（写面不改写合法输入）+ 读回
/// 往返不回归。
#[test]
fn valid_date_forms_preserved_byte_exact() {
    for date in [
        "2026-11-07T08:30:00Z",
        "2026-11-07T08:30:00+08:00",
        "2026-11-07T08:30:00-05:30",
        "2026-11-07T08:30:00.123456789Z",
        "2026-11-07T08:30:00",
    ] {
        let mut w = DocxWriter::new();
        w.insert_run_revised(INS_TEXT, &RevisionMark::new(AUTHOR, date));
        w.delete_run_revised(DEL_TEXT, &RevisionMark::new(AUTHOR, date));
        let bytes = w.save_to_vec().expect("合法形态必须可保存");
        let xml = document_xml(&bytes);
        // ins + del 两处 w:date 逐字节无损（原文属性值精确匹配）。
        let expect = format!("w:date=\"{date}\"");
        let hits = xml.matches(&expect).count();
        assert_eq!(hits, 2, "ins+del 双处 w:date 逐字节等于原文（{date}）");
        // 无注入残留面（修复前形态不回归的负证据）。
        assert!(!xml.contains("injected"), "无伪造属性");

        // 读回往返（验收 3 面：本 crate 读面消费合法产物）。
        let doc = read_docx_from_bytes_with_options(&bytes, &ReadOptions { compat: true })
            .expect("合法修订文档应可读回");
        let mut flagged = String::new();
        for b in &doc.blocks {
            if let Block::Paragraph(p) = b {
                for r in &p.runs {
                    if r.revision_inserted {
                        flagged.push_str(&r.text);
                    }
                }
            }
        }
        assert_eq!(flagged, INS_TEXT, "ins 文本读回不回归（{date}）");
    }
}

/// #586 验收 3：空串显式拒绝（「不关心时间」用 default epoch 兜底，
/// 不用空串占位——兼容选择见 RevisionMark 文档）+ 粘性不被合法发射
/// 覆盖（首错语义）。
#[test]
fn empty_date_rejected_and_sticky_error_survives_valid_emission() {
    let mut w = DocxWriter::new();
    w.insert_run_revised(INS_TEXT, &RevisionMark::new(AUTHOR, ""));
    let err = w.save_to_vec().expect_err("空串拒绝（W3CDTF 无空形态）");
    assert!(err.to_string().contains("长度 0"), "空串诊断可读：{err}");

    // 粘性：违规后的合法发射不覆盖（save 恒拒直至重建写器）。
    let mut w2 = DocxWriter::new();
    w2.delete_run_revised(DEL_TEXT, &RevisionMark::new(AUTHOR, "2026-13-01T00:00:00Z"));
    w2.insert_run_revised(INS_TEXT, &mark()); // 合法
    let err2 = w2.save_to_vec().expect_err("粘性错误持续（首错不被覆盖）");
    assert!(
        err2.to_string().contains("delete_run_revised"),
        "首错接口上下文保持：{err2}"
    );

    // 对照：全新写器 + 合法 mark 正常产出（重建 = 唯一解，非全局禁写）。
    mixed_revision_docx();
}
