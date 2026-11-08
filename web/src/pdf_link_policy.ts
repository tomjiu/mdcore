/**
 * #557：PDF 外链输入策略单一模块（pdf_view.ts 消费）。
 *
 * 语义分层（#466 既有语义全保留 + #557 输入策略两项）：
 *
 * 1. 输入预算 `URL_INPUT_BUDGET = 2048`——**项目输入预算，非 URL 协议上限**。
 *    对原始串与规范化串（`new URL().href`）执行一致上限：非 ASCII 百分号
 *    化会膨胀（1 CJK 字 → 9 href 字符），两侧都过才算过。
 * 2. 零宽/双向控制码显式拒绝——两种形态都拦：
 *    · 原始字符形态（U+200B-200F 零宽族 / U+202A-202E Bidi 覆盖 /
 *      U+2066-2069 Bidi 隔离 / U+2028-2029 行段分隔 / U+FEFF 零宽 BOM）
 *    · 有效百分号编码形态（%E2%80%8B 等）——**单次解码**后检测；
 *      不做双重解码（%25E2%80%8B 双编形态解码一次后是字面
 *      `%E2%80%8B` 文本，无控制码——不误伤，浏览器同款语义）。
 * 3. 绝对 URL + scheme 白名单 http/https/mailto（#466 语义原样）。
 *
 * 拒绝语义（pdf_view 消费侧）：不生成外链热区 + reason 纳入
 * linkPolicySkipped 可观察计数——不静默、不让整份 PDF 打开失败。
 *
 * 纯函数模块（无 DOM 依赖）——e2e 经 `__pdfLinkPolicy` 探针直测。
 */

/** 输入预算（项目输入预算口径——非 URL 协议上限，协议 8K/64K 不背书）。 */
export const URL_INPUT_BUDGET = 2048;

/** 零宽与 Bidi 控制码检测（显式码点区间，不用 \p{Cf} 泛类——
 * 阿拉伯语合法格式字符（ALM U+061C 之外的连字等）不在拒绝面，
 * 避免误伤正常 RTL 文档的合法外链）。 */
const ZERO_WIDTH_BIDI_RE = /[\u200B-\u200F\u202A-\u202E\u2028\u2029\u2066-\u2069\uFEFF]/;

/** #568：检查用容错 UTF-8 解码器；坏字节替换，不遮蔽相邻合法控制码。
 * ignoreBOM=true 保留 U+FEFF，使编码 BOM 仍受控制码策略检查。
 * Source: https://nodejs.org/docs/latest-v24.x/api/util.html#new-textdecoderencoding-options
 */
const PERCENT_UTF8_DECODER = new TextDecoder('utf-8', {
  fatal: false,
  ignoreBOM: true,
});

/**
 * 单次百分号解码（仅检查面）：连续 %XX 段按 UTF-8 解码；非法字节替换，
 * 合法的相邻控制码仍可检测。截断/非 %XX 文本不二次解码，不修改返回 URL。
 */
function decodePercentOnce(s: string): string {
  return s.replace(/(?:%[0-9A-Fa-f]{2})+/g, (run) => {
    const bytes = new Uint8Array(run.length / 3);
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = Number.parseInt(run.slice(i * 3 + 1, i * 3 + 3), 16);
    }
    return PERCENT_UTF8_DECODER.decode(bytes);
  });
}

/** #466 外链 scheme 白名单（http/https/mailto——阅读器惯例）。 */
const URL_SCHEME_ALLOW = new Set(['http:', 'https:', 'mailto:']);

/** 拒绝原因（可观察契约——进 linkPolicySkipped 计数）。 */
export type LinkPolicyReason =
  | 'url-over-budget' // 原始或规范化超 2048
  | 'zero-width-bidi' // 原始控制码形态
  | 'zero-width-bidi-encoded' // 百分号编码形态的控制码
  | 'relative-url' // 非绝对 URL（#466：PDF 注释上下文无基准）
  | string; // scheme 不在白名单——值 = scheme 名（#446→#466 可观察惯例保持）

export interface LinkPolicyVerdict {
  /** 通过 = true（url 为规范化安全 href）；拒绝 = false（url 为 null，不生成热区）。 */
  ok: boolean;
  url: string | null;
  /** 拒绝原因（ok=false 时非空）；通过时 null。 */
  reason: LinkPolicyReason | null;
}

/**
 * 外链输入策略判定（pdf_view 链接读面唯一消费入口）。
 *
 * 检查序（每步显式 reason，先输入策略后既有语义——预算/控制码在解析前，
 * 超长输入不进 URL 构造器）：
 */
export function policyExternalUrl(raw: string): LinkPolicyVerdict {
  // ① 输入预算——原始串
  if (raw.length > URL_INPUT_BUDGET) {
    return { ok: false, url: null, reason: 'url-over-budget' };
  }
  // ② 零宽/Bidi——原始字符形态
  if (ZERO_WIDTH_BIDI_RE.test(raw)) {
    return { ok: false, url: null, reason: 'zero-width-bidi' };
  }
  // ③ 零宽/Bidi——有效百分号编码形态（单次解码；不双重解码）
  if (ZERO_WIDTH_BIDI_RE.test(decodePercentOnce(raw))) {
    return { ok: false, url: null, reason: 'zero-width-bidi-encoded' };
  }
  // ④ 绝对 URL（相对串在 PDF 注释上下文无基准——#466 语义）
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    return { ok: false, url: null, reason: 'relative-url' };
  }
  // ⑤ scheme 白名单（reason = scheme 名——沿用 #466 可观察惯例，
  //    既有 e2e 断言 policySkipped == ["javascript"] 口径不变）
  if (!URL_SCHEME_ALLOW.has(u.protocol)) {
    return { ok: false, url: null, reason: u.protocol.replace(/:$/, '') || 'bad-scheme' };
  }
  // ⑥ 输入预算——规范化串（与原始串一致上限：膨胀面在此拦截）
  if (u.href.length > URL_INPUT_BUDGET) {
    return { ok: false, url: null, reason: 'url-over-budget' };
  }
  return { ok: true, url: u.href, reason: null };
}
