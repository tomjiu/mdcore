//! #306：OCR 词模型——读面（[`crate::ocr`]）与写回面
//! （[`crate::ocr_writeback`]）的共享数据契约。
//!
//! 纯数据零引擎依赖（无 feature 门——pdfium 产词 / lopdf 消词两门
//! 独立组合时本类型恒在位，裸 crate 检查面不破）。

/// 词级 OCR 产物（一页一词行）。
///
/// 坐标系：**PDF 用户空间**（原点左下，pt）——tesseract 的图像坐标
/// （原点左上，像素）已在 [`crate::ocr::ocr_page_words`] 产出前完成
/// 翻转 + 缩放，写回与显示两消费方不再各自换算（单一换算点纪律）。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    /// 识别文本（tesseract TSV 词列原样——低置信词保留：搜索面宁多勿漏，
    /// 展示面由消费方决定是否按置信过滤）。
    pub text: String,
    /// 词框 `(x0, y0, x1, y1)`（pt，用户空间；y0 = 基线侧）。
    pub bbox: [f32; 4],
    /// 行锚 `(block, par, line)`（tesseract TSV 结构列——同锚词同行；
    /// 读面按锚分组成行、按 block/par 分段，保留视觉行序）。
    pub line: (u32, u32, u32),
}

impl OcrWord {
    /// 词框高（pt）——写回面的字号估计（词高 ≈ 字号：OCR 词框贴合
    /// 字形包围盒，含上下伸部的近似——隐形文本无需精排版，覆盖词框
    /// 即搜索命中区）。
    pub fn height_pt(&self) -> f32 {
        (self.bbox[3] - self.bbox[1]).max(1.0)
    }

    /// 基线 y（pt）——写回面的 `Td` 落点（词框下缘：tesseract 词框含
    /// 下伸部，基线略高于下缘，隐形文本取下缘即覆盖近似）。
    pub fn baseline_y(&self) -> f32 {
        self.bbox[1]
    }

    /// 是否全部 BMP 字符（写回面 Identity-H 的 2 字节 CID 面——
    /// 非 BMP 码位装不下，写回时逐词跳过该等字符并计数，不静默截断）。
    pub fn all_bmp(&self) -> bool {
        self.text.chars().all(|c| (c as u32) <= 0xFFFF)
    }
}
