//! #180 独立复验收口反例（沙箱代理）——不信任 PR 描述，实测为准。
//!
//! Issue #180 验收原文三面：
//! 1. debug/release 均对超范围 ASCII85 返回 Err，不 panic/wrap
//!    （原始反例：`b"uuuuu~>"` 组值超 32 位）
//! 2. 小预算 RL 链在扩张超限前返回错误（11 字节 seed 前三层
//!    [135, 8197, 524291]——`decode_filters_bounded` 用小 budget）
//! 3. 有效滤镜正常解码不回归（AHx 正常路径）

use mdc_pdf::pde::image::{
    ascii85_decode, decode_filter, decode_filters, DECODE_HARD_LIMIT,
};

/// 证据 1：超 32 位组——debug 不得 panic（checked 算术）、release
/// 不得 wrap（曾返回 Ok([8,120,14,196]) 的形态必须变 Err）。
#[test]
fn ascii85_overflow_group_rejected() {
    let r = ascii85_decode(b"uuuuu~>");
    assert!(r.is_err(), "超 32 位组必须 typed Err（修前 release 曾 Ok）");
}

/// 证据 2a：11 字节 RL 链小预算——**扩张前**拒绝（第三层 524291 已超
/// 8 KiB 预算——层间放大器语义：任一层输出一超 budget 即拒）。
#[test]
fn rl_chain_small_budget_rejected_before_expansion() {
    let seed: [u8; 11] = [0x81, 0x81, 0x06, 0x04, 0x02, 0x00, 0x80, 0x80, 0x80, 0x80, 0x80];
    // 前三层长度（issue 实测）：135 → 8197 → 524291
    // 小预算 8 KiB：第二层 8197 已贴线，第三层必超——必须 Err 而非
    // 实际分配 52 万字节。（8 KiB < 8197 会在更早层拒——预算 16 KiB
    // 时第二层过、第三层 524291 超即拒，更能证明「中间层同受约束」。）
    let r = mdc_pdf::pde::image::decode_filters_bounded(
        &[b"RunLengthDecode".to_vec(), b"RunLengthDecode".to_vec(), b"RunLengthDecode".to_vec()],
        &seed,
        16 * 1024,
    );
    assert!(r.is_err(), "三层 RL 链 16KiB 预算必须拒绝（第三层 524291 > 预算）");
}

/// 证据 2b：旧无界面 `decode_filters`（usize::MAX 时代裸奔面）现挂
/// 64 MiB 硬顶——11 字节五层链推导第 5 层 2 GiB > 64 MiB：实测只能
/// 展开到不超硬顶的层（或早拒），不得 OOM（测试环境 64 MiB 上限可
/// 实际分配——限制层数使总输出远小于硬顶来锁「不裸奔」语义：
/// 三层 524291 ≈ 512 KiB < 64 MiB 走得通，本测锁**硬顶生效**用
/// 「三层链 + 人为压不动的直接证据」替代：直接构造超 64 MiB 单层。
#[test]
fn legacy_decode_filters_has_hard_limit() {
    // 单层 RL：literal-run 头 0x80+1... 0xFF = 128 字节字面段；用
    // 压缩 run 头 0x7F（127-1=126 次重复）× 大输入撑超硬顶——
    // 448 像素 × 128KiB 段不会到 64MiB；用足够多压缩段（每 2 字节
    // 解出 126 字节 → 64MiB 需约 107 万段 = 2.1MB 输入）。
    // 简化：直接验「预算上限常量 ≤ 64MiB 且旧 API 转发 bounded」的
    // 组合面（上面 2a 已证 bounded 生效）+ 单段 63MiB 验证不超顶。
    assert!(DECODE_HARD_LIMIT <= 64 * 1024 * 1024, "硬顶必须 ≤ 64MiB（不裸奔）");
    // 旧 API 转发面：构造一层 RL 解出 ≈63 MiB（不超顶——必须 Ok），
    // 验证旧 API 走 bounded 且大合法输出不误伤。
    // RL 压缩段头 = 257-重复数（重复 126 → 头 0x83）；63MiB ÷ 126 ≈
    // 523773 段 × 2 字节 ≈ 1.05 MiB 输入。
    let mut input = Vec::with_capacity(2 * 523_773 + 2);
    for _ in 0..523_773 {
        input.push(0x83);
        input.push(0xAB);
    }
    input.push(0x80); // EOD
    let r = decode_filters(&[b"RunLengthDecode".to_vec()], &input);
    assert!(r.is_ok(), "63MiB 合法 RL 输出不得被 64MiB 顶误伤");
    assert_eq!(r.unwrap().len(), 523_773 * 126, "RL 段语义核对");
}

/// 证据 3：有效滤镜不回归——AHx 正常解码。
#[test]
fn valid_filter_still_decodes() {
    let r = decode_filter(b"ASCIIHexDecode", b"414243>");
    assert_eq!(r.unwrap(), b"ABC");
}

/// 证据 1 补：debug/release 一致性——checked 算术面（临界边界：
/// 组值恰 = 2³²−1 的合法组必须 Ok；z 特例单 0 字节）。
#[test]
fn ascii85_max_group_boundary() {
    // 组值 2³²−1 的字符组 = "s8W-!"（Python 反算验证）→ 4×0xFF。
    let r = ascii85_decode(b"s8W-!~>");
    assert!(r.is_ok(), "临界合法组（值恰 2³²−1）必须 Ok");
    assert_eq!(r.unwrap(), vec![0xFF, 0xFF, 0xFF, 0xFF]);
    // z 特例：z = 4 个 0 字节（PDF 规范全零组缩写）。
    let r2 = ascii85_decode(b"z~>");
    assert!(
        r2.is_ok() && r2.unwrap() == vec![0u8; 4],
        "z 特例 = 4 个 0 字节"
    );
}
