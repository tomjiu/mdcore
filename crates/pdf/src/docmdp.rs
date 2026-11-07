//! #411 —— DocMDP（文档修改检测策略）检测 + 写前守卫。
//!
//! PDF 有两级签名保护：普通数字签名（字段级），与 **DocMDP**
//! （Document Modification Detection Policy——认证签名的修改策略，
//! ISO 32000-2 §12.8.2.2）。DocMDP 在 catalog `/Perms /DocMDP` 声明，
//! 携带三级策略：
//!
//! | P 值 | 允许的修改 | 拒绝的修改 |
//! |---|---|---|
//! | 1 | 无 | 一切（连标注都不许） |
//! | 2 | 表单填写 + 签署 | 其余全部（含标注增删改） |
//! | 3 | 表单填写 + 签署 + 标注增删改 | 页内容重写等其余修改 |
//!
//! 现状问题（#411 现象面）：writer_lopdf 增量写标注时对 DocMDP **无
//! 感知**——P=1/P=2 档上写标注 = 违反文档修改策略（阅读器验签即红），
//! 此前静默产出违约档。本模块把「检测」与「策略判定」沉淀为共享面，
//! 三个写面接线（[`crate::writer_lopdf`] 标注 / [`crate::forms`] 表单
//! / [`crate::stamp`] 盖章）。
//!
//! ## fail-closed 纪律（与 #408 签名门同哲学）
//!
//! - `/Perms /DocMDP` **在场**但变换参数不可解析（`/Reference` 缺失/
//!   空、`/TransformParams` 缺失、`/P` 缺失/非整数/超 1-3 范围）→
//!   按 **P=1**（最严）处理：误拒不产出，误放静默违约不可逆；
//! - 多个 SigRef 携带不同 `/P` → 取**最严**（P 最小值）——签名覆盖
//!   变换参数的并集，许可面取交集；
//! - `/Perms /DocMDP` **缺席** → `None`，一切写操作放行（存量无策略
//!   档语义不变——门是增量防线，不是新负担）。
//!
//! ## 与 #408 签名门（[`crate::stamp`]）的分工
//!
//! - #408 [`stamp`] 面的 `doc_declares_signature` 三路判定管「签名
//!   在场」（含 DocMDP 在场这一路）——覆盖**普通签名档**（只有
//!   `/FT /Sig` 域或 SigFlags，无 DocMDP）；
//! - 本模块管「DocMDP 策略级」——错误归因到 P 级，且覆盖标注/表单
//!   两个增量写面（#408 只盖了 stamp 面）。
//!
//! [`stamp`]: crate::stamp

use lopdf::{Document, Object};

// ---------------------------------------------------------------------------
// 策略级模型
// ---------------------------------------------------------------------------

/// DocMDP 修改策略级（ISO 32000-2 §12.8.2.2，Table 254）。
///
/// 判别值与 PDF `/P` 值一致（1/2/3），`as_u8` 直接可比——
/// 「最严」= 数值最小。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocMdpLevel {
    /// P=1：禁止任何修改（连标注都不许）。
    P1 = 1,
    /// P=2：表单填写 + 签署允许；标注增删改拒绝。
    P2 = 2,
    /// P=3：表单填写 + 签署 + 标注增删改允许；页内容重写等其余拒绝。
    P3 = 3,
}

impl DocMdpLevel {
    /// `/P` 数值面（与 PDF 字面一致）。
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// 两级取严（多 SigRef 并存时许可面取交集）。
    fn most_restrictive(self, other: Self) -> Self {
        if self.as_u8() <= other.as_u8() {
            self
        } else {
            other
        }
    }
}

impl std::fmt::Display for DocMdpLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P={}", self.as_u8())
    }
}

/// 写操作类别（写前门的判定入参——每张写脸声明自己的类别，
/// 策略表在本模块单点维护）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOp {
    /// 页内容重写（盖章进内容流 / 扁平化 widget 画入页面 / 页面重组
    /// 等）——**任何 P 级都不放**：P=3 的最大放行面也只到标注增删改。
    ContentRewrite,
    /// 标注增删改（`/Annots` 增量修订）——P=3 起放行。
    Annotation,
    /// 表单填写（域 `/V` + 外观流增量修订）——P=2 起放行。
    FormFill,
    /// 签署（新增签名域）——P=2 起放行（当前仓无签署写面，
    /// 预留策略锚——接写脸时直接复用本门）。
    Signature,
}

