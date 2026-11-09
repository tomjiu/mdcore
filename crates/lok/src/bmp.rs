//! BGRA tile → BMP（24/32 位）side file 写出 + 非空像素统计。
//!
//! LO Linux（cairo 后端）tile 缓冲为 **BGRA**、stride = width*4（ElyOffice
//! 实证；本 crate 以 `SAL_DISABLESKIA=1` 强制 cairo 保序）。BMP 行序自下
//! 而上、4 字节对齐——32 位 BGRA 源直接转 BGRA-BMP（BITMAPV4HEADER +
//! BI_BITFIELDS）最小搬运。
//!
//! 「非空实际像素」验收（issue：不能用 1x1 占位像素冒充）：统计唯一颜色数、
//! 非白/非黑占比、左上/中心/右下采样点，供 smoke 断言。

use std::io::Write;
use std::path::Path;

/// paintTile 结果统计（证明非空渲染）。
#[derive(Debug, Clone, PartialEq)]
pub struct TileStats {
    pub width: usize,
    pub height: usize,
    /// 唯一颜色数（BGRA → u32）。
    pub unique_colors: usize,
    /// 非白像素占比（0.0-1.0）。全白 = 空白页。
    pub non_white_ratio: f64,
    /// 非黑非白像素占比（有实际着色内容，如红色探针图片/黑字边缘抗锯齿）。
    pub colored_ratio: f64,
}

/// 统计 BGRA 缓冲（不做 IO，worker 与测试共用）。
pub fn stats_bgra(buf: &[u8], w: usize, h: usize) -> TileStats {
    assert_eq!(buf.len(), w * h * 4, "BGRA 缓冲长度必须 = w*h*4");
    let mut colors = std::collections::HashSet::new();
    let mut non_white = 0usize;
    let mut colored = 0usize;
    for px in buf.as_chunks::<4>().0 {
        let (b, g, r, a) = (px[0], px[1], px[2], px[3]);
        colors.insert((b, g, r, a));
        let near = |v: u8, t: u8| v.abs_diff(t) <= 16;
        let is_white = near(b, 255) && near(g, 255) && near(r, 255);
        let is_black = near(b, 0) && near(g, 0) && near(r, 0);
        if !is_white {
            non_white += 1;
        }
        if !is_white && !is_black {
            colored += 1;
        }
    }
    let total = (w * h) as f64;
    TileStats {
        width: w,
        height: h,
        unique_colors: colors.len(),
        non_white_ratio: non_white as f64 / total,
        colored_ratio: colored as f64 / total,
    }
}

/// BGRA 缓冲 → 32 位 BMP（BI_BITFIELDS，行序翻转），写出 side file。
pub fn write_bmp(path: &Path, buf: &[u8], w: usize, h: usize) -> std::io::Result<()> {
    assert_eq!(buf.len(), w * h * 4);
    let stride = w * 4; // 已 4 对齐
    let data_len = stride * h;
    // BITMAPV4HEADER（108 字节）+ BGRA 掩码 → 可靠的 32 位 BMP
    let mut out = std::fs::File::create(path)?;
    let mut head = Vec::with_capacity(14 + 108);
    head.extend_from_slice(b"BM");
    head.extend_from_slice(&((14 + 108 + data_len) as u32).to_le_bytes());
    head.extend_from_slice(&[0; 4]); // reserved
    head.extend_from_slice(&((14 + 108) as u32).to_le_bytes()); // pixel offset
    head.extend_from_slice(&108u32.to_le_bytes()); // BITMAPV4HEADER size
    head.extend_from_slice(&(w as i32).to_le_bytes());
    head.extend_from_slice(&(h as i32).to_le_bytes());
    head.extend_from_slice(&1u16.to_le_bytes()); // planes
    head.extend_from_slice(&32u16.to_le_bytes()); // bpp
    head.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    head.extend_from_slice(&(data_len as u32).to_le_bytes());
    head.extend_from_slice(&2835u32.to_le_bytes()); // 72 DPI
    head.extend_from_slice(&2835u32.to_le_bytes());
    head.extend_from_slice(&[0; 8]); // colors used/important
    head.extend_from_slice(&0x00FF0000u32.to_le_bytes()); // R mask
    head.extend_from_slice(&0x0000FF00u32.to_le_bytes()); // G mask
    head.extend_from_slice(&0x000000FFu32.to_le_bytes()); // B mask
    head.extend_from_slice(&0xFF000000u32.to_le_bytes()); // A mask
    head.extend_from_slice(&b"BgRs".map(|c| c)); // CSType = sRGB 常量（'sRGB' LE）
    head.extend_from_slice(&[0; 48]); // CIEXYZTRIPLE(36) + gamma r/g/b(12) = V4 头补足 108
    out.write_all(&head)?;
    // 行序翻转（BMP 自下而上）
    for y in (0..h).rev() {
        let row = &buf[y * stride..(y + 1) * stride];
        out.write_all(row)?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_detect_blank_vs_content() {
        let blank = vec![0xFF; 8 * 4 * 4]; // 全白 BGRA
        let s = stats_bgra(&blank, 8, 4);
        assert_eq!(s.unique_colors, 1);
        assert_eq!(s.non_white_ratio, 0.0);
        assert_eq!(s.colored_ratio, 0.0);

        // 4x4 中前 4 像素红、其余白（探针图片色占比 25%）
        let mut buf = vec![0xFFu8; 16 * 4];
        for i in 0..4 {
            buf[i * 4] = 30;
            buf[i * 4 + 1] = 30;
            buf[i * 4 + 2] = 200;
        }
        let s = stats_bgra(&buf, 4, 4);
        assert_eq!(s.unique_colors, 2);
        assert_eq!(s.non_white_ratio, 0.25);
        assert!(
            (s.colored_ratio - 0.25).abs() < 1e-9,
            "红像素占比应 25%：{s:?}"
        );
    }

    #[test]
    fn bmp_header_shape() {
        let w = 2usize;
        let h = 2usize;
        // 左上红、右上绿、左下蓝、右下白
        let mut buf = vec![0u8; w * h * 4];
        let put = |buf: &mut [u8], x: usize, y: usize, b: u8, g: u8, r: u8| {
            let i = (y * w + x) * 4;
            buf[i] = b;
            buf[i + 1] = g;
            buf[i + 2] = r;
            buf[i + 3] = 255;
        };
        put(&mut buf, 0, 0, 30, 30, 200); // 红
        put(&mut buf, 1, 0, 30, 200, 30); // 绿
        put(&mut buf, 0, 1, 200, 30, 30); // 蓝
        put(&mut buf, 1, 1, 255, 255, 255); // 白
        let path = std::env::temp_dir().join("mdc-lok-bmp-ut.bmp");
        write_bmp(&path, &buf, w, h).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..2], b"BM");
        assert_eq!(u32::from_le_bytes(bytes[14..18].try_into().unwrap()), 108); // V4 头
        assert_eq!(
            u32::from_le_bytes(bytes[18..22].try_into().unwrap()),
            w as u32
        );
        // 像素区首行 = 源最后一行（BMP 自下而上）：蓝、白
        let off = 14 + 108;
        assert_eq!(&bytes[off..off + 4], &[200, 30, 30, 255]); // (0,1) 蓝
        assert_eq!(&bytes[off + 4..off + 8], &[255, 255, 255, 255]); // (1,1) 白
        let _ = std::fs::remove_file(&path);
    }
}
