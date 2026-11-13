//! codec —— 最小 zlib/DEFLATE 解压（crate 级共享件）。
//!
//! 2026-11-13 从 stamp.rs `flate` 模块上移（issue #162 PDE 批②）：
//! PDE 层（`pde::image`，FlateDecode 滤镜）与盖章层（stamp.rs，PNG
//! IDAT）共用同一实现。**零引擎依赖纪律**——本模块纯 std，不被任何
//! feature 门控（stamp 原实现挂在 engine-lopdf 门内，PDE 层
//! default = [] 也要用它，故上移为 crate 级公共件）。
//!
//! 为什么自己做（stamp.rs 原注释保留）：本 crate 的依赖纪律是
//! 「零 image crate」，而 flate2 是 lopdf 的内部依赖（未在本 crate
//! 直接声明）。PNG 的 IDAT 必须是 zlib，没有它读不了任何真实公章
//! PNG。三百行不到的 inflate 比引入一个新三方依赖更符合本 crate 的
//! 既有纪律（`engine_pdfium` 渲染面同理不引 image）。
//! （signature.rs 手写 zlib inflate 同一条纪律：能自己写清楚的，
//! 不引依赖。）

/// zlib 包装（2 字节头 + deflate + 4 字节 adler32）。
///
/// 存储块 + fixed/dynamic Huffman 全形态（PNG IDAT 与 PDF
/// FlateDecode 滤镜共用面）。
///
/// # 安全面（#180 修复轮，2026-11-13）
///
/// - **Adler-32 校验**：尾 4 字节大端读出与输出比对——旧实现
///   读都不读（stamp.rs 原样迁移的旧债；stamp 信任域=自家公章
///   PNG，PDE 面解任意不可信文档流，旧债升险——#180 修复轮收口）。
/// - **输出预算**：无预算的 zlib 解码是 DoS 放大器（11 字节输入可
///   膨胀 47,662 倍——Lead #180 实测）。本函数默认无上限（stamp
///   PNG IDAT 已知几何面在外层），有界面走 [`zlib_inflate_bounded`]。
pub fn zlib_inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    zlib_inflate_bounded(data, usize::MAX)
}

/// 有预算的 zlib 解压（#180：解码输出限额面）。
///
/// `budget` = 输出字节上限（usize::MAX = 无限）。超预算 = Err（错误
/// 信息含实际膨胀字节数——诊断面）。头校验 + Adler-32 校验与
/// [`zlib_inflate`] 同门。
pub fn zlib_inflate_bounded(data: &[u8], budget: usize) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib 数据过短".into());
    }
    if data[0] & 0x0F != 8 {
        return Err(format!("非 deflate 压缩方法：{}", data[0] & 0x0F));
    }
    let header = data[0] as u16 * 256 + data[1] as u16;
    if !header.is_multiple_of(31) {
        return Err("zlib 头校验失败".into());
    }
    let mut out = Vec::new();
    inflate_deflate(&data[2..data.len() - 4], &mut out, budget)?;
    // Adler-32 校验（#180：尾 4 字节大端与输出比对——不匹配即 Err，
    // 不再静默接受错数据）。
    let expect = u32::from_be_bytes([
        data[data.len() - 4],
        data[data.len() - 3],
        data[data.len() - 2],
        data[data.len() - 1],
    ]);
    let actual = adler32(&out);
    if expect != actual {
        return Err(format!(
            "Adler-32 校验失败：期望 0x{expect:08X} 实得 0x{actual:08X}（数据损坏或截断）"
        ));
    }
    Ok(out)
}

/// Adler-32（RFC 1950——zlib 尾校验和）。
///
/// 参考实现与 Python `zlib.adler32` 逐字节核对（含 10 万字节流），
/// 常量 65521 = 4 字节内最大素数（RFC 规定值）。
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits {
            data,
            pos: 0,
            bit: 0,
        }
    }
    fn get(&mut self, n: u32) -> Result<u32, String> {
        let mut v = 0u32;
        for i in 0..n {
            let byte = *self.data.get(self.pos).ok_or("deflate 数据提前结束")?;
            let b = (byte >> self.bit) & 1;
            v |= (b as u32) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Ok(v)
    }
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }
}

