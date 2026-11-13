//! pde —— PDE 兼容抽象层（issue #162，Adobe PDFEdit/PDE 产品架构参照）。
//!
//! 以 **Adobe Acrobat SDK 的 PDFEdit API（PDE 层）** 为产品架构参照：
//! "以 Adobe PDFEdit/PDE 为产品架构参照，以 PDFium/MuPDF 为底层实现
//! 基础"（用户主指令）。Adobe 的 PDE 层是「页面内容对象模型」——把
//! 任意 PDF 内容流重建为可编辑对象树，编辑后可靠写回；本模块在
//! mdcore 建立同构抽象层，补足开源引擎之间的差异。
//!
//! # 分层与批次
//!
//! - **批①（本版）**：对象模型 + 内容流 tokenizer + 解析器 + 写回，
//!   零引擎依赖（纯 Rust）——「任意内容流到可编辑对象的重建」的
//!   核心闭环。Adobe 类对照见 [`model`] 模块文档表。
//! - **批②（本版增量）**：资源表解引用（`Do` 名字 → XObject 对象
//!   → form 递归 PdeContent——`PDEFormGetContent` 面，见 [`resources`]）
//!   + PDEImage 数据面（FlateDecode/AHx/A85/RL 滤镜解码，见
//!   [`image`]）+ 写回资源收集（`PDEContentToCosObj` 的 resourcesP
//!   输出面，见 [`resources::collect_resources`]）。对象图形态为自带
//!   值模型 [`resources::ResValue`]（lopdf 互转门归引擎挂接层批③）。
//! - **批③**：PDEText 编辑深面（ReplaceChars/SplitRunAt 实语义 +
//!   PDEFont 度量——字符寻址 kPDETextChar 面）+ DocumentEngine 挂接。
//! - **批④（远期）**：PDSEdit 第二目标（逻辑结构树/阅读顺序/段落
//!   表格结构——对 AI 理解文档重要）。
//!
//! # 与现有 PDF 面的关系
//!
//! mdc-pdf 现有面（PDF-c 阅读/PDF-d 标注增量写）的引擎数据源是
//! pdfium；PDE 层在其**上方**提供对象模型级编辑能力（Adobe 分层：
//! PDPage → PDEContent → PDEElement），写回走内容流重生成。零引擎
//! 依赖纪律与 crate 现有 default = [] 面一致——批①不引入任何新依赖。

pub mod font;
pub mod image;
pub mod lexer;
pub mod model;
pub mod parser;
pub mod resources;
pub mod writer;

pub use model::{
    Clip, ColorSpec, ColorValues, GState, Matrix, Operand, PaintOp, PathSeg, PdePath, PdePlace,
    PdeShading, PdeText, PdeUnknown, PdeXObject, TextRun, TextState, IDENTITY, PDE_BEFORE_FIRST,
};