impl std::fmt::Display for WriteOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            WriteOp::ContentRewrite => "页内容重写",
            WriteOp::Annotation => "标注增删改",
            WriteOp::FormFill => "表单填写",
            WriteOp::Signature => "签署",
        };
        f.write_str(name)
    }
}

// ---------------------------------------------------------------------------
// 拒写错误（稳定策略码 + 人读文案双面）
// ---------------------------------------------------------------------------

/// 协议面冻结锚：P=1 拒一切写（[`crate::command::CommandError::SecurityDeny`]
/// 的 `reason` 稳定码——与 `crate::security` 的 REASON_* 常量同纪律，
/// 本模块策略码随检测面内聚在此）。
pub const REASON_DOCMDP_P1_NO_CHANGES: &str = "docmdp-p1-no-changes";
/// 协议面冻结锚：P=2 拒标注增删改。
pub const REASON_DOCMDP_P2_ANNOTATION: &str = "docmdp-p2-annotation-denied";
/// 协议面冻结锚：任何 P 级都拒页内容重写。
pub const REASON_DOCMDP_CONTENT_REWRITE: &str = "docmdp-content-rewrite";
/// 防御性兜底码（策略表当前不产此组合——总函数的完备性要求）。
pub const REASON_DOCMDP_POLICY: &str = "docmdp-policy";

/// DocMDP 拒写（typed：策略级 + 操作类别双字段，错误面可程序化消费）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocMdpViolation {
    level: DocMdpLevel,
    op: WriteOp,
}

impl DocMdpViolation {
    /// 命中的策略级。
    pub fn level(&self) -> DocMdpLevel {
        self.level
    }

    /// 被拒的操作类别。
    pub fn op(&self) -> WriteOp {
        self.op
    }

    /// 协议面稳定策略码（[`crate::command::CommandError::SecurityDeny`]
    /// 的 `reason` 入参——冻结锚，宿主可按码分流 UI 提示）。
    pub fn reason_code(&self) -> &'static str {
        match (self.level, self.op) {
            (DocMdpLevel::P1, _) => REASON_DOCMDP_P1_NO_CHANGES,
            (_, WriteOp::ContentRewrite) => REASON_DOCMDP_CONTENT_REWRITE,
            (DocMdpLevel::P2, WriteOp::Annotation) => REASON_DOCMDP_P2_ANNOTATION,
            // 不可达组合（ensure_write_allowed 的策略表不产）：P=2 表单
            // /签署与 P=3 全放行。总函数防御，不 panic（typed 纪律）。
            _ => REASON_DOCMDP_POLICY,
        }
    }
}

impl std::fmt::Display for DocMdpViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.level, self.op) {
            (DocMdpLevel::P1, _) => write!(
                f,
                "文档带 DocMDP {} 修改策略：禁止任何修改（{} 被拒）",
                self.level, self.op
            ),
            (_, WriteOp::ContentRewrite) => write!(
                f,
                "文档带 DocMDP {} 修改策略：{} 被拒（P=3 的最大放行面也只到标注增删改）",
                self.level, self.op
            ),
            (DocMdpLevel::P2, WriteOp::Annotation) => write!(
                f,
                "文档带 DocMDP {} 修改策略：只许表单填写与签署，{} 被拒",
                self.level, self.op
            ),
            // 不可达（同 reason_code 注）——防御性总分支。
            _ => write!(f, "文档带 DocMDP {} 修改策略：{} 被拒", self.level, self.op),
        }
    }
}

impl std::error::Error for DocMdpViolation {}

// ---------------------------------------------------------------------------
// 检测
// ---------------------------------------------------------------------------

/// 把对象解析成字典（直接字典 / 间接引用两形态——`/Perms`、
/// `/DocMDP`、SigRef、`/TransformParams` 四层都常见两形态）。
fn as_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Dictionary(d)) => Some(d),
            _ => None,
        },
        _ => None,
    }
}