struct Huff {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huff {
    fn new(lengths: &[u8]) -> Self {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Huff { counts, symbols }
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, String> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..16 {
            code |= bits.get(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                let i = (index + (code - first)) as usize;
                return self
                    .symbols
                    .get(i)
                    .copied()
                    .ok_or_else(|| "符号越界".to_string());
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("无效 Huffman 码".into())
    }
}

fn inflate_deflate(input: &[u8], out: &mut Vec<u8>, budget: usize) -> Result<(), String> {
    let mut bits = Bits::new(input);
    loop {
        let last = bits.get(1)?;
        let btype = bits.get(2)?;
        match btype {
            0 => {
                bits.align();
                let len = (bits.get(16)?) as usize;
                let _nlen = bits.get(16)?;
                for _ in 0..len {
                    if out.len() >= budget {
                        return Err(budget_err(out.len(), budget));
                    }
                    out.push(bits.get(8)? as u8);
                }
            }
            1 => {
                let mut lit = [0u8; 288];
                for (i, v) in lit.iter_mut().enumerate() {
                    *v = if i < 144 {
                        8
                    } else if i < 256 {
                        9
                    } else if i < 280 {
                        7
                    } else {
                        8
                    };
                }
                let lit_h = Huff::new(&lit);
                let dist_h = Huff::new(&[5u8; 30]);
                inflate_block(&mut bits, out, &lit_h, &dist_h, budget)?;
            }
            2 => {
                let hlit = bits.get(5)? as usize + 257;
                let hdist = bits.get(5)? as usize + 1;
                let hclen = bits.get(4)? as usize + 4;
                const ORDER: [usize; 19] = [
                    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ];
                let mut cl = [0u8; 19];
                for i in 0..hclen {
                    cl[ORDER[i]] = bits.get(3)? as u8;
                }
                let cl_h = Huff::new(&cl);
                let mut lengths = Vec::with_capacity(hlit + hdist);
                while lengths.len() < hlit + hdist {
                    let sym = cl_h.decode(&mut bits)?;
                    match sym {
                        0..=15 => lengths.push(sym as u8),
                        16 => {
                            let prev = *lengths.last().ok_or("重复码无前值")?;
                            let rep = bits.get(2)? + 3;
                            for _ in 0..rep {
                                lengths.push(prev);
                            }
                        }
                        17 => {
                            let rep = bits.get(3)? + 3;
                            lengths.resize(lengths.len() + rep as usize, 0);
                        }
                        18 => {
                            let rep = bits.get(7)? + 11;
                            lengths.resize(lengths.len() + rep as usize, 0);
                        }
                        _ => return Err("无效重复码".into()),
                    }
                }
                let lit_h = Huff::new(&lengths[..hlit]);
                let dist_h = Huff::new(&lengths[hlit..]);
                inflate_block(&mut bits, out, &lit_h, &dist_h, budget)?;
            }
            _ => return Err("无效 block type 3".into()),
        }
        if last == 1 {
            break;
        }
    }
    Ok(())
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn inflate_block(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    lit: &Huff,
    dist: &Huff,
    budget: usize,
) -> Result<(), String> {
    loop {
        let sym = lit.decode(bits)?;
        match sym {
            0..=255 => {
                if out.len() >= budget {
                    return Err(budget_err(out.len(), budget));
                }
                out.push(sym as u8);
            }
            256 => break,
            257..=285 => {
                let i = (sym - 257) as usize;
                let len = LEN_BASE[i] as usize + bits.get(LEN_EXTRA[i] as u32)? as usize;
                let dsym = dist.decode(bits)? as usize;
                if dsym >= DIST_BASE.len() {
                    return Err("无效距离码".into());
                }
                let d = DIST_BASE[dsym] as usize + bits.get(DIST_EXTRA[dsym] as u32)? as usize;
                if d > out.len() {
                    return Err("回溯距离超出输出".into());
                }
                if out.len() + len > budget {
                    return Err(budget_err(out.len() + len, budget));
                }
                let start = out.len() - d;
                for k in 0..len {
                    let b = out[start + k];
                    out.push(b);
                }
            }
            _ => return Err("无效字面码".into()),
        }
    }
    Ok(())
}

/// 预算超限错误（#180——错误信息带膨胀字节数，诊断面）。
fn budget_err(at: usize, budget: usize) -> String {
    format!("解码输出超预算：第 {at} 字节处超出上限 {budget}（膨胀炸弹拒绝）")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// zlib.compress(bytes(range(8))) 固定向量（Python zlib 标准产出，
    /// adler-32 = 0x5C001D——机器生成核验）。
    const ZLIB_RANGE8: &[u8] = &[
        0x78, 0x9c, 0x63, 0x60, 0x64, 0x62, 0x66, 0x61, 0x65, 0x63, 0x07, 0x00, 0x00, 0x5c, 0x00,
        0x1d,
    ];

    #[test]
    fn zlib_roundtrip_range8() {
        let out = zlib_inflate(ZLIB_RANGE8).unwrap();
        assert_eq!(out, bytes_range8());
    }

    fn bytes_range8() -> Vec<u8> {
        (0u8..8).collect()
    }

    /// #180 旧债收口：Adler-32 校验——篡改尾 4 字节必须 Err。
    ///
    /// round143 复现向量：合法流尾字节颠倒（0x005C001D → 0x018E0400），
    /// 旧实现 Ok(10) 静默接受错数据。
    #[test]
    fn zlib_adler32_tamper_rejected() {
        let mut z = ZLIB_RANGE8.to_vec();
        z[12..16].copy_from_slice(&[0x01, 0x8e, 0x04, 0x00]); // 篡改 adler
        let r = zlib_inflate(&z);
        assert!(r.is_err(), "篡改 adler 必须拒绝");
        let msg = r.unwrap_err();
        assert!(msg.contains("Adler-32"), "错误信息含校验名：{msg}");
    }

    /// #180 预算面：合法流在超紧预算下拒绝（错误信息带膨胀字节数）。
    #[test]
    fn zlib_budget_rejects_early() {
        // 合法 8 字节输出，预算 4 → 第 4 字节即拒
        let r = zlib_inflate_bounded(ZLIB_RANGE8, 4);
        assert!(r.is_err());
        let msg = r.unwrap_err();
        assert!(msg.contains("超预算"), "错误信息带预算语义：{msg}");
        // 预算 = 输出长度 → 通过（边界含等号——恰好放行）
        assert!(zlib_inflate_bounded(ZLIB_RANGE8, 8).is_ok());
    }

    /// Adler-32 实现自证：与 RFC 1950 参考向量核对。
    ///
    /// Python zlib.adler32(b"")=1 / (b"Hello")=0x58C01F5——机器核验。
    #[test]
    fn adler32_known_values() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Hello"), 0x58C01F5);
        assert_eq!(adler32(&bytes_range8()), 0x5C001D);
    }
}