/// 检测 DocMDP 策略级：catalog `/Perms /DocMDP` → `/Reference` SigRef
/// → `/TransformParams /P`，多 SigRef 取最严。
///
/// 返回 `None` = 无 DocMDP（一切放行）。**DocMDP 在场但级不可解析
/// → `Some(P1)`**（fail-closed，见模块文档）。
///
/// 检测永不失败（畸形态归一为最严级）——错误面在
/// [`ensure_write_allowed`]，不在本函数。
pub fn detect_doc_mdp(doc: &Document) -> Option<DocMdpLevel> {
    let catalog = doc.catalog().ok()?;
    let perms = as_dict(doc, catalog.get(b"Perms").ok()?)?;
    let sig = as_dict(doc, perms.get(b"DocMDP").ok()?)?;
    // /Reference 缺失 / 空 / 非数组 = DocMDP 声明在但变换参数不可达
    // → fail-closed P1。
    let reference = match sig.get(b"Reference") {
        Ok(Object::Array(items)) => items,
        _ => return Some(DocMdpLevel::P1),
    };
    let mut level: Option<DocMdpLevel> = None;
    for item in reference {
        let Some(sigref) = as_dict(doc, item) else {
            continue;
        };
        let Ok(tp_obj) = sigref.get(b"TransformParams") else {
            continue;
        };
        let Some(tp) = as_dict(doc, tp_obj) else {
            continue;
        };
        let p = match tp.get(b"P") {
            Ok(Object::Integer(1)) => DocMdpLevel::P1,
            Ok(Object::Integer(2)) => DocMdpLevel::P2,
            Ok(Object::Integer(3)) => DocMdpLevel::P3,
            // /P 缺失 / 非整数 / 超范围 = 畸形 → fail-closed P1。
            _ => DocMdpLevel::P1,
        };
        level = Some(match level {
            None => p,
            Some(prev) => prev.most_restrictive(p),
        });
    }
    // 一个可解析的 /P 都没有 → 同上 fail-closed P1。
    Some(level.unwrap_or(DocMdpLevel::P1))
}

// ---------------------------------------------------------------------------
// 写前门（策略表单点）
// ---------------------------------------------------------------------------

/// 写前守卫：按 DocMDP 策略级判定写操作许可（策略表见模块文档）。
///
/// 三张写脸的接线位（#411）：
/// - [`crate::writer_lopdf`]：`write_annotation` / `remove_annotation`
///   入口（`WriteOp::Annotation`）——开面不拒，拒第一个写手势（只读
///   会话在受限档上合法）；
/// - [`crate::forms`]：`save`（`WriteOp::FormFill`——零脏早退之后，
///   只拦真实写回）与 `flatten`（`WriteOp::ContentRewrite`——扁平化
///   把 widget 画进页内容并删除域，超出「表单填写」的放行面）；
/// - [`crate::stamp`]：`WriteOp::ContentRewrite`（盖章改页内容流），
///   `allow_break_signature` 显式放行时**不调本门**（#408 契约：该
///   开关即「违反签名纪律」的显式知情确认）。
pub fn ensure_write_allowed(doc: &Document, op: WriteOp) -> Result<(), DocMdpViolation> {
    let Some(level) = detect_doc_mdp(doc) else {
        return Ok(());
    };
    let denied = match (level, op) {
        // 内容重写：任何 P 级都不放——P=3 的最大放行面也只到标注。
        (_, WriteOp::ContentRewrite) => true,
        // P=1：禁止任何修改。
        (DocMdpLevel::P1, _) => true,
        // P=2：表单填写 + 签署放行，标注拒。
        (DocMdpLevel::P2, WriteOp::Annotation) => true,
        (DocMdpLevel::P2, _) => false,
        // P=3：标注 + 表单 + 签署全放（内容重写已由首臂兜住）。
        (DocMdpLevel::P3, _) => false,
    };
    if denied {
        Err(DocMdpViolation { level, op })
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 单测（检测矩阵 + 策略矩阵 + 冻结码；三写脸接线的验收面见
// tests/pdfp_docmdp.rs——本文件只锁纯函数面）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn fixture_doc() -> Document {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/small.pdf"),
        )
        .expect("small.pdf 缺失");
        Document::load_mem(&bytes).expect("small.pdf 应可装载")
    }

    /// 直接字典形态注入 DocMDP（stamp.rs #408 测试同款纪律：夹具
    /// 无签名基座，逐级注入目录标记）。
    fn inject_doc_mdp(doc: &mut Document, p: i64) {
        doc.catalog_mut().unwrap().set(
            "Perms",
            dictionary! {
                "DocMDP" => dictionary! {
                    "Type" => "Sig",
                    "Reference" => Object::Array(vec![Object::Dictionary(
                        dictionary! {
                            "Type" => "SigRef",
                            "DigestMethod" => "SHA256",
                            "TransformParams" => dictionary! { "P" => p, "V" => "R" },
                        },
                    )]),
                },
            },
        );
    }

    #[test]
    fn plain_doc_has_no_doc_mdp() {
        assert_eq!(detect_doc_mdp(&fixture_doc()), None, "small.pdf 无 DocMDP");
    }

    #[test]
    fn detect_p1_p2_p3_direct_form() {
        for (p, expect) in [
            (1, DocMdpLevel::P1),
            (2, DocMdpLevel::P2),
            (3, DocMdpLevel::P3),
        ] {
            let mut doc = fixture_doc();
            inject_doc_mdp(&mut doc, p);
            assert_eq!(
                detect_doc_mdp(&doc),
                Some(expect),
                "直接字典形态 P={p} 应解析为 {expect:?}"
            );
        }
    }

    #[test]
    fn detect_indirect_reference_forms() {
        // /Perms 与 /TransformParams 均走间接引用（真实生成器常见形态
        // ——大字典常独立成对象）。/DocMDP 直接、SigRef 引用两形态混合。
        let mut doc = fixture_doc();
        let tp_id = doc.add_object(dictionary! { "P" => 2, "V" => "R" });
        let sigref_id = doc.add_object(dictionary! {
            "Type" => "SigRef",
            "TransformParams" => Object::Reference(tp_id),
        });
        let docmdp_id = doc.add_object(dictionary! {
            "Type" => "Sig",
            "Reference" => Object::Array(vec![Object::Reference(sigref_id)]),
        });
        let perms_id = doc.add_object(dictionary! { "DocMDP" => Object::Reference(docmdp_id) });
        doc.catalog_mut()
            .unwrap()
            .set("Perms", Object::Reference(perms_id));
        assert_eq!(
            detect_doc_mdp(&doc),
            Some(DocMdpLevel::P2),
            "间接引用四层形态应解析"
        );
    }

    #[test]
    fn malformed_transform_fail_closed_p1() {
        // /Reference 缺失。
        let mut doc = fixture_doc();
        doc.catalog_mut().unwrap().set(
            "Perms",
            dictionary! { "DocMDP" => dictionary! { "Type" => "Sig" } },
        );
        assert_eq!(
            detect_doc_mdp(&doc),
            Some(DocMdpLevel::P1),
            "Reference 缺失应 P1"
        );

        // TransformParams 缺失（SigRef 空）。
        let mut doc = fixture_doc();
        doc.catalog_mut().unwrap().set(
            "Perms",
            dictionary! {
                "DocMDP" => dictionary! {
                    "Reference" => Object::Array(vec![Object::Dictionary(
                        dictionary! { "Type" => "SigRef" },
                    )]),
                },
            },
        );
        assert_eq!(
            detect_doc_mdp(&doc),
            Some(DocMdpLevel::P1),
            "TransformParams 缺失应 P1"
        );

        // /P 缺失。
        let mut doc = fixture_doc();
        doc.catalog_mut().unwrap().set(
            "Perms",
            dictionary! {
                "DocMDP" => dictionary! {
                    "Reference" => Object::Array(vec![Object::Dictionary(
                        dictionary! { "TransformParams" => dictionary! { "V" => "R" } },
                    )]),
                },
            },
        );
        assert_eq!(detect_doc_mdp(&doc), Some(DocMdpLevel::P1), "/P 缺失应 P1");

        // /P 非整数（字符串）与超范围（7）——同归 P1。
        for bad in [
            Object::String(b"2".to_vec(), lopdf::StringFormat::Literal),
            Object::Integer(7),
        ] {
            let mut doc = fixture_doc();
            doc.catalog_mut().unwrap().set(
                "Perms",
                dictionary! {
                    "DocMDP" => dictionary! {
                        "Reference" => Object::Array(vec![Object::Dictionary(
                            dictionary! { "TransformParams" => dictionary! { "P" => bad.clone() } },
                        )]),
                    },
                },
            );
            assert_eq!(
                detect_doc_mdp(&doc),
                Some(DocMdpLevel::P1),
                "畸形 /P（{bad:?}）应 fail-closed P1"
            );
        }
    }

    #[test]
    fn multi_sigref_takes_most_restrictive() {
        let mut doc = fixture_doc();
        doc.catalog_mut().unwrap().set(
            "Perms",
            dictionary! {
                "DocMDP" => dictionary! {
                    "Reference" => Object::Array(vec![
                        Object::Dictionary(dictionary! {
                            "TransformParams" => dictionary! { "P" => 3 },
                        }),
                        Object::Dictionary(dictionary! {
                            "TransformParams" => dictionary! { "P" => 2 },
                        }),
                    ]),
                },
            },
        );
        assert_eq!(
            detect_doc_mdp(&doc),
            Some(DocMdpLevel::P2),
            "多 SigRef 许可面取交集（最严）"
        );
    }

    #[test]
    fn policy_matrix() {
        let plain = fixture_doc();
        for op in [
            WriteOp::ContentRewrite,
            WriteOp::Annotation,
            WriteOp::FormFill,
            WriteOp::Signature,
        ] {
            assert!(
                ensure_write_allowed(&plain, op).is_ok(),
                "无 DocMDP：{op} 应放行"
            );
        }

        let matrix: &[(DocMdpLevel, WriteOp, bool)] = &[
            // P=1：全拒。
            (DocMdpLevel::P1, WriteOp::Annotation, false),
            (DocMdpLevel::P1, WriteOp::FormFill, false),
            (DocMdpLevel::P1, WriteOp::Signature, false),
            (DocMdpLevel::P1, WriteOp::ContentRewrite, false),
            // P=2：标注拒，表单/签署放，内容重写拒。
            (DocMdpLevel::P2, WriteOp::Annotation, false),
            (DocMdpLevel::P2, WriteOp::FormFill, true),
            (DocMdpLevel::P2, WriteOp::Signature, true),
            (DocMdpLevel::P2, WriteOp::ContentRewrite, false),
            // P=3：标注/表单/签署放，内容重写拒。
            (DocMdpLevel::P3, WriteOp::Annotation, true),
            (DocMdpLevel::P3, WriteOp::FormFill, true),
            (DocMdpLevel::P3, WriteOp::Signature, true),
            (DocMdpLevel::P3, WriteOp::ContentRewrite, false),
        ];
        for (p, op, allow) in matrix {
            let mut doc = fixture_doc();
            inject_doc_mdp(&mut doc, p.as_u8() as i64);
            let r = ensure_write_allowed(&doc, *op);
            assert_eq!(
                r.is_ok(),
                *allow,
                "P={} {} 应{}（实测 {r:?}）",
                p.as_u8(),
                op,
                if *allow { "放行" } else { "被拒" }
            );
        }
    }

    #[test]
    fn reason_codes_are_frozen_literals() {
        // 协议面冻结锚——宿主按码分流，字面变更 = 破坏性改动。
        assert_eq!(REASON_DOCMDP_P1_NO_CHANGES, "docmdp-p1-no-changes");
        assert_eq!(REASON_DOCMDP_P2_ANNOTATION, "docmdp-p2-annotation-denied");
        assert_eq!(REASON_DOCMDP_CONTENT_REWRITE, "docmdp-content-rewrite");
        assert_eq!(REASON_DOCMDP_POLICY, "docmdp-policy");

        let v = DocMdpViolation {
            level: DocMdpLevel::P2,
            op: WriteOp::Annotation,
        };
        assert_eq!(v.reason_code(), REASON_DOCMDP_P2_ANNOTATION);
        assert_eq!(v.level(), DocMdpLevel::P2);
        assert_eq!(v.op(), WriteOp::Annotation);
        assert!(v.to_string().contains("P=2"), "人读文案带 P 级：{}", v);
    }
}
