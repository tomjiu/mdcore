// Phase 5 §八.4 — PDF 表面视图（web 面）：pdf.js 显示面 + WasmPdfSession
// （mdc-pdf wasm 桥）模型面。
//
// 架构（渲染分离、真相不分离——路线定谳 B）：
//
//   显示面 = pdf.js（Apache-2.0，首个三方运行时 npm 依赖——决策依据
//   §八.4「成熟组件复用」）：canvas 位图 + textLayer 文字层（原生文字
//   选区）+ 几何换算（viewport.convertToPdfPoint / convertToViewportPoint）。
//   pdf.js 只画，不持有任何「真相」。
//
//   模型面 = WasmPdfSession（web/pkg_pdf/——mdc-pdf 独立 wasm 包，主代
//   理并行实施中）：安全评估（pdf_open_scan）/ 标注库（Annotate* 命令）/
//   搜索（Search）/ 落盘字节（attach_writer + flush_annotations 增量写）
//   全部走会话——会话是单一真相，本模块不建第二份标注/文本数据。
//
//   文本层（v0 宿主注入通道——mdc-pdf document.rs set_page_text 的设计
//   降级路径，wasm32 无 pdfium 引擎时的唯一文本源）：pdf.js 逐页
//   getTextContent 抽取 → 会话 set_page_text 注入 → Search/ExtractText
//   在会话内执行（会话命中权威 + pdf.js 几何定位显示——双面诚实）。
//
//   页缓存（TS 侧，语义镜像 mdc-pdf PageCache）：(页号, scale) 键的
//   ImageBitmap LRU，字节预算 64MB；缩放变更失效缓存；远页 canvas 释放
//   （位图留缓存）；回滚命中直接贴位图。
//
//   wasm 桥缺位（pkg_pdf 未构建——主代理并行实施）：面板照常构建，
//   打开走 display-only 降级（pdf.js 渲染/选区/缩放可用；会话面全部
//   typed bridge_absent 失败），探针 __pdfState().wasm = false，e2e
//   会话类用例 SKIP（照 evidence_panel 模式）。
//
//   所有 wasm 调用包裹：抛出的 JSON 串解析为 typed {ok:false, code,
//   message}——本模块任何路径不向顶层抛出。
//
// #240 层③——打印面（web 打印态不再藏 PDF 面板）：
//
//   打印 CSS（index.html @media print）键于 body[data-pdf-active]
//   （setupTabs 维护）：PDF 页签激活时打印 PDF 页（canvas 位图 + 一页
//   一纸 break-after；工具栏/状态行/文字层/交互叠加层不进纸面），未激
//   活时维持旧行为（打印编辑器正文）。
//
//   全页渲染：懒渲染/远页 canvas 释放面向**屏幕**——打印需要每页位图
//   在场。打印按钮（#pdf-print-btn）= renderAllPages() → window.print()
//   → afterprint 恢复懒态（主路径，全保真）。Ctrl+P（beforeprint）无法
//   await（浏览器同步进预览快照）——best-effort 启动全页渲染，未完成
//   的页可能空白（诚实登记；打印按钮路径无此限制）。

import { wasmBoot } from './wasm_boot.js';
import { mount_surface_guide } from './empty_state.js';
import { t } from './i18n/index.js';
// #475(长尾⑨缩小版):注释列表面板——会话 AnnotListed 数据可视化
// (新模块:行形状 + DOM 纯函数;pdf_view 只做接线)。
import { renderAnnotListDom, type AnnotRowLike } from './pdf_annot_list.js';
// #472(长尾⑦):文档属性面板——pdf.js getMetadata + 会话/评估汇总
// (新模块:数据形状 + 字段表 DOM;pdf_view 只做接线)。
import {
  propFields,
  renderPropsDom,
  type DocProps,
  type PdfInfoLike,
} from './pdf_props.js';


// #463（长尾④）：大纲树——pdf.js getOutline → 引擎形状 + 树 DOM（新模块，
// pdf_view 只做接线：取数/解析回调/跳页回调/会话 ctor 注入）。
import {
  buildOutlineTree,
  flattenOutline,
  outlineTreeJson,
  renderOutlineDom,
  type OutlineNodeJson,
} from './pdf_outline.js';
import type {
  PageViewport,
  PDFDocumentProxy,
  PDFPageProxy,
  RenderTask,
} from 'pdfjs-dist';

/** TextContent（pdfjs 根出口未重导出——从页方法推导，免深路径依赖）。 */
type PdfTextContent = Awaited<ReturnType<PDFPageProxy['getTextContent']>>;

/** legacy 构建（WebView/旧引擎兼容：pdf.js v6 modern 构建依赖
 * Map.getOrInsertComputed 等新 API，legacy 构建内置 polyfill——宿主壳
 * webkit2gtk/JavaScriptCore 兼容面的稳妥路径）。 */
type PdfjsModule = typeof import('pdfjs-dist/legacy/build/pdf.mjs');

// ---------------------------------------------------------------------------
// wasm 桥类型（pkg_pdf 局部类型桥——照 evidence_panel 模式；pkg 构建前
// 以运行时鸭子检查探测，缺面 = typed bridge_absent）
// ---------------------------------------------------------------------------

/** WasmPdfSession 方法面（wasm-bindgen web-target，方法名 snake_case）。 */
interface WasmPdfSessionFace {
  /** 六命令单一调度入口（serde externally-tagged Command JSON）。 */
  execute(cmdJson: string): string;
  /** 会话时钟注入（宿主是唯一时间源——wasm32 无 std 时钟）。 */
  set_clock_ms(now: bigint): void;
  /** 装载 lopdf 增量写器（含打开扫描共用——首存懒装载）。 */
  attach_writer(bytes: Uint8Array): void;
  /** 保存事务：{bytes_appended, prefix_bytes} JSON；失败抛。 */
  flush_annotations(): string;
  /** 增量产物字节（prefix + appended——Blob 下载面）。 */
  product_bytes(): Uint8Array;
  /** v0 文本层注入（document.rs set_page_text 的桥面——无引擎环境唯一
   * 文本源；可选面：引擎侧自供文本的桥可缺省）。 */
  set_page_text?(page: number, text: string): unknown;
}

type WasmPdfSessionCtor = new (
  metaJson: string,
  assessmentJson: string,
  pagesJson: string,
  outlineJson: string,
) => WasmPdfSessionFace;

/** pkg_pdf 模块面（pdf_wasm.js——构建脚本 build-pdf-wasm.sh 产出）。 */
interface PdfWasmModule {
  pdf_open_scan?: (bytes: Uint8Array) => string;
  WasmPdfSession?: WasmPdfSessionCtor;
  default?: () => Promise<unknown>;
}

// ---------------------------------------------------------------------------
// serde 镜像（crates/pdf 各类型 JSON 形——构造集中于此，形态漂移单点修）
// ---------------------------------------------------------------------------

/** PDF 用户空间点（pt）。 */
type PdfPoint = { x: number; y: number };
/** 高亮四边形（环序：p0→p1 为文本行方向边）。 */
type QuadJson = { points: [PdfPoint, PdfPoint, PdfPoint, PdfPoint] };
type RgbJson = { r: number; g: number; b: number };
/** 轴对齐矩形（x0<=x1 且 y0<=y1 由会话构造校验）。 */
type RectJson = { x0: number; y0: number; x1: number; y1: number };

/** AnnotationKind serde（externally-tagged 载荷；#45：Ink 增 color/width
 * ——wasm 面旧 JSON 无两字段时引擎侧 serde 缺省）。 */
type AnnotKindJson =
  | { Highlight: { quads: QuadJson[]; color: RgbJson } }
  | { Note: { rect: RectJson } }
  | { FreeText: { rect: RectJson; text: string } }
  | { Ink: { strokes: PdfPoint[][]; color: RgbJson; width: number } };

/** Annotation serde（document.rs / annotation.rs）。 */
type AnnotationJson = {
  id: number;
  page: number;
  kind: AnnotKindJson;
  author: string;
  origin: 'Human' | 'Ai';
  created_ms: number;
  modified_ms: number;
};

/** AnnotateList 过滤（合取；缺省维度不过滤）。 */
type AnnotFilterJson = {
  kinds?: string[];
  page?: number;
  origin?: 'Human' | 'Ai';
};

/** SearchHit（command.rs——页 + 摘录，无几何矩形）。 */
type SearchHitJson = { page: number; snippet: string };

/** Outcome serde（externally-tagged）。 */
type OutcomeJson =
  | { ExtractedText: { pages: [number, string][] } }
  | { SearchResults: { hits: SearchHitJson[]; truncated: boolean } }
  | { Outline: unknown }
  | { AnnotAdded: AnnotationJson }
  | { AnnotEdited: AnnotationJson }
  | { AnnotListed: AnnotationJson[] };

/** OpenAssessment（security.rs）。 */
type AssessmentJson = {
  encrypted: boolean;
  warnings: { family: string; detail: string }[];
  form_probe?: { field_count: number; fields: unknown[] };
};

type PdfOk<T> = { ok: true; value: T };
type PdfFail = { ok: false; code: string; message: string };

const BRIDGE_ABSENT: PdfFail = {
  ok: false,
  code: 'bridge_absent',
  message: 'WasmPdfSession not present in pkg_pdf (pending mdc-pdf-wasm build)',
};

// ---------------------------------------------------------------------------
// cmd/ctor JSON 构造（单一出口——形态漂移单点修 + 运行时形态自检）
// ---------------------------------------------------------------------------
//
// 形态依据（crates/pdf serde 实读，非任务书近似）：
//   DocumentMeta = {"id":<u64>,"file_name":"…","page_count":<u32>}
//     （PdfDocumentId/PageCount newtype 透明序列化 → 裸数值）
//   PageGeometry = {"width_pt":<f32>,"height_pt":<f32>,"rotation":"Deg0"|…}
//   OutlineTree  = {"roots":[…]}（v0 空）
//   Command（externally-tagged）：
//     {"ExtractText":{"target":{"Pages":{"start":N,"end":M}},"source":"view"}}
//     {"ExtractText":{"target":"All","source":"view"}}
//     {"Search":{"query":"…","opts":{case_insensitive,max_hits},"source":"view"}}
//     {"Outline":{"source":"view"}}
//     {"AnnotateAdd":{"draft":{page,kind,author,origin},"source":"view"}}
//     {"AnnotateEdit":{"id":N,"patch":{…},"source":"view"}}
//     {"AnnotateList":{"filter":{…},"source":"view"}}
//   Source（snake_case）= "view" | "undo" | "redo" | "ai" | "system"

const OP_SOURCE = 'view'; // 命令源（镜像 history::Source——人手交互）
const OP_AUTHOR = 'human:web'; // /T author（Human 面——工作台 HUMAN_OP 同款）

function buildMetaJson(docId: number, fileName: string, pageCount: number): string {
  return JSON.stringify({ id: docId, file_name: fileName, page_count: pageCount });
}

function buildPagesJson(
  pages: { widthPt: number; heightPt: number; rotation: string }[],
): string {
  return JSON.stringify(
    pages.map((p) => ({
      width_pt: p.widthPt,
      height_pt: p.heightPt,
      rotation: p.rotation,
    })),
  );
}

// （#463 后：大纲 JSON 注入面迁至 pdf_outline.ts outlineTreeJson——
// 　原恒空树函数移除，会话 ctor 与树 UI 同一数据源。）

function buildCmdExtractTextAll(): string {
  return JSON.stringify({ ExtractText: { target: 'All', source: OP_SOURCE } });
}

function buildCmdSearch(query: string, caseInsensitive: boolean, maxHits: number): string {
  return JSON.stringify({
    Search: {
      query,
      opts: { case_insensitive: caseInsensitive, max_hits: maxHits },
      source: OP_SOURCE,
    },
  });
}

function buildCmdAnnotateAdd(
  page: number,
  kind: AnnotKindJson,
  author = OP_AUTHOR,
  origin: 'Human' | 'Ai' = 'Human',
): string {
  return JSON.stringify({
    AnnotateAdd: { draft: { page, kind, author, origin }, source: OP_SOURCE },
  });
}

function buildCmdAnnotateList(filter: AnnotFilterJson = {}): string {
  return JSON.stringify({ AnnotateList: { filter, source: OP_SOURCE } });
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/** 抛出的 wasm JSON 串 → typed 失败（永不抛出）。 */
function wasmFail(e: unknown): PdfFail {
  const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
  try {
    const parsed = JSON.parse(raw) as { code?: string; message?: string };
    if (parsed && typeof parsed.code === 'string') {
      return { ok: false, code: parsed.code, message: parsed.message ?? raw };
    }
  } catch {
    // 非 JSON 错误面（如 serde 文本）——原样携带
  }
  return { ok: false, code: 'unknown', message: raw || String(e) };
}

/** Uint8Array → base64（分块展开——无调用栈溢出）。 */
function bytesToBase64(bytes: Uint8Array): string {
  let out = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    out += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(out);
}

/** base64 → 新 Uint8Array。 */
function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/** serde 形态漂移自检（execute/ctor 失败消息含 serde 面关键词 → 追加提示）。 */
const SERDE_HINT_RE =
  /unknown variant|unknown field|missing field|invalid type|expected|serde/i;
function withShapeHint(fail: PdfFail): PdfFail {
  if (SERDE_HINT_RE.test(fail.message)) {
    return {
      ...fail,
      message: `${fail.message} [JSON shape mismatch — check buildCmd*/build*Json in pdf_view.ts]`,
    };
  }
  return fail;
}

function downloadBytes(bytes: Uint8Array, mime: string, filename: string): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

// ---------------------------------------------------------------------------
// 页缓存（TS 侧，语义镜像 mdc-pdf PageCache：字节预算 LRU）
// ---------------------------------------------------------------------------

const CACHE_BUDGET_BYTES = 64 * 1024 * 1024; // 64MB

class PageBitmapCache {
  private readonly entries = new Map<string, { bitmap: ImageBitmap; bytes: number }>();
  private totalBytes = 0;

  has(key: string): boolean {
    return this.entries.has(key);
  }

  /** LRU 取用（命中即重排到最新端）。 */
  peek(key: string): ImageBitmap | null {
    const entry = this.entries.get(key);
    if (!entry) return null;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.bitmap;
  }

  put(key: string, bitmap: ImageBitmap): void {
    if (this.entries.has(key)) return; // 不重复入账
    const bytes = bitmap.width * bitmap.height * 4;
    this.entries.set(key, { bitmap, bytes });
    this.totalBytes += bytes;
    this.evict();
  }

  /** 预算驱逐（最旧端起；驱逐即 bitmap.close() 释放）。单条目超预算
   * 不滞留（审查 S-2：原 `size > 1` 守卫使孤立超预算位图永久驻留——
   * 超限且仅剩一条时也驱逐：位图释放，活页显示不受影响[canvas 面]）。 */
  private evict(): void {
    while (this.totalBytes > CACHE_BUDGET_BYTES && this.entries.size > 0) {
      const oldest = this.entries.keys().next().value as string | undefined;
      if (oldest === undefined) break;
      const entry = this.entries.get(oldest);
      if (entry) {
        this.totalBytes -= entry.bytes;
        entry.bitmap.close();
      }
      this.entries.delete(oldest);
    }
  }

  clear(): void {
    for (const entry of this.entries.values()) entry.bitmap.close();
    this.entries.clear();
    this.totalBytes = 0;
  }
}

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

const SCALE_MIN = 0.25;
const SCALE_MAX = 4;
const SCALE_STEP = 1.25; // 倍率步进（×1.25 / ÷1.25）
const PAGE_MARGIN = 24; // fit 宽度基准留白 px
const CLEAR_MARGIN_FACTOR = 1.5; // 远页 canvas 释放距离（× 视口高）
const MAX_SEARCH_HITS = 500; // 请求值（会话侧 S3 限额 1000 钳制）
const IO_ROOT_MARGIN = '100% 0px'; // 懒渲染窗口（视口 ±1 视口高 ≈ ±1 页）
const HIGHLIGHT_COLOR: RgbJson = { r: 255, g: 214, b: 10 };
const NOTE_SIZE_PT = 22; // 便签锚定矩形边长（pt）
const DEFAULT_ASSESSMENT: AssessmentJson = { encrypted: false, warnings: [] };

type AnnotMode = '' | 'highlight' | 'note' | 'freetext' | 'ink';

// ---------------------------------------------------------------------------
// window 探针面（测试/宿主）
// ---------------------------------------------------------------------------

declare global {
  interface Window {
    /** PDF 面板启动旗标（pdfjs + pkg_pdf 均已解析 + DOM 就绪）。 */
    __pdfReady: boolean;
    __pdfState: () => {
      open: boolean;
      /** pkg_pdf 的 WasmPdfSession ctor 在场（false = e2e SKIP 门）。 */
      wasm: boolean;
      pages: number;
      scale: number;
      encrypted: boolean;
      warnings: string[];
      annots: number;
    };
    /** 第 n 页（1 基）渲染状态。 */
    __pdfPage: (n: number) => {
      rendered: boolean;
      cached: boolean;
      lastSource: 'render' | 'cache' | 'none';
    };
    /** 会话 Search 透传（语义命中权威；显示层异步刷新）。 */
    __pdfSearch: (
      q: string,
    ) => Promise<PdfOk<{ hits: SearchHitJson[]; truncated: boolean }> | PdfFail>;
    /** 会话 ExtractText 透传（全档页文本——v0 注入面回读验证）。 */
    __pdfExtractText: () => Promise<PdfOk<[number, string][]> | PdfFail>;
    /** AnnotateList 透传（可选 filter JSON）。 */
    __pdfAnnots: (filterJson?: string) => AnnotationJson[] | PdfFail;
    /** 程序化加高亮（测试面；rect = 用户空间 {x,y,w,h}，y 自页底）。 */
    __pdfAddHighlight: (
      page: number,
      rect: { x: number; y: number; w: number; h: number },
    ) => PdfOk<{ id: number; annotation: AnnotationJson }> | PdfFail;
    /** #45：程序化加墨迹（测试面；载荷 = UI 选择器状态同源构造）。 */
    __pdfAddInk: (
      page: number,
      strokes: PdfPoint[][],
    ) => PdfOk<{ id: number; annotation: AnnotationJson }> | PdfFail;
    /** flush 透传（不下载）+ 产物字节。 */
    __pdfFlush: () =>
      | { ok: true; bytes_appended: number; prefix_bytes: number; bytes: string; len: number }
      | PdfFail;
    /** 测试字节注入（base64）——open 全流程（扫描→显示面→会话）。 */
    __pdfOpenBytes: (b64: string) => Promise<PdfOk<{ pages: number }> | PdfFail>;
    __pdfScale: () => number;
    /** 当前 textLayer 选中文本（原生选区透传）。 */
    __pdfSelection: () => string;
    /** evidence-jump 钩子记录（后续阶段接真数据——本阶段钩子面）。 */
    __pdfJumpLog: () => number[];
    /** #240 层③：打印态面观测（CSS 判据 + 渲染完备度——e2e 用）。 */
    __pdfPrintInfo: () => {
      pdfActive: boolean;
      pages: number;
      rendered: number;
    };
    /** #240 层③：打印准备（全页渲染——打印按钮同一路径；返回渲染
     * 完成数，不调 window.print()——测试面/宿主可单独驱动）。 */
    __pdfPrintPrepare: () => Promise<number>;
    /** #475：注释列表面观测（行数/页序/开态——e2e 断言面）。 */
    __pdfAnnotsUi: () => {
      count: number;
      ids: number[];
      pages: number[];
      drawerOpen: boolean;
    };
    /** #466：链接面观测（逐页内/外链计数 + 策略拒绝计数——e2e 断言面）。 */
    __pdfLinks: () => {
      perPage: { total: number; internal: number; external: number }[];
      policySkipped: string[];
    };

    /** #472：文档属性面观测（DocProps 镜像 + 抽屉开态——e2e 断言面）。 */
    __pdfProps: () => {
      open: boolean;
      props: {
        title: string | null;
        author: string | null;
        subject: string | null;
        keywords: string | null;
        producer: string | null;
        creator: string | null;
        creationDate: string | null;
        modDate: string | null;
        pdfVersion: string | null;
        fileName: string;
        pageCount: number;
        fileSize: number;
        encrypted: boolean;
        warnings: number;
      } | null;
      drawerOpen: boolean;
    };


    /** #463：大纲面观测（树在场性/先序平铺计数与标题/跳过数/抽屉开态）。 */
    __pdfOutline: () => {
      present: boolean;
      count: number;
      titles: string[];
      skipped: number;
      drawerOpen: boolean;
    };
    /** #463：会话 Outline 命令透传（模型面验证——ctor 注入的同源树）。 */
    __pdfOutlineSession: () =>
      | PdfOk<{ count: number; titles: string[] }>
      | PdfFail;
  }
}

// ---------------------------------------------------------------------------
// PdfViewUI
// ---------------------------------------------------------------------------

interface PageState {
  el: HTMLElement;
  canvas: HTMLCanvasElement;
  textLayer: HTMLDivElement;
  overlay: HTMLDivElement;
  annotLayer: HTMLDivElement;
  searchLayer: HTMLDivElement;
  evidenceLayer: HTMLDivElement;
  /** #466：链接热区层（overlay 末位 = 点击优先级最高）。 */
  linkLayer: HTMLDivElement;
  proxy: PDFPageProxy;
  viewport: PageViewport; // 当前 scale 的 CSS px 视口
  rendered: boolean;
  lastSource: 'render' | 'cache' | 'none';
  renderToken: number;
  renderTask: RenderTask | null;
  textLayerActive: { cancel: () => void } | null;
}

/** #466：链接热区（内链 target = 0 基页号；外链 url = scheme 白名单后串）。 */
interface PageLinkBox {
  target: number | null;
  url: string | null;
  rect: { x0: number; y0: number; x1: number; y1: number };
}

/** #466：pdf.js 注释面的最小结构鸭子（不依赖 pdfjs-dist 精确类型面）。 */
interface PdfAnnotLike {
  subtype?: string;
  rect?: number[];
  dest?: unknown;
  url?: string;
  unsafeUrl?: string;
  action?: string;
}

/** #466：外链 scheme 白名单（http/https/mailto——阅读器惯例；
 * javascript:/data:/file: 等一律不渲染，策略拒绝面入探针计数不静默）。 */
const URL_SCHEME_ALLOW = new Set(['http:', 'https:', 'mailto:']);

function allowExternalUrl(raw: string): string | null {
  try {
    // 仅绝对 URL（相对串在 PDF 注释上下文无基准——拒绝并计数）
    const u = new URL(raw);
    return URL_SCHEME_ALLOW.has(u.protocol) ? u.href : null;
  } catch {
    return null;
  }
}

export class PdfViewUI {
  private readonly pdfjs: PdfjsModule;
  private readonly sessionCtor: WasmPdfSessionCtor | null;
  private readonly scanFn: ((bytes: Uint8Array) => string) | null;

  // 会话状态
  private doc: PDFDocumentProxy | null = null;
  private loadingTask: { destroy(): Promise<void> } | null = null;
  private session: WasmPdfSessionFace | null = null;
  private originalBytes: Uint8Array | null = null;
  private writerAttached = false;
  private assessment: AssessmentJson = DEFAULT_ASSESSMENT;
  private annotations: AnnotationJson[] = [];
  /** Note 便签悬停文本（宿主侧显示面——v0 会话 Note 无文本载荷，PDF
   * /Contents 落盘映射随 PDF-d 写映射定稿；此表仅本会话显示用）。 */
  private readonly noteText = new Map<number, string>();
  /** 会话构造失败消息（open 尾部 applySecurityStatus 不吞错误面）。 */
  private sessionInitFail: PdfFail | null = null;
  /** v0 文本注入进行标记（Search 前等待其收敛）。 */
  private textInjected: Promise<number> | null = null;
  private readonly tcCache = new Map<number, PdfTextContent>();

  // 布局/渲染状态
  private readonly pages: PageState[] = [];
  private scale = 1;
  private readonly cache = new PageBitmapCache();
  private io: IntersectionObserver | null = null;
  private rafPending = false;
  private scrollRafPending = false;
  /** 单页渲染串行链（pdf.js 同 canvas 并发 render 拒绝——排队避免）。 */
  private readonly renderChain = new Map<number, Promise<void>>();
  /** 打开时面板隐藏 → fit 宽度挂起（tab 显示时补算）。 */
  private defaultFitPending = false;

  // 搜索状态
  private searchQuery = '';
  private searchHits: SearchHitJson[] = [];
  private searchIndex = -1;

  // 标注交互状态
  private mode: AnnotMode = '';
  private inkStroke: PdfPoint[] = [];
  private inkPreview: SVGSVGElement | null = null;
  private inkPage = -1;
  /** #45：墨迹视觉态（UI 选择器 → 预览 + 载荷同源；缺省 = 旧硬编码
   * 蓝 #1a73e8 / 2pt——引擎 serde 缺省同值，旧会话往返视觉零漂移）。 */
  private inkColor = '#1a73e8';
  private inkWidth = 2;
  private inkColorSelect: HTMLSelectElement | null = null;
  private inkWidthSelect: HTMLSelectElement | null = null;
  private freetextInput: HTMLInputElement | null = null;

  // 链接状态（#466——显示面数据源 pdf.js getAnnotations；内链页号解析
  // 复用 dest 解析语义；策略拒绝面计数不静默）
  private pageLinks: PageLinkBox[][] = [];
  private linkPolicySkipped: string[] = [];

  // 证据区域叠加（钩子面——后续阶段接真数据）
  private readonly evidenceRegions: { page: number; rect: RectJson }[] = [];
  private readonly jumpLog: number[] = [];

  // 注释列表面状态（#475——数据源 = this.annotations 会话真源）
  private annotsBtn: HTMLButtonElement | null = null;
  private annotsDrawer: HTMLDivElement | null = null;
  private annotsBody: HTMLDivElement | null = null;
  private annotsEmptyEl: HTMLDivElement | null = null;
  // 文档属性状态（#472——显示面数据源 pdf.js getMetadata + 会话/评估汇总）
  private docProps: DocProps | null = null;
  private propsBtn: HTMLButtonElement | null = null;
  private propsDrawer: HTMLDivElement | null = null;
  private propsBody: HTMLDivElement | null = null;

  // 大纲状态（#463——显示面数据源 pdf.js，会话 ctor 注入同一棵树）
  private outlineTree: OutlineNodeJson[] = [];
  private outlineSkipped = 0;
  private outlineBtn: HTMLButtonElement | null = null;
  private outlineDrawer: HTMLDivElement | null = null;
  private outlineBody: HTMLDivElement | null = null;
  private outlineEmptyEl: HTMLDivElement | null = null;

  // DOM 句柄
  private panel!: HTMLElement;
  private viewportEl!: HTMLDivElement;
  private contentEl!: HTMLElement;
  /** #475：工具栏句柄（抽屉 top 动态锚——wrap 到两行时不叠盖）。 */
  /** #472：工具栏句柄（抽屉 top 动态锚——wrap 到两行时不叠盖）。 */
  private toolbarEl!: HTMLDivElement;
  /** #3 UI 批 E：空态引导卡片层（文档在场 = hidden）。 */
  private guideEl!: HTMLDivElement;
  private pageLabel!: HTMLElement;
  private zoomLabel!: HTMLElement;
  private searchInput!: HTMLInputElement;
  private searchCount!: HTMLElement;
  private statusEl!: HTMLElement;
  private fileInput!: HTMLInputElement;
  private annotModeSelect!: HTMLSelectElement;
  private saveBtn!: HTMLButtonElement;

  constructor(pdfjs: PdfjsModule, pdfWasm: PdfWasmModule | null) {
    this.pdfjs = pdfjs;
    this.sessionCtor = pdfWasm?.WasmPdfSession ?? null;
    this.scanFn = pdfWasm?.pdf_open_scan ?? null;
    this.buildDom();
    this.installHandlers();
    this.setupTabs();
    this.installProbes();
  }

  // -----------------------------------------------------------------------
  // DOM 构建（index.html 只提供 #pdf-panel 壳 + tab 按钮）
  // -----------------------------------------------------------------------

  private buildDom(): void {
    const panel = document.getElementById('pdf-panel');
    if (!panel) return; // 无壳页（如 bench）——惰性
    panel.replaceChildren();

    const toolbar = document.createElement('div');
    toolbar.id = 'pdf-toolbar';

    const openBtn = document.createElement('label');
    // #43 问题 4：打开入口强化（pdf-open-btn = accent 边框 + 加重字重；
    // label-for 关联不动——DOM 契约）。
    openBtn.className = 'pdf-btn pdf-open-btn';
    openBtn.textContent = t('pdf.open');
    openBtn.htmlFor = 'pdf-file';
    openBtn.title = t('pdf.openTitle');

    const fileInput = document.createElement('input');
    fileInput.type = 'file';
    fileInput.id = 'pdf-file';
    fileInput.hidden = true;
    fileInput.accept = '.pdf,application/pdf';

    const pageLabel = document.createElement('span');
    pageLabel.id = 'pdf-page-label';
    pageLabel.textContent = t('pdf.noPages');

    const mkBtn = (id: string, label: string, title = ''): HTMLButtonElement => {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'pdf-btn';
      b.id = id;
      b.textContent = label;
      if (title) b.title = title;
      return b;
    };
    const zoomOut = mkBtn('pdf-zoom-out', '−', t('pdf.zoomOutTitle'));
    const zoomIn = mkBtn('pdf-zoom-in', '+', t('pdf.zoomInTitle'));
    const zoomFit = mkBtn('pdf-zoom-fit', t('pdf.fitWidth'));
    const searchPrev = mkBtn('pdf-search-prev', '↑', t('pdf.prevHitTitle'));
    const searchNext = mkBtn('pdf-search-next', '↓', t('pdf.nextHitTitle'));

    const zoomLabel = document.createElement('span');
    zoomLabel.id = 'pdf-zoom-label';
    zoomLabel.textContent = '100%';

    const searchInput = document.createElement('input');
    searchInput.type = 'search';
    searchInput.id = 'pdf-search';
    searchInput.placeholder = t('pdf.searchPlaceholder');
    searchInput.autocomplete = 'off';

    const searchCount = document.createElement('span');
    searchCount.id = 'pdf-search-count';
    searchCount.textContent = '';

    const modeSelect = document.createElement('select');
    modeSelect.id = 'pdf-annot-mode';
    for (const [value, label] of [
      ['', t('pdf.modeOff')],
      ['highlight', t('pdf.modeHighlight')],
      ['note', t('pdf.modeNote')],
      ['freetext', t('pdf.modeFreetext')],
      ['ink', t('pdf.modeInk')],
    ] as const) {
      const opt = document.createElement('option');
      opt.value = value;
      opt.textContent = label;
      modeSelect.appendChild(opt);
    }

    // #45：墨迹颜色/粗细选择器（六色预设 + 三档粗细——预设与 UI 状态
    // 同源驱动预览与载荷；id 面 = pdf-ink-color / pdf-ink-width）。
    const inkColor = document.createElement('select');
    inkColor.id = 'pdf-ink-color';
    inkColor.title = t('pdf.inkColorTitle');
    for (const [value, key] of [
      ['#ff0000', 'pdf.colorRed'],
      ['#ffd400', 'pdf.colorYellow'],
      ['#30a46c', 'pdf.colorGreen'],
      ['#1a73e8', 'pdf.colorBlue'],
      ['#8e4ec6', 'pdf.colorPurple'],
      ['#111111', 'pdf.colorBlack'],
    ] as const) {
      const o = document.createElement('option');
      o.value = value;
      o.textContent = t(key);
      if (value === this.inkColor) o.selected = true;
      inkColor.appendChild(o);
    }
    inkColor.addEventListener('change', () => {
      this.inkColor = inkColor.value;
    });

    const inkWidth = document.createElement('select');
    inkWidth.id = 'pdf-ink-width';
    inkWidth.title = t('pdf.inkWidthTitle');
    for (const [value, key] of [
      ['1', 'pdf.inkThin'],
      ['2', 'pdf.inkMedium'],
      ['4', 'pdf.inkThick'],
    ] as const) {
      const o = document.createElement('option');
      o.value = value;
      o.textContent = t(key);
      if (Number(value) === this.inkWidth) o.selected = true;
      inkWidth.appendChild(o);
    }
    inkWidth.addEventListener('change', () => {
      this.inkWidth = Number(inkWidth.value);
    });

    const saveBtn = document.createElement('button');
    saveBtn.type = 'button';
    saveBtn.className = 'pdf-btn';
    saveBtn.id = 'pdf-save-btn';
    saveBtn.textContent = t('pdf.saveAnnotated');

    // #240 层③：打印按钮（全页渲染 → window.print()——懒渲染面向
    // 屏幕，打印需要每页位图在场；与探针 __pdfPrintPrepare 同一路径）。
    const printBtn = mkBtn('pdf-print-btn', t('pdf.print'), t('pdf.printTitle'));

    // #475：注释列表按钮（文档开 = 使能；零批注 = 空态文案）。
    const annotsBtn = mkBtn('pdf-annots-btn', t('pdf.annots'), t('pdf.annotsTitle'));
    // #472(长尾⑦)：文档属性按钮（无文档态禁用——syncPropsPanel 维护）。
    const propsBtn = mkBtn('pdf-props-btn', t('pdf.props'), t('pdf.propsTitle'));
    propsBtn.disabled = true;

    // #463（长尾④）：大纲切换按钮（无大纲档禁用——syncOutlinePanel 维护）。
    const outlineBtn = mkBtn('pdf-outline-btn', t('pdf.outline'), t('pdf.outlineTitle'));
    outlineBtn.disabled = true;

    toolbar.append(
      openBtn, fileInput, pageLabel,
      zoomOut, zoomLabel, zoomIn, zoomFit, outlineBtn,
      searchInput, searchPrev, searchNext, searchCount,
      modeSelect, inkColor, inkWidth, saveBtn, printBtn, annotsBtn,
      modeSelect, inkColor, inkWidth, saveBtn, printBtn, propsBtn,
    );

    const viewport = document.createElement('div');
    viewport.id = 'pdf-viewport';
    viewport.tabIndex = 0;

    const content = document.createElement('div');
    content.id = 'pdf-content';
    viewport.appendChild(content);

    const status = document.createElement('div');
    status.id = 'pdf-status';
    status.textContent =
      'open a .pdf to start (display: pdf.js · model: mdc-pdf wasm session)';

    // #475：注释列表抽屉（面板内浮层;top 动态随工具栏高度;打印态不进纸面）。
    const annotsDrawer = document.createElement('div');
    annotsDrawer.id = 'pdf-annots-drawer';
    annotsDrawer.hidden = true;
    const annotsHead = document.createElement('div');
    annotsHead.className = 'pdf-annots-head';
    annotsHead.textContent = t('pdf.annots');
    const annotsBody = document.createElement('div');
    annotsBody.className = 'pdf-annots-body';
    const annotsEmpty = document.createElement('div');
    annotsEmpty.className = 'pdf-annots-empty';
    annotsEmpty.textContent = t('pdf.annotsEmpty');
    annotsEmpty.hidden = true;
    annotsDrawer.append(annotsHead, annotsBody, annotsEmpty);

    panel.append(toolbar, viewport, status, annotsDrawer);
    // #472：属性抽屉（面板内浮层——绝对定位；打印态不进纸面）。
    const propsDrawer = document.createElement('div');
    propsDrawer.id = 'pdf-props-drawer';
    propsDrawer.hidden = true;
    const propsHead = document.createElement('div');
    propsHead.className = 'pdf-props-head';
    propsHead.textContent = t('pdf.props');
    const propsBody = document.createElement('div');
    propsBody.className = 'pdf-props-body';
    propsDrawer.append(propsHead, propsBody);

    panel.append(toolbar, viewport, status, propsDrawer);

    // #463：大纲抽屉（面板内绝对定位——#pdf-panel 需 position:relative；
    // 打印态 CSS 隐藏，不进纸面）。
    const drawer = document.createElement('div');
    drawer.id = 'pdf-outline-drawer';
    drawer.hidden = true;
    const drawerHead = document.createElement('div');
    drawerHead.className = 'pdf-outline-head';
    drawerHead.textContent = t('pdf.outline');
    const drawerBody = document.createElement('div');
    drawerBody.className = 'pdf-outline-body';
    const drawerEmpty = document.createElement('div');
    drawerEmpty.className = 'pdf-outline-empty';
    drawerEmpty.textContent = t('pdf.outlineEmpty');
    drawerEmpty.hidden = true;
    drawer.append(drawerHead, drawerBody, drawerEmpty);

    panel.append(toolbar, viewport, status, drawer);

    this.panel = panel;
    this.viewportEl = viewport;
    this.contentEl = content;
    this.toolbarEl = toolbar;
    // #3 UI 批 E：空态引导卡片（文档打开成功前——纯色块改引导）。
    this.guideEl = mount_surface_guide(viewport, 'pdf-guide', {
      icon: 'M7 2.5h7.5L19.5 7v14.5h-13V2.5z M14 2.5V7h5 M9.5 12.5h5 M9.5 16h5',
      title: t('pdf.guideTitle'),
      hints: [
        t('pdf.guideHint1'),
        t('pdf.guideHint2'),
      ],
    });
    this.pageLabel = pageLabel;
    this.zoomLabel = zoomLabel;
    this.searchInput = searchInput;
    this.searchCount = searchCount;
    this.statusEl = status;
    this.fileInput = fileInput;
    this.annotModeSelect = modeSelect;
    this.inkColorSelect = inkColor;
    this.inkWidthSelect = inkWidth;
    this.saveBtn = saveBtn;
    this.annotsBtn = annotsBtn;
    this.annotsDrawer = annotsDrawer;
    this.annotsBody = annotsBody;
    this.annotsEmptyEl = annotsEmpty;
    this.propsBtn = propsBtn;
    this.propsDrawer = propsDrawer;
    this.propsBody = propsBody;


    this.outlineBtn = outlineBtn;
    this.outlineDrawer = drawer;
    this.outlineBody = drawerBody;
    this.outlineEmptyEl = drawerEmpty;
    this.syncButtons();
  }

  // -----------------------------------------------------------------------
  // 事件接线
  // -----------------------------------------------------------------------

  private installHandlers(): void {
    if (!this.panel) return;

    this.fileInput.addEventListener('change', async (): Promise<void> => {
      const f = this.fileInput.files?.[0];
      this.fileInput.value = ''; // 允许重选同名文件
      if (!f) return;
      const bytes = new Uint8Array(await f.arrayBuffer());
      void this.openBytes(bytes, f.name);
    });

    document.getElementById('pdf-zoom-out')?.addEventListener('click', () =>
      this.setScale(this.scale / SCALE_STEP));
    document.getElementById('pdf-zoom-in')?.addEventListener('click', () =>
      this.setScale(this.scale * SCALE_STEP));
    document.getElementById('pdf-zoom-fit')?.addEventListener('click', () =>
      this.setScale(this.fitScale()));

    this.searchInput.addEventListener('input', () => {
      void this.searchOnce(this.searchInput.value);
    });
    document.getElementById('pdf-search-prev')?.addEventListener('click', () =>
      this.navigateHit(-1));
    document.getElementById('pdf-search-next')?.addEventListener('click', () =>
      this.navigateHit(1));

    this.annotModeSelect.addEventListener('change', () => {
      this.setMode(this.annotModeSelect.value as AnnotMode);
    });

    this.saveBtn.addEventListener('click', () => {
      void this.save();
    });

    // #240 层③：打印按钮（全页渲染 → window.print()）。
    document.getElementById('pdf-print-btn')?.addEventListener('click', () => {
      void this.printDocument();
    });

    // #475：注释抽屉开合（文档开 = 可用;开合 = 用户选择）。
    this.annotsBtn?.addEventListener('click', () => {
      if (!this.annotsDrawer) return;
      this.annotsDrawer.hidden = !this.annotsDrawer.hidden;
    });

    // #472：属性抽屉开合（无文档态禁用；重开新档 = 保持用户开合选择）。
    this.propsBtn?.addEventListener('click', () => {
      if (!this.propsDrawer) return;
      this.propsDrawer.hidden = !this.propsDrawer.hidden;
    });


    // #463：大纲抽屉开合（按钮态由 syncOutlinePanel 维护——禁用 = 无大纲）。
    this.outlineBtn?.addEventListener('click', () => {
      if (!this.outlineDrawer) return;
      this.outlineDrawer.hidden = !this.outlineDrawer.hidden;
    });
    // #240 层③：Ctrl+P 路径（beforeprint）——best-effort 启动全页渲染
    // （无法 await：浏览器同步进入预览快照——未完成的页可能空白；
    // 打印按钮路径无此限制，诚实登记见模块文档）。afterprint 恢复
    // 懒渲染纪律（远页位图重释放——屏幕内存面回归）。
    window.addEventListener('beforeprint', () => {
      if (document.body.dataset.pdfActive !== undefined && this.doc) {
        void this.renderAllPages();
      }
    });
    window.addEventListener('afterprint', () => {
      if (document.body.dataset.pdfActive !== undefined) {
        this.scheduleRenderVisible();
        this.clearFarCanvases(); // 恢复「只保视口附近位图」的内存纪律
      }
    });

    // 滚动：页码跟踪 + 远页释放（rAF 节流）
    this.viewportEl.addEventListener('scroll', () => {
      if (this.scrollRafPending) return;
      this.scrollRafPending = true;
      requestAnimationFrame(() => {
        this.scrollRafPending = false;
        this.updatePageLabel();
        this.clearFarCanvases();
      });
    });
    window.addEventListener('resize', () => this.scheduleRenderVisible());

    // 高亮模式：textLayer 选区 → mouseup 提交（选区完成信号）
    this.viewportEl.addEventListener('mouseup', () => {
      if (this.mode === 'highlight') void this.commitSelectionHighlight();
    });

    // Note/FreeText：页内点击落点（这两模式下 overlay 仍透传——点击落
    // 到 textLayer/页元素后冒泡到此处）。
    this.viewportEl.addEventListener('click', (e) => {
      if (this.mode !== 'note' && this.mode !== 'freetext') return;
      if (this.freetextInput) return;
      const target = e.target instanceof HTMLElement ? e.target : null;
      const pageEl = target?.closest<HTMLElement>('.pdf-page');
      if (!pageEl) return;
      const idx = Number(pageEl.dataset.page);
      const st = this.pages[idx];
      if (!st) return;
      const rect = pageEl.getBoundingClientRect();
      const localX = e.clientX - rect.left;
      const localY = e.clientY - rect.top;
      if (this.mode === 'note') void this.addNoteAt(idx, localX, localY);
      else this.beginFreeText(idx, localX, localY);
    });

    // 墨迹：pointer 采样（overlay 在 ink 模式接管 pointer 事件，见 CSS）
    this.viewportEl.addEventListener('pointerdown', (e) => {
      if (this.mode !== 'ink') return;
      const target = e.target instanceof HTMLElement ? e.target : null;
      const pageEl = target?.closest<HTMLElement>('.pdf-page');
      if (!pageEl) return;
      const idx = Number(pageEl.dataset.page);
      const st = this.pages[idx];
      if (!st) return;
      e.preventDefault();
      const rect = pageEl.getBoundingClientRect();
      const [px, py] = st.viewport.convertToPdfPoint(
        e.clientX - rect.left,
        e.clientY - rect.top,
      );
      this.inkStroke = [{ x: px, y: py }];
      this.inkPage = idx;
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.classList.add('pdf-ink-preview');
      st.overlay.appendChild(svg);
      this.inkPreview = svg;
      const move = (ev: PointerEvent): void => {
        if (this.mode !== 'ink' || this.inkPage < 0) return;
        const st2 = this.pages[this.inkPage];
        const r2 = st2.el.getBoundingClientRect();
        const [mx, my] = st2.viewport.convertToPdfPoint(
          ev.clientX - r2.left,
          ev.clientY - r2.top,
        );
        this.inkStroke.push({ x: mx, y: my });
        this.drawInkPreview(st2);
      };
      const up = (): void => {
        window.removeEventListener('pointermove', move);
        window.removeEventListener('pointerup', up);
        this.finishInkStroke();
      };
      window.addEventListener('pointermove', move);
      window.addEventListener('pointerup', up);
    });
  }

  /** 视图 tab（doc/grid 既有逻辑不动——本模块只追加 pdf 面显隐）。
   * #240 层③：showPdf/hidePdf 同时维护 body[data-pdf-active]——
   * 打印 CSS 的单一判据（打印态打印 PDF 页而非藏面板）。 */
  private setupTabs(): void {
    const pdfTab = document.getElementById('tab-pdf');
    const docTab = document.getElementById('tab-doc');
    const gridTab = document.getElementById('tab-grid');
    const editor = document.getElementById('editor');
    const gridPanel = document.getElementById('grid-panel');
    if (!pdfTab || !docTab || !gridTab || !editor || !gridPanel || !this.panel) return;
    const showPdf = (): void => {
      docTab.classList.remove('active');
      gridTab.classList.remove('active');
      pdfTab.classList.add('active');
      editor.style.display = 'none';
      gridPanel.style.display = 'none';
      this.panel.style.display = 'block';
      document.body.dataset.pdfActive = '1'; // #240 层③：打印态判据
      if (this.defaultFitPending) {
        // 打开时面板隐藏（clientWidth 0）→ fit 宽度挂起，此时补算
        this.defaultFitPending = false;
        this.setScale(this.fitScale(), { skipAnchor: true });
      }
      this.scheduleRenderVisible(); // display:none 期间无渲染信号
    };
    const hidePdf = (): void => {
      pdfTab.classList.remove('active');
      this.panel.style.display = 'none';
      delete document.body.dataset.pdfActive; // #240 层③：打印态判据
    };
    pdfTab.addEventListener('click', showPdf);
    docTab.addEventListener('click', hidePdf); // 叠加监听——不动既有处理器
    gridTab.addEventListener('click', hidePdf);
  }

  // -----------------------------------------------------------------------
  // 打开/关闭
  // -----------------------------------------------------------------------

  /** 打开（评估先行 → pdf.js 显示面 → 会话模型面）。永不抛出。 */
  async openBytes(
    bytes: Uint8Array,
    fileName: string,
  ): Promise<PdfOk<{ pages: number }> | PdfFail> {
    this.teardownDocument();

    // ① 安全评估先行（pkg_pdf 在场时——lopdf 对象图扫描，只报告不阻断
    //    打开；typed 拒绝 = 畸形输入/超限）
    let assessment: AssessmentJson = DEFAULT_ASSESSMENT;
    if (this.scanFn) {
      try {
        assessment = JSON.parse(this.scanFn(bytes)) as AssessmentJson;
      } catch (e) {
        const fail = withShapeHint(wasmFail(e));
        this.showStatus(t('pdf.openRejected', { code: fail.code, message: fail.message }), 'error');
        return fail;
      }
    }

    // ② pdf.js 显示面（data 传副本——worker 可能转移所有权）
    let doc: PDFDocumentProxy;
    try {
      const task = this.pdfjs.getDocument({ data: bytes.slice() });
      this.loadingTask = task;
      doc = await task.promise;
    } catch (e) {
      const err = e as { name?: string; message?: string };
      const fail: PdfFail = {
        ok: false,
        code: err?.name === 'PasswordException' ? 'pdf_password' : 'pdf_open_failed',
        message: err?.message ?? String(e),
      };
      this.showStatus(t('pdf.openFailed', { code: fail.code, message: fail.message }), 'error');
      return fail;
    }

    this.doc = doc;
    this.originalBytes = bytes.slice();
    this.assessment = assessment;

    // ③-bis #472：文档属性读面（pdf.js getMetadata → /Info 字段；
    // 会话/评估汇总——文件名/页数/字节数/加密态/警告数）。失败 = 全空
    // /Info 面（诚实空态，不阻断打开）。
    this.docProps = null;
    try {
      const md = await doc.getMetadata();
      const info = (md?.info ?? {}) as PdfInfoLike;
      const pick = (s: string | undefined): string | null =>
        typeof s === 'string' && s.trim() ? s : null;
      this.docProps = {
        title: pick(info.Title),
        author: pick(info.Author),
        subject: pick(info.Subject),
        keywords: pick(info.Keywords),
        producer: pick(info.Producer),
        creator: pick(info.Creator),
        creationDate: pick(info.CreationDate),
        modDate: pick(info.ModDate),
        pdfVersion: pick(info.PDFFormatVersion),
        fileName,
        pageCount: doc.numPages,
        fileSize: this.originalBytes.byteLength,
        encrypted: assessment.encrypted,
        warnings: assessment.warnings.length,
      };
    } catch {
      // getMetadata 异常（无 /Info 或损坏字典）——仅丢失属性面，主流程照常
      this.docProps = {
        title: null, author: null, subject: null, keywords: null,
        producer: null, creator: null, creationDate: null, modDate: null,
        pdfVersion: null,
        fileName,
        pageCount: doc.numPages,
        fileSize: this.originalBytes.byteLength,
        encrypted: assessment.encrypted,
        warnings: assessment.warnings.length,
      };
    }

    // ③ 页几何（pdf.js 实数据 → 会话 pagesJson）
    const proxies: PDFPageProxy[] = [];
    const geoms: { widthPt: number; heightPt: number; rotationDeg: number }[] = [];
    for (let n = 1; n <= doc.numPages; n++) {
      const proxy = await doc.getPage(n);
      proxies.push(proxy);
      const vp = proxy.getViewport({ scale: 1 });
      const view = vp.viewBox;
      geoms.push({
        widthPt: view[2] - view[0],
        heightPt: view[3] - view[1],
        rotationDeg: ((vp.rotation % 360) + 360) % 360,
      });
    }

    // ③-bis #466：链接读面（pdf.js getAnnotations → Link 热区；内链
    // dest 解析为 0 基页号，外链过 scheme 白名单——策略拒绝面计数）。
    this.pageLinks = [];
    this.linkPolicySkipped = [];
    for (let n = 0; n < proxies.length; n++) {
      const links: PageLinkBox[] = [];
      try {
        const annots = (await proxies[n].getAnnotations()) as PdfAnnotLike[];
        for (const a of annots) {
          if (a.subtype !== 'Link' || !a.rect || a.rect.length < 4) continue;
          const rect = {
            x0: Math.min(a.rect[0], a.rect[2]),
            y0: Math.min(a.rect[1], a.rect[3]),
            x1: Math.max(a.rect[0], a.rect[2]),
            y1: Math.max(a.rect[1], a.rect[3]),
          };
          if (a.dest !== undefined && a.dest !== null) {
            const idx = await this.resolveLinkDest(doc, a.dest);
            if (idx !== null) {
              links.push({ target: idx, url: null, rect });
            } else {
              // 悬空命名目标 / GoToR 远程目标——pdf-render-ext §4 登记的
              // 合法 Unresolved 面：不渲染，计数不静默。
              this.linkPolicySkipped.push('unresolved-dest');
            }
            continue;
          }
          const raw = typeof a.url === 'string' && a.url
            ? a.url
            : typeof a.unsafeUrl === 'string' && a.unsafeUrl
              ? a.unsafeUrl
              : null;
          if (raw) {
            const safe = allowExternalUrl(raw);
            if (safe) {
              links.push({ target: null, url: safe, rect });
            } else {
              this.linkPolicySkipped.push(raw.split(':')[0] ?? 'bad-url');
            }
            continue;
          }
          if (a.action) {
            // /Launch 等宿主动作——web 面不执行（安全边界），计数登记
            this.linkPolicySkipped.push(`action:${a.action}`);
          }
        }
      } catch {
        // 单页注释面失败 = 该页无链接（诚实空面，不阻断打开）
      }
      this.pageLinks.push(links);
    }

    // ③-bis #463：大纲读面（pdf.js getOutline → 解析为 0 基页目标的
    // 引擎形状——会话 ctor 与树 UI 同一数据源；解析失败面 = 恒空树不阻断）。
    this.outlineTree = [];
    this.outlineSkipped = 0;
    try {
      const items = await doc.getOutline();
      const built = await buildOutlineTree(items, (dest) =>
        this.resolveOutlineDest(doc, dest),
      );
      this.outlineTree = built.tree;
      this.outlineSkipped = built.skipped;
    } catch {
      // 显示面无大纲（getOutline 异常形态）——空树，不阻断打开主流程
    }

    // ④ 会话模型面（pkg_pdf 缺位 → display-only 降级）
    this.sessionInitFail = null;
    if (this.sessionCtor) {
      try {
        const session = new this.sessionCtor(
          buildMetaJson(1, fileName, doc.numPages),
          JSON.stringify(assessment),
          buildPagesJson(
            geoms.map((g) => ({
              widthPt: g.widthPt,
              heightPt: g.heightPt,
              rotation: `Deg${g.rotationDeg}`,
            })),
          ),
          outlineTreeJson(this.outlineTree), // #463：实数据（原恒 {"roots":[]}）
        );
        session.set_clock_ms(BigInt(Date.now()));
        this.session = session;
        this.writerAttached = false;
        await this.refreshAnnotations();
        this.textInjected = this.injectPageText(proxies);
      } catch (e) {
        const fail = withShapeHint(wasmFail(e));
        this.session = null;
        this.sessionInitFail = fail;
        this.showStatus(
          t('pdf.displayOnlyFailed', { code: fail.code, message: fail.message }),
          'error',
        );
      }
    } else {
      this.showStatus(t('pdf.displayOnlyPending'), 'warn');
    }

    // ⑤ 页 DOM + 首屏（fit 宽度起步；面板隐藏时预渲染前两页）
    this.buildPages(proxies, geoms);
    const visibleNow = this.viewportEl.clientWidth > 0;
    this.setScale(visibleNow ? this.fitScale() : 1, { skipAnchor: true });
    this.defaultFitPending = !visibleNow;
    this.renderAllAnnotOverlays(); // refreshAnnotations 早于 buildPages——此处补投影
    this.renderAllLinkOverlays(); // #466：链接热区投影（open 尾一次）
    void this.renderPage(0);
    if (this.pages.length > 1) void this.renderPage(1);
    this.updatePageLabel();

    this.applySecurityStatus();
    this.syncButtons();
    this.syncGuide();
    this.syncPropsPanel(); // #472：属性面板（按钮态 + 字段表重渲染）

    this.syncOutlinePanel(); // #463：树 DOM + 按钮态 + 抽屉开合
    return { ok: true, value: { pages: doc.numPages } };
  }

  /** #475：注释列表面同步（按钮态 + 行渲染 + 抽屉 top 动态锚；数据源
   * = this.annotations 会话真源——refreshAnnotations/增删后调用）。 */
  private syncAnnotsPanel(): void {
    if (!this.annotsBtn || !this.annotsBody || !this.annotsEmptyEl) return;
    this.annotsBtn.disabled = this.doc === null;
    if (this.annotsDrawer && this.toolbarEl) {
      this.annotsDrawer.style.top = `${this.toolbarEl.offsetHeight + 8}px`;
    }
    const has = this.annotations.length > 0;
    this.annotsEmptyEl.hidden = has;
    if (this.doc) {
      renderAnnotListDom(
        this.annotsBody,
        this.annotations as AnnotRowLike[],
        (page) => this.jumpToListPage(page),
        {
          kind: {
            Highlight: t('pdf.annotKHighlight'),
            Note: t('pdf.annotKNote'),
            FreeText: t('pdf.annotKFreeText'),
            Ink: t('pdf.annotKInk'),
          },
          page: (n) => t('pdf.annotsPageN', { n }),
        },
      );
    } else {
      this.annotsBody.replaceChildren();
    }
  }

  /** #475：清单行跳页（复用大纲/链接同语义——目标页滚到视口顶）。 */
  private jumpToListPage(idx: number): void {
    const st = this.pages[idx];
    if (!st) return;
    st.el.scrollIntoView({ block: 'start', behavior: 'auto' });
    this.updatePageLabel();
  }

  /** #463：pdf.js dest（数组/命名目标）→ 0 基页号；不可解析 = null
   * （悬空命名目标/远程目标——pdf-render-ext §4 登记的合法 Unresolved 面）。 */
  private async resolveOutlineDest(
    doc: PDFDocumentProxy,
    dest: unknown,
  ): Promise<number | null> {
    try {
      let d: unknown = dest;
      if (typeof d === 'string') d = await doc.getDestination(d);
      if (!Array.isArray(d) || d.length === 0) return null;
      const idx = await doc.getPageIndex(d[0] as Parameters<
        typeof doc.getPageIndex
      >[0]);
      return idx;
    } catch {
      return null;
    }
  }

  /** #463：大纲节点跳页（树 UI 点击 → 目标页滚到视口顶 + 页码行同步）。 */
  private jumpToOutlinePage(idx: number): void {
    const st = this.pages[idx];
    if (!st) return;
    st.el.scrollIntoView({ block: 'start', behavior: 'auto' });
    this.updatePageLabel();
  }

  /** #463：大纲面板同步（树渲染 + 按钮态 + 抽屉开合——有大纲档自动展开，
   * Acrobat 同款；无大纲/拆卸态 = 抽屉隐藏 + 按钮禁用；抽屉 top 随
   * 工具栏实际高度动态锚定——wrap 两行不叠盖）。 */
  private syncOutlinePanel(): void {
    if (
      !this.outlineBtn ||
      !this.outlineDrawer ||
      !this.outlineBody ||
      !this.outlineEmptyEl
    ) {
      return;
    }
    if (this.toolbarEl) {
      this.outlineDrawer.style.top = `${this.toolbarEl.offsetHeight + 8}px`;
    }
    const has = this.outlineTree.length > 0;
    this.outlineBtn.disabled = !has;
    this.outlineEmptyEl.hidden = has;
    renderOutlineDom(this.outlineBody, this.outlineTree, (idx) =>
      this.jumpToOutlinePage(idx),
    );
    this.outlineDrawer.hidden = !has;
  }
  /** #3 UI 批 E：空态引导同步（文档在场 = 隐藏；打开失败/拆卸后回显）。 */
  private syncGuide(): void {
    this.guideEl.hidden = this.doc !== null;
  }

  /** #466：pdf.js 链接 dest（数组/命名目标）→ 0 基页号；不可解析 = null
   * （悬空命名/GoToR 远程目标的 Unresolved 面——pdf-render-ext §4）。 */
  private async resolveLinkDest(
    doc: PDFDocumentProxy,
    dest: unknown,
  ): Promise<number | null> {
    try {
      let d: unknown = dest;
      if (typeof d === 'string') d = await doc.getDestination(d);
      if (!Array.isArray(d) || d.length === 0) return null;
      const idx = await doc.getPageIndex(d[0] as Parameters<
        typeof doc.getPageIndex
      >[0]);
      return idx;
    } catch {
      return null;
    }
  }

  /** #466：跳页（内链点击——目标页滚到视口顶 + 页码行同步）。 */
  private jumpToPage(idx: number): void {
    const st = this.pages[idx];
    if (!st) return;
    st.el.scrollIntoView({ block: 'start', behavior: 'auto' });
    this.updatePageLabel();
  }

  /** #466：全部页链接热区投影（open 尾 + relayout 重投影）。 */
  private renderAllLinkOverlays(): void {
    for (let i = 0; i < this.pages.length; i++) this.renderLinkOverlay(i);
  }

  /** #466：单页链接热区（内链 = preventDefault + 跳页；外链 = 白名单
   * 后 href + _blank + noopener——手势级可点击 <a>）。 */
  private renderLinkOverlay(page: number): void {
    const st = this.pages[page];
    if (!st) return;
    st.linkLayer.replaceChildren();
    for (const link of this.pageLinks[page] ?? []) {
      const box = this.userRectToViewport(st, link.rect);
      const a = document.createElement('a');
      a.className = 'pdf-link-hotspot';
      a.dataset.page = String(page);
      if (link.target !== null) {
        a.href = '#';
        a.title = t('pdf.linkToPage', { n: link.target + 1 });
        a.addEventListener('click', (e) => {
          e.preventDefault();
          this.jumpToPage(link.target as number);
        });
      } else {
        a.href = link.url ?? '#';
        a.target = '_blank';
        a.rel = 'noopener noreferrer';
        a.title = link.url ?? '';
      }
      a.style.left = `${box.x}px`;
      a.style.top = `${box.y}px`;
      a.style.width = `${box.w}px`;
      a.style.height = `${box.h}px`;
      st.linkLayer.appendChild(a);
    }
  }

  /** #472：属性面板同步（按钮态 + 字段表；抽屉 top 随工具栏实际高度
   * 锚定——wrap 两行不叠盖；抽屉开合 = 用户选择，换档不强制重置）。 */
  private syncPropsPanel(): void {
    if (!this.propsBtn || !this.propsBody) return;
    this.propsBtn.disabled = this.docProps === null;
    if (this.propsDrawer && this.toolbarEl) {
      this.propsDrawer.style.top = `${this.toolbarEl.offsetHeight + 8}px`;
    }
    if (this.docProps) {
      const fields = propFields(
        this.docProps,
        t('pdf.propsEmpty'),
        {
          title: t('pdf.propsTitleField'),
          author: t('pdf.propsAuthor'),
          subject: t('pdf.propsSubject'),
          keywords: t('pdf.propsKeywords'),
          producer: t('pdf.propsProducer'),
          creator: t('pdf.propsCreator'),
          created: t('pdf.propsCreated'),
          modified: t('pdf.propsModified'),
          pdfVersion: t('pdf.propsPdfVersion'),
          pages: t('pdf.propsPages'),
          fileSize: t('pdf.propsFileSize'),
          encrypted: t('pdf.propsEncrypted'),
          warnings: t('pdf.propsWarnings'),
        },
        (n) => `${n} B`,
        (b) => (b ? t('pdf.propsYes') : t('pdf.propsNo')),
      );
      renderPropsDom(this.propsBody, fields);
    } else {
      this.propsBody.replaceChildren();
    }
  }

  /** 旧文档拆卸（IO/渲染任务/位图缓存/overlay/文本缓存全清）。 */
  private teardownDocument(): void {
    this.io?.disconnect();
    this.io = null;
    this.renderChain.clear();
    for (const st of this.pages) {
      st.renderTask?.cancel();
      st.textLayerActive?.cancel();
      st.renderToken++;
    }
    this.pages.length = 0;
    this.cache.clear();
    this.tcCache.clear();
    this.contentEl.replaceChildren();
    this.annotations = [];
    this.noteText.clear();
    this.evidenceRegions.length = 0;
    this.searchQuery = '';
    this.searchHits = [];
    this.searchIndex = -1;
    this.searchCount.textContent = '';
    this.textInjected = null;
    this.session = null;
    this.writerAttached = false;
    this.cancelFreetextInput();
    if (this.loadingTask) {
      void this.loadingTask.destroy().catch(() => {});
      this.loadingTask = null;
    }
    this.doc = null;
    // #475：注释列表面拆卸（层随页元素移除;开合保留用户选择）。
    this.syncAnnotsPanel();
    // #466：链接面拆卸（热区/策略拒绝计数归零——层随页元素移除）。
    this.pageLinks = [];
    this.linkPolicySkipped = [];

    // #472：属性面拆卸（数据归零 + 抽屉隐藏；按钮态随 syncPropsPanel）。
    this.docProps = null;
    if (this.propsBody) this.propsBody.replaceChildren();
    if (this.propsDrawer) this.propsDrawer.hidden = true;
    this.syncPropsPanel();


    // #463：大纲面拆卸（树/跳过数归零 + 抽屉隐藏——按钮态随 syncOutlinePanel）。
    this.outlineTree = [];
    this.outlineSkipped = 0;
    if (this.outlineBody) this.outlineBody.replaceChildren();
    if (this.outlineDrawer) this.outlineDrawer.hidden = true;
    // #3 UI 批 E：拆卸后表面归零——引导卡片回显（打开失败/换开路径）。
    this.syncGuide();
    this.syncOutlinePanel();
  }

  /**
   * v0 文本层注入（会话文本唯一来源——wasm32 无 pdfium 引擎）：
   * pdf.js getTextContent 逐页抽取 → set_page_text。返回注入页数
   * promise（Search 前等待其收敛）。
   */
  private injectPageText(proxies: PDFPageProxy[]): Promise<number> {
    const session = this.session;
    if (!session || typeof session.set_page_text !== 'function') {
      return Promise.resolve(0);
    }
    return (async (): Promise<number> => {
      let injected = 0;
      for (let i = 0; i < proxies.length; i++) {
        try {
          const tc = await this.textContent(proxies[i], i);
          const text = tc.items
            .map((item) => ('str' in item ? item.str : ''))
            .join('\n');
          session.set_page_text?.(i, text);
          injected += 1;
        } catch {
          // 单页抽取失败：跳过（该页无会话文本——搜索面诚实为无命中）
        }
      }
      return injected;
    })();
  }

  /** textContent（带页级缓存——文本层/搜索/注入共用；与 scale 无关）。 */
  private async textContent(proxy: PDFPageProxy, idx: number): Promise<PdfTextContent> {
    const cached = this.tcCache.get(idx);
    if (cached) return cached;
    const tc = await proxy.getTextContent();
    this.tcCache.set(idx, tc);
    return tc;
  }

  // -----------------------------------------------------------------------
  // 会话命令包装（typed；形态自检）
  // -----------------------------------------------------------------------

  private sessionExec(cmdJson: string): PdfOk<OutcomeJson> | PdfFail {
    if (!this.session) {
      this.showStatus(BRIDGE_ABSENT.message, 'error');
      return BRIDGE_ABSENT;
    }
    try {
      const out = JSON.parse(this.session.execute(cmdJson)) as OutcomeJson;
      return { ok: true, value: out };
    } catch (e) {
      const fail = withShapeHint(wasmFail(e));
      this.showStatus(t('pdf.sessionCmdFailed', { code: fail.code, message: fail.message }), 'error');
      return fail;
    }
  }

  private async refreshAnnotations(): Promise<void> {
    const r = this.sessionExec(buildCmdAnnotateList());
    if (r.ok && 'AnnotListed' in r.value) {
      this.annotations = r.value.AnnotListed;
    } else {
      this.annotations = [];
    }
    this.renderAllAnnotOverlays();
    this.syncAnnotsPanel(); // #475：会话清单变化 → 列表面同步
  }

  private listAnnotations(filter: AnnotFilterJson): AnnotationJson[] | PdfFail {
    const r = this.sessionExec(buildCmdAnnotateList(filter));
    if (!r.ok) return r;
    if (!('AnnotListed' in r.value)) return [];
    return r.value.AnnotListed;
  }

  /** 加标注（成功后局部刷新 overlay + 本地镜像列表）。 */
  private addAnnotation(
    page: number,
    kind: AnnotKindJson,
  ): PdfOk<{ id: number; annotation: AnnotationJson }> | PdfFail {
    const r = this.sessionExec(buildCmdAnnotateAdd(page, kind));
    if (!r.ok) return r;
    if (!('AnnotAdded' in r.value)) {
      return {
        ok: false,
        code: 'outcome_shape',
        message: JSON.stringify(r.value).slice(0, 200),
      };
    }
    const annot = r.value.AnnotAdded;
    this.annotations.push(annot);
    this.renderAnnotOverlay(page);
    this.syncAnnotsPanel(); // #475：增批注 → 列表面同步
    this.syncButtons();
    return { ok: true, value: { id: annot.id, annotation: annot } };
  }

  /** 程序化/选区高亮（rect 用户空间 {x,y,w,h}，y 自页底）。 */
  addHighlightRect(
    page: number,
    rect: { x: number; y: number; w: number; h: number },
  ): PdfOk<{ id: number; annotation: AnnotationJson }> | PdfFail {
    const quad: QuadJson = {
      points: [
        { x: rect.x, y: rect.y },
        { x: rect.x + rect.w, y: rect.y },
        { x: rect.x + rect.w, y: rect.y + rect.h },
        { x: rect.x, y: rect.y + rect.h },
      ],
    };
    return this.addAnnotation(page, {
      Highlight: { quads: [quad], color: HIGHLIGHT_COLOR },
    });
  }

  // -----------------------------------------------------------------------
  // 会话搜索（语义命中权威）+ pdf.js 几何显示（双面诚实）
  // -----------------------------------------------------------------------

  private async searchOnce(
    q: string,
  ): Promise<PdfOk<{ hits: SearchHitJson[]; truncated: boolean }> | PdfFail> {
    if (!q) {
      this.clearSearch();
      return { ok: true, value: { hits: [], truncated: false } };
    }
    if (!this.session) {
      this.showStatus(BRIDGE_ABSENT.message, 'error');
      return BRIDGE_ABSENT;
    }
    await this.textInjected; // v0 注入收敛后再查（会话文本面完整）
    const r = this.sessionExec(
      buildCmdSearch(q, /* caseInsensitive */ true, MAX_SEARCH_HITS),
    );
    if (!r.ok) return r;
    if (!('SearchResults' in r.value)) {
      const fail: PdfFail = {
        ok: false,
        code: 'outcome_shape',
        message: JSON.stringify(r.value).slice(0, 200),
      };
      return fail;
    }
    const { hits, truncated } = r.value.SearchResults;
    this.searchQuery = q;
    this.searchHits = hits;
    this.searchIndex = hits.length > 0 ? 0 : -1;
    await this.renderSearchOverlay();
    this.searchCount.textContent =
      hits.length === 0
        ? t('pdf.zeroHits')
        : `${this.searchIndex + 1} / ${hits.length}${truncated ? '+' : ''}`;
    this.showStatus(
      t('pdf.searchStatus', {
        q,
        n: hits.length,
        trunc: truncated ? t('pdf.searchTrunc') : '',
      }),
    );
    if (this.searchIndex >= 0) this.focusHit(this.searchIndex);
    return { ok: true, value: { hits, truncated } };
  }

  private clearSearch(): void {
    this.searchQuery = '';
    this.searchHits = [];
    this.searchIndex = -1;
    this.searchCount.textContent = '';
    for (const st of this.pages) st.searchLayer.replaceChildren();
  }

  private navigateHit(dir: number): void {
    if (this.searchHits.length === 0) return;
    this.searchIndex =
      (this.searchIndex + dir + this.searchHits.length) % this.searchHits.length;
    this.searchCount.textContent = `${this.searchIndex + 1} / ${this.searchHits.length}`;
    this.focusHit(this.searchIndex);
  }

  /** 跳转活跃命中：滚到所在页 + 标记该页首个命中框（v1 显示近似——
   * 会话命中是权威计数，显示框来自 pdf.js 文本几何）。 */
  private focusHit(index: number): void {
    const hit = this.searchHits[index];
    if (!hit) return;
    const st = this.pages[hit.page];
    if (!st) return;
    st.el.scrollIntoView({ block: 'center', behavior: 'auto' });
    this.markActiveHit();
  }

  private markActiveHit(): void {
    for (const st of this.pages) {
      for (const el of st.searchLayer.children) el.classList.remove('active');
    }
    const active = this.searchHits[this.searchIndex];
    if (!active) return;
    const layer = this.pages[active.page]?.searchLayer;
    if (!layer || layer.children.length === 0) return;
    // 活跃命中在其页内的序（会话命中计数；显示框数量不截断时一一致）
    let ordinal = 0;
    for (let i = 0; i <= this.searchIndex; i++) {
      if (this.searchHits[i].page === active.page) ordinal += 1;
    }
    const el = layer.children[Math.min(ordinal - 1, layer.children.length - 1)];
    el.classList.add('active');
  }

  /** 搜索几何显示层：pdf.js textContent 项内匹配（几何来自 pdf.js，
   * 语义命中来自会话——显示层独立于会话命中页集，逐页全量画）。 */
  private async renderSearchOverlay(): Promise<void> {
    const needle = this.searchQuery.toLowerCase();
    for (let i = 0; i < this.pages.length; i++) {
      const st = this.pages[i];
      st.searchLayer.replaceChildren();
      if (!needle) continue;
      let tc: PdfTextContent;
      try {
        tc = await this.textContent(st.proxy, i);
      } catch {
        continue;
      }
      // 拼接页文本（项间 \n）+ 项边界表
      let hay = '';
      const spans: { str: string; transform: number[]; width: number; height: number; start: number }[] = [];
      for (const item of tc.items) {
        if (!('str' in item)) continue; // TextMarkedContent
        spans.push({
          str: item.str,
          transform: item.transform,
          width: item.width,
          height: item.height,
          start: hay.length,
        });
        hay += item.str + '\n';
      }
      const lower = hay.toLowerCase();
      let idx = lower.indexOf(needle);
      while (idx !== -1) {
        const end = idx + needle.length;
        for (const span of spans) {
          const s0 = span.start;
          const s1 = span.start + span.str.length;
          if (s1 <= idx || s0 >= end) continue;
          // 项内字符区间 → x 比例近似（等宽假设——v1 显示面）
          const from = Math.max(0, (idx - s0) / span.str.length);
          const to = Math.min(1, (end - s0) / span.str.length);
          const rect = PdfViewUI.itemRectUserSpace(span, from, to);
          const box = this.userRectToViewport(st, rect);
          const div = document.createElement('div');
          div.className = 'pdf-search-hit';
          div.style.left = `${box.x}px`;
          div.style.top = `${box.y}px`;
          div.style.width = `${box.w}px`;
          div.style.height = `${box.h}px`;
          st.searchLayer.appendChild(div);
        }
        idx = lower.indexOf(needle, end);
      }
    }
    this.markActiveHit();
  }

  /** 文本项字符区间的用户空间矩形（transform [a,b,c,d,e,f]——基线起点
   * (e,f)，宽 width、高 height；上下沿按 ascent .8 / descent .22 近似）。 */
  private static itemRectUserSpace(
    item: { transform: number[]; width: number; height: number },
    from: number,
    to: number,
  ): { x0: number; y0: number; x1: number; y1: number } {
    const e = item.transform[4];
    const f = item.transform[5];
    const h = item.height || 10;
    return {
      x0: e + item.width * from,
      x1: e + item.width * to,
      y0: f - 0.22 * h,
      y1: f + 0.8 * h,
    };
  }

  // -----------------------------------------------------------------------
  // 页构建/布局/渲染
  // -----------------------------------------------------------------------

  private buildPages(
    proxies: PDFPageProxy[],
    geoms: { widthPt: number; heightPt: number; rotationDeg: number }[],
  ): void {
    const frag = document.createDocumentFragment();
    for (let i = 0; i < proxies.length; i++) {
      const el = document.createElement('div');
      el.className = 'pdf-page';
      el.dataset.page = String(i);

      const canvas = document.createElement('canvas');
      canvas.className = 'pdf-canvas';

      const textLayer = document.createElement('div');
      textLayer.className = 'textLayer';

      const overlay = document.createElement('div');
      overlay.className = 'pdf-overlay';
      const annotLayer = document.createElement('div');
      annotLayer.className = 'pdf-annot-layer';
      const searchLayer = document.createElement('div');
      searchLayer.className = 'pdf-search-layer';
      const evidenceLayer = document.createElement('div');
      evidenceLayer.className = 'pdf-evidence-layer';
      // #466：链接层（overlay 末位——点击优先级最高；打印态随 .pdf-overlay 隐藏）
      const linkLayer = document.createElement('div');
      linkLayer.className = 'pdf-link-layer';
      overlay.append(annotLayer, searchLayer, evidenceLayer, linkLayer);

      el.append(canvas, textLayer, overlay);
      frag.appendChild(el);

      const st: PageState = {
        el,
        canvas,
        textLayer,
        overlay,
        annotLayer,
        searchLayer,
        evidenceLayer,
        linkLayer,
        proxy: proxies[i],
        viewport: proxies[i].getViewport({ scale: this.scale }),
        rendered: false,
        lastSource: 'none',
        renderToken: 0,
        renderTask: null,
        textLayerActive: null,
      };
      this.sizePage(st);
      this.pages.push(st);
    }
    this.contentEl.replaceChildren(frag);

    // 懒渲染（视口 ±1 页）
    this.io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const idx = Number((entry.target as HTMLElement).dataset.page);
          if (Number.isFinite(idx)) void this.renderPage(idx);
        }
      },
      { root: this.viewportEl, rootMargin: IO_ROOT_MARGIN },
    );
    for (const st of this.pages) this.io.observe(st.el);
  }

  /** 单页尺寸 + CSS 变量（textLayer 几何基准）。 */
  private sizePage(st: PageState): void {
    st.viewport = st.proxy.getViewport({ scale: this.scale });
    st.el.style.width = `${st.viewport.width}px`;
    st.el.style.height = `${st.viewport.height}px`;
    st.el.style.setProperty('--total-scale-factor', String(this.scale));
    st.el.style.setProperty('--scale-round-x', '1px');
    st.el.style.setProperty('--scale-round-y', '1px');
  }

  /** 重排：全部页重取视口 + 尺寸 + 文本层/overlay 重投影。 */
  private relayout(): void {
    for (const st of this.pages) {
      this.sizePage(st);
      st.rendered = false; // 尺寸变化即失显（内容待渲染/贴缓存）
      st.renderToken++;
      st.renderTask?.cancel();
      st.renderTask = null;
      st.textLayerActive?.cancel();
      st.textLayerActive = null;
      st.textLayer.replaceChildren();
    }
    void this.renderSearchOverlay();
    this.renderAllAnnotOverlays();
    this.renderAllEvidenceOverlays();
    this.renderAllLinkOverlays(); // #466：链接热区随 scale 重投影
  }

  private scheduleRenderVisible(): void {
    if (this.rafPending) return;
    this.rafPending = true;
    requestAnimationFrame(() => {
      this.rafPending = false;
      for (let i = 0; i < this.pages.length; i++) {
        if (this.isPageNearView(i)) void this.renderPage(i);
      }
    });
  }

  private isPageNearView(i: number): boolean {
    const st = this.pages[i];
    if (!st.el.isConnected) return false;
    const top = st.el.offsetTop;
    const bottom = top + st.el.offsetHeight;
    const viewTop = this.viewportEl.scrollTop;
    const viewBottom = viewTop + this.viewportEl.clientHeight;
    const margin = this.viewportEl.clientHeight;
    return bottom >= viewTop - margin && top <= viewBottom + margin;
  }

  /** 页渲染入口（单页串行链——并发调用排队；pdf.js 禁止同 canvas
   * 并发 render，取消/失败链后续请求自动重试）。 */
  private renderPage(i: number): Promise<void> {
    const st = this.pages[i];
    if (!st) return Promise.resolve();
    const prev = this.renderChain.get(i) ?? Promise.resolve();
    const next = prev.catch(() => {}).then(() => this.doRenderPage(i));
    this.renderChain.set(i, next);
    return next;
  }

  // -----------------------------------------------------------------------
  // #240 层③：打印面（全页渲染 → window.print → 懒态恢复）
  // -----------------------------------------------------------------------

  /** 打印按钮路径：全页渲染（懒渲染面向屏幕——打印需要每页位图在
   * 场）→ window.print()（打印 CSS 键于 body[data-pdf-active]）。
   * afterprint 监听器负责懒态恢复（见 installHandlers）。 */
  private async printDocument(): Promise<void> {
    if (!this.doc || this.pages.length === 0) {
      this.showStatus(t('pdf.printNoDoc'), 'warn');
      return;
    }
    const rendered = await this.renderAllPages();
    if (rendered < this.pages.length) {
      // 渲染链有页未完成（取消/失败）——如实提示不静默（打印可能缺页）。
      this.showStatus(
        t('pdf.printPartial', { done: rendered, total: this.pages.length }),
        'warn',
      );
    }
    window.print();
  }

  /** 全页渲染（顺序走既有 renderPage 链——已渲染/缓存命中的页零成本；
   * 与探针 __pdfPrintPrepare / beforeprint best-effort 同一路径）。
   * 返回渲染完成数。 */
  private async renderAllPages(): Promise<number> {
    for (let i = 0; i < this.pages.length; i++) {
      await this.renderPage(i);
    }
    return this.pages.filter((st) => st.rendered).length;
  }

  /** 实际渲染（缓存命中贴位图；否则 pdf.js render + 位图入缓存）。
   * token 化：旧渲染完成时 token 过期则丢弃（防错序）。 */
  private async doRenderPage(i: number): Promise<void> {
    const st = this.pages[i];
    if (!st || st.rendered) return;
    const key = this.cacheKey(i);
    const bitmap = this.cache.peek(key);
    if (bitmap) {
      this.paintBitmap(st, bitmap);
      st.rendered = true;
      st.lastSource = 'cache';
      void this.renderTextLayer(i);
      return;
    }
    const token = ++st.renderToken;
    // Display-side pixel clamp (review S-2 — mirrors mdc-pdf L2 limits:
    // single side 8192px / total 64M px). pdf.js has no built-in cap; an
    // unclamped dpr at high zoom on large pages would allocate ~hundreds
    // of MB of canvas backing store. Clamp the raster resolution (CSS
    // dimensions unchanged) — engine-side rejection semantics preserved
    // as a display-side degrade.
    const L2_MAX_SIDE = 8192;
    const L2_MAX_PIXELS = 64_000_000;
    const dprRaw = window.devicePixelRatio || 1;
    let dpr = dprRaw;
    let w = Math.floor(st.viewport.width * dpr);
    let h = Math.floor(st.viewport.height * dpr);
    if (w > L2_MAX_SIDE || h > L2_MAX_SIDE || w * h > L2_MAX_PIXELS) {
      const bySide = Math.min(L2_MAX_SIDE / w, L2_MAX_SIDE / h);
      const byTotal = Math.sqrt(L2_MAX_PIXELS / (w * h));
      dpr = Math.max(1, dprRaw * Math.min(bySide, byTotal));
      w = Math.floor(st.viewport.width * dpr);
      h = Math.floor(st.viewport.height * dpr);
    }
    st.canvas.width = Math.max(1, w);
    st.canvas.height = Math.max(1, h);
    st.canvas.style.width = `${st.viewport.width}px`;
    st.canvas.style.height = `${st.viewport.height}px`;
    const transform = dpr === 1 ? undefined : [dpr, 0, 0, dpr, 0, 0];
    try {
      const task = st.proxy.render({ canvas: st.canvas, viewport: st.viewport, transform });
      st.renderTask = task;
      await task.promise;
    } catch {
      return; // 取消/失败——下次渲染 token 已递增
    }
    if (token !== st.renderToken) return; // 过期丢弃（防错序）
    st.rendered = true;
    st.lastSource = 'render';
    try {
      const bmp = await createImageBitmap(st.canvas);
      if (token !== st.renderToken) {
        bmp.close();
        return;
      }
      this.cache.put(key, bmp);
    } catch {
      // 位图快照失败——只影响回滚缓存命中，不影响显示
    }
    void this.renderTextLayer(i);
  }

  private paintBitmap(st: PageState, bitmap: ImageBitmap): void {
    const ctx = st.canvas.getContext('2d');
    if (!ctx) return;
    st.canvas.width = bitmap.width;
    st.canvas.height = bitmap.height;
    st.canvas.style.width = `${st.viewport.width}px`;
    st.canvas.style.height = `${st.viewport.height}px`;
    ctx.drawImage(bitmap, 0, 0);
  }

  /** textLayer 渲染（pdf.js 标准两层——文字选区原生支持）。 */
  private async renderTextLayer(i: number): Promise<void> {
    const st = this.pages[i];
    if (!st || st.textLayer.childElementCount > 0) return;
    try {
      const tc = await this.textContent(st.proxy, i);
      const tl = new this.pdfjs.TextLayer({
        textContentSource: tc,
        container: st.textLayer,
        viewport: st.viewport,
      });
      st.textLayerActive = tl;
      await tl.render();
    } catch {
      // 文本层失败不阻断位图显示（选区/搜索显示面降级）
    }
  }

  /** 远页 canvas 释放（位图留缓存——回滚命中直接贴）。 */
  private clearFarCanvases(): void {
    const viewTop = this.viewportEl.scrollTop;
    const viewBottom = viewTop + this.viewportEl.clientHeight;
    const margin = this.viewportEl.clientHeight * CLEAR_MARGIN_FACTOR;
    for (const st of this.pages) {
      const top = st.el.offsetTop;
      const bottom = top + st.el.offsetHeight;
      const far = bottom < viewTop - margin || top > viewBottom + margin;
      if (far && st.rendered) {
        st.canvas.width = 0; // 释放后备存储
        st.canvas.height = 0;
        st.textLayer.replaceChildren(); // 选区 DOM 同步释放
        st.textLayerActive?.cancel();
        st.textLayerActive = null;
        st.rendered = false;
        st.lastSource = 'none';
      }
    }
  }

  private cacheKey(i: number): string {
    return `${i}:${this.scale.toFixed(3)}`;
  }

  // -----------------------------------------------------------------------
  // 页码跟踪 / 缩放
  // -----------------------------------------------------------------------

  private updatePageLabel(): void {
    if (this.pages.length === 0) {
      this.pageLabel.textContent = t('pdf.noPages');
      return;
    }
    const center = this.viewportEl.scrollTop + this.viewportEl.clientHeight / 2;
    let current = 0;
    for (let i = 0; i < this.pages.length; i++) {
      if (center >= this.pages[i].el.offsetTop) current = i;
      else break;
    }
    this.pageLabel.textContent = `${current + 1} / ${this.pages.length}`;
  }

  private fitScale(): number {
    if (this.pages.length === 0) return 1;
    const maxW = Math.max(
      ...this.pages.map((st) => st.viewport.width / st.viewport.scale),
    );
    const avail = this.viewportEl.clientWidth - PAGE_MARGIN;
    if (maxW <= 0 || avail <= 0) return 1;
    return clampNum(avail / maxW, SCALE_MIN, SCALE_MAX);
  }

  /** 缩放调整：失效缓存 + 重排 + 滚动锚点保持（视口中心页/中心点）。 */
  private setScale(next: number, opts: { skipAnchor?: boolean } = {}): void {
    if (this.pages.length === 0) return;
    const target = clampNum(next, SCALE_MIN, SCALE_MAX);
    if (Math.abs(target - this.scale) < 1e-9 && this.pages[0].el.style.width !== '') {
      return; // 同值免重排（页尺寸已就位）
    }

    // 锚点：视口中心的页 + 用户空间点（改 scale 前取）
    let anchor: { page: number; pdfX: number; pdfY: number } | null = null;
    if (!opts.skipAnchor && this.viewportEl.clientHeight > 0) {
      const center = this.viewportEl.scrollTop + this.viewportEl.clientHeight / 2;
      let page = 0;
      for (let i = 0; i < this.pages.length; i++) {
        if (center >= this.pages[i].el.offsetTop) page = i;
        else break;
      }
      const st = this.pages[page];
      const localY = center - st.el.offsetTop;
      const localX = this.viewportEl.clientWidth / 2;
      const [pdfX, pdfY] = st.viewport.convertToPdfPoint(localX, localY);
      anchor = { page, pdfX, pdfY };
    }

    this.scale = target;
    this.zoomLabel.textContent = `${Math.round(target * 100)}%`;
    this.cache.clear(); // 缩放失效缓存（旧分辨率位图不再可用）
    this.relayout();
    this.scheduleRenderVisible();

    if (anchor) {
      const st = this.pages[anchor.page];
      const [lx, ly] = st.viewport.convertToViewportPoint(anchor.pdfX, anchor.pdfY);
      const targetScrollTop = st.el.offsetTop + ly - this.viewportEl.clientHeight / 2;
      this.viewportEl.scrollTop = Math.max(0, targetScrollTop);
    }
    this.updatePageLabel();
  }

  // -----------------------------------------------------------------------
  // 标注交互（四 kind）
  // -----------------------------------------------------------------------

  private setMode(mode: AnnotMode): void {
    this.mode = mode;
    this.cancelFreetextInput();
    this.inkStroke = [];
    this.inkPreview?.remove();
    this.inkPreview = null;
    this.viewportEl.classList.toggle('pdf-mode-highlight', mode === 'highlight');
    this.viewportEl.classList.toggle('pdf-mode-ink', mode === 'ink');
    this.showStatus(
      mode === ''
        ? t('pdf.annotateOff')
        : mode === 'highlight'
          ? t('pdf.annotateHighlight')
          : t('pdf.annotateDraw', { mode: modeLabel(mode) }),
    );
  }

  /** 高亮：textLayer 选区 → client rects → 用户空间 Quad → AnnotateAdd。 */
  private async commitSelectionHighlight(): Promise<void> {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0) return;
    const range = sel.getRangeAt(0);
    const rects = Array.from(range.getClientRects()).filter(
      (r) => r.width > 1 && r.height > 1,
    );
    if (rects.length === 0) return;
    // client rect → 页归属（rect 中心落页判定）→ 页内 viewport 坐标
    const perPage = new Map<number, QuadJson[]>();
    for (const rect of rects) {
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      let owner: PageState | null = null;
      let ownerRect: DOMRect | null = null;
      for (const st of this.pages) {
        const pr = st.el.getBoundingClientRect();
        if (cx >= pr.left && cx <= pr.right && cy >= pr.top && cy <= pr.bottom) {
          owner = st;
          ownerRect = pr;
          break;
        }
      }
      if (!owner || !ownerRect) continue;
      const x0 = rect.left - ownerRect.left;
      const x1 = rect.right - ownerRect.left;
      const y0 = rect.top - ownerRect.top;
      const y1 = rect.bottom - ownerRect.top;
      // 环序：底左→底右→顶右→顶左（p0→p1 = 文本行方向边）
      const [bx0, by0] = owner.viewport.convertToPdfPoint(x0, y1);
      const [bx1, by1] = owner.viewport.convertToPdfPoint(x1, y1);
      const [tx1, ty1] = owner.viewport.convertToPdfPoint(x1, y0);
      const [tx0, ty0] = owner.viewport.convertToPdfPoint(x0, y0);
      const idx = this.pages.indexOf(owner);
      const quads = perPage.get(idx) ?? [];
      quads.push({
        points: [
          { x: bx0, y: by0 },
          { x: bx1, y: by1 },
          { x: tx1, y: ty1 },
          { x: tx0, y: ty0 },
        ],
      });
      perPage.set(idx, quads);
    }
    if (perPage.size === 0) return;
    let added = 0;
    let firstFail: PdfFail | null = null;
    for (const [idx, quads] of perPage) {
      const r = this.addAnnotation(idx, {
        Highlight: { quads, color: HIGHLIGHT_COLOR },
      });
      if (r.ok) added += 1;
      else firstFail = firstFail ?? r;
    }
    if (added > 0) {
      sel.removeAllRanges(); // 提交后清选区（视觉反馈）
      this.showStatus(t('pdf.highlightAdded', { n: added }));
    } else if (firstFail) {
      this.showStatus(
        t('pdf.highlightFailed', { code: firstFail.code, message: firstFail.message }),
        'error',
      );
    }
  }

  /** 便签：点击落点 → prompt 文本 → Note（v0 无文本载荷——悬停文本为
   * 宿主侧显示面，见 noteText 注释）。 */
  private addNoteAt(page: number, localX: number, localY: number): void {
    const st = this.pages[page];
    const [px, py] = st.viewport.convertToPdfPoint(localX, localY);
    const half = NOTE_SIZE_PT / 2;
    const text =
      window.prompt('note text (v0: hover display only — not persisted to /Contents yet)') ?? '';
    if (!text) return;
    const r = this.addAnnotation(page, {
      Note: { rect: { x0: px - half, y0: py - half, x1: px + half, y1: py + half } },
    });
    if (r.ok) {
      this.noteText.set(r.value.id, text);
      this.renderAnnotOverlay(page);
      this.showStatus(t('pdf.noteAdded', { id: r.value.id }));
    }
  }

  /** 自由文本：点击落点 → 行内输入框 → Enter/blur 提交。 */
  private beginFreeText(page: number, localX: number, localY: number): void {
    const st = this.pages[page];
    const input = document.createElement('input');
    input.type = 'text';
    input.className = 'pdf-freetext-input';
    input.placeholder = 'free text — Enter commits, Esc cancels';
    input.style.left = `${localX}px`;
    input.style.top = `${localY}px`;
    input.style.width = '160px';
    st.overlay.appendChild(input);
    input.focus();
    this.freetextInput = input;
    const commit = (): void => {
      const value = input.value.trim();
      const rect = input.getBoundingClientRect();
      const pageRect = st.el.getBoundingClientRect();
      const x0 = rect.left - pageRect.left;
      const y0 = rect.top - pageRect.top;
      const x1 = x0 + rect.width;
      const y1 = y0 + rect.height;
      this.cancelFreetextInput();
      if (!value) return;
      const [ux0, uy0] = st.viewport.convertToPdfPoint(x0, y1);
      const [ux1, uy1] = st.viewport.convertToPdfPoint(x1, y0);
      const r = this.addAnnotation(page, {
        FreeText: {
          rect: {
            x0: Math.min(ux0, ux1),
            y0: Math.min(uy0, uy1),
            x1: Math.max(ux0, ux1),
            y1: Math.max(uy0, uy1),
          },
          text: value,
        },
      });
      if (r.ok) this.showStatus(t('pdf.freeTextAdded', { id: r.value.id }));
    };
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        commit();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        this.cancelFreetextInput();
      }
      e.stopPropagation();
    });
    input.addEventListener('blur', () => {
      if (this.freetextInput === input) commit();
    });
  }

  private cancelFreetextInput(): void {
    if (this.freetextInput) {
      this.freetextInput.remove();
      this.freetextInput = null;
    }
  }

  private drawInkPreview(st: PageState): void {
    if (!this.inkPreview) return;
    const pts = this.inkStroke.map((p) => {
      const [vx, vy] = st.viewport.convertToViewportPoint(p.x, p.y);
      return `${vx.toFixed(1)},${vy.toFixed(1)}`;
    });
    this.inkPreview.setAttribute(
      'viewBox',
      `0 0 ${st.viewport.width} ${st.viewport.height}`,
    );
    // #282（P2-25 伴修）：与 renderAnnotOverlay 同模式——预览侧同样改
    // createElementNS + setAttribute（this.inkColor/inkWidth 现为 UI 选择器
    // 定值，同修消除文件内残留的 innerHTML 属性插值面）。innerHTML 赋值
    // 隐含的清空改 replaceChildren 显式化。
    this.inkPreview.replaceChildren();
    const polyline = document.createElementNS(
      'http://www.w3.org/2000/svg',
      'polyline',
    );
    polyline.setAttribute('points', pts.join(' '));
    polyline.setAttribute('fill', 'none');
    polyline.setAttribute('stroke', this.inkColor);
    polyline.setAttribute('stroke-width', String(this.inkWidth));
    polyline.setAttribute('stroke-linecap', 'round');
    polyline.setAttribute('stroke-linejoin', 'round');
    this.inkPreview.appendChild(polyline);
  }

  private finishInkStroke(): void {
    const page = this.inkPage;
    const stroke = this.inkStroke;
    this.inkPreview?.remove();
    this.inkPreview = null;
    this.inkStroke = [];
    this.inkPage = -1;
    if (page < 0 || stroke.length < 2) return; // 单点不成划
    const r = this.addAnnotation(page, this.inkPayload([stroke]));
    if (r.ok) this.showStatus(t('pdf.inkAdded', { id: r.value.id }));
  }

  /** #45：墨迹载荷构造（UI 视觉态 → color/width 字段；预览/真实笔划
   * /探针三面同源——选择器改状态后下一笔即生效）。 */
  private inkPayload(strokes: PdfPoint[][]): { Ink: { strokes: PdfPoint[][]; color: RgbJson; width: number } } {
    const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(this.inkColor);
    const color: RgbJson = m
      ? { r: parseInt(m[1], 16), g: parseInt(m[2], 16), b: parseInt(m[3], 16) }
      : { r: 0x1a, g: 0x73, b: 0xe8 }; // 非法 hex 兑底 = 缺省蓝
    return { Ink: { strokes, color, width: this.inkWidth } };
  }

  // ---- overlay 渲染（AnnotateList 数据源——本地图谱仅镜像显示）----

  private renderAllAnnotOverlays(): void {
    for (let i = 0; i < this.pages.length; i++) this.renderAnnotOverlay(i);
  }

  private renderAnnotOverlay(page: number): void {
    const st = this.pages[page];
    if (!st) return;
    st.annotLayer.replaceChildren();
    const annots = this.annotations.filter((a) => a.page === page);
    for (const annot of annots) {
      if ('Highlight' in annot.kind) {
        for (const quad of annot.kind.Highlight.quads) {
          const box = this.quadToViewportBox(st, quad);
          const div = document.createElement('div');
          div.className = 'pdf-annot-highlight';
          div.dataset.annotId = String(annot.id);
          div.style.left = `${box.x}px`;
          div.style.top = `${box.y}px`;
          div.style.width = `${box.w}px`;
          div.style.height = `${box.h}px`;
          st.annotLayer.appendChild(div);
        }
      } else if ('Note' in annot.kind) {
        const box = this.userRectToViewport(st, annot.kind.Note.rect);
        const div = document.createElement('div');
        div.className = 'pdf-annot-note';
        div.dataset.annotId = String(annot.id);
        div.title = this.noteText.get(annot.id) ?? `note ${annot.id}`;
        div.style.left = `${box.x}px`;
        div.style.top = `${box.y}px`;
        st.annotLayer.appendChild(div);
      } else if ('FreeText' in annot.kind) {
        const box = this.userRectToViewport(st, annot.kind.FreeText.rect);
        const div = document.createElement('div');
        div.className = 'pdf-annot-freetext';
        div.dataset.annotId = String(annot.id);
        div.textContent = annot.kind.FreeText.text;
        div.style.left = `${box.x}px`;
        div.style.top = `${box.y}px`;
        div.style.minWidth = `${Math.max(box.w, 24)}px`;
        div.style.minHeight = `${Math.max(box.h, 14)}px`;
        st.annotLayer.appendChild(div);
      } else if ('Ink' in annot.kind) {
        for (const stroke of annot.kind.Ink.strokes) {
          if (stroke.length < 2) continue;
          const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
          svg.classList.add('pdf-annot-ink');
          svg.setAttribute(
            'viewBox',
            `0 0 ${st.viewport.width} ${st.viewport.height}`,
          );
          const pts = stroke.map((p) => {
            const [vx, vy] = st.viewport.convertToViewportPoint(p.x, p.y);
            return `${vx.toFixed(1)},${vy.toFixed(1)}`;
          });
          // #45：视觉态从存储标注读（颜色 rgb() 面 + 载荷宽——会话往返
          // 后与落盘 /C /BS 一致；旧会话缺字段 = 引擎 serde 缺省蓝/2pt）。
          // #282（P2-25）：属性插值改 createElementNS + setAttribute——
          // 敌意/损坏标注字段经 innerHTML 可断 `stroke="` 注任意属性/
          // 子元素（v0 标注全本地构造不可达；PDF 既有标注解析落地即活化）。
          // setAttribute 把载荷存为惰性属性值，不再过 markup 解析器。
          const ink = annot.kind.Ink;
          const strokeCss = `rgb(${ink.color.r}, ${ink.color.g}, ${ink.color.b})`;
          const polyline = document.createElementNS(
            'http://www.w3.org/2000/svg',
            'polyline',
          );
          polyline.setAttribute('points', pts.join(' '));
          polyline.setAttribute('fill', 'none');
          polyline.setAttribute('stroke', strokeCss);
          polyline.setAttribute('stroke-width', String(ink.width));
          polyline.setAttribute('stroke-linecap', 'round');
          polyline.setAttribute('stroke-linejoin', 'round');
          svg.appendChild(polyline);
          st.annotLayer.appendChild(svg);
        }
      }
    }
  }

  /** 用户空间矩形 → 页内 viewport 盒（pdf.js v6 无 convertToViewportRectangle
   * 面，两点换算 + min/max 合成——旋转页也正确）。 */
  private userRectToViewport(
    st: PageState,
    rect: { x0: number; y0: number; x1: number; y1: number },
  ): { x: number; y: number; w: number; h: number } {
    const [ax, ay] = st.viewport.convertToViewportPoint(rect.x0, rect.y0);
    const [bx, by] = st.viewport.convertToViewportPoint(rect.x1, rect.y1);
    return {
      x: Math.min(ax, bx),
      y: Math.min(ay, by),
      w: Math.abs(bx - ax),
      h: Math.abs(by - ay),
    };
  }

  private quadToViewportBox(
    st: PageState,
    quad: QuadJson,
  ): { x: number; y: number; w: number; h: number } {
    const xs: number[] = [];
    const ys: number[] = [];
    for (const p of quad.points) {
      const [vx, vy] = st.viewport.convertToViewportPoint(p.x, p.y);
      xs.push(vx);
      ys.push(vy);
    }
    const x = Math.min(...xs);
    const y = Math.min(...ys);
    return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
  }

  // -----------------------------------------------------------------------
  // 证据区域叠加（钩子面——后续阶段接真数据）
  // -----------------------------------------------------------------------

  /** 证据区域显示（公开面——evidence 面板事件接线后续阶段接入）。 */
  highlightEvidence(regions: { page: number; rect: RectJson }[]): void {
    this.evidenceRegions.length = 0;
    this.evidenceRegions.push(...regions);
    this.renderAllEvidenceOverlays();
  }

  private renderAllEvidenceOverlays(): void {
    for (let i = 0; i < this.pages.length; i++) {
      const st = this.pages[i];
      st.evidenceLayer.replaceChildren();
      for (const region of this.evidenceRegions) {
        if (region.page !== i) continue;
        const box = this.userRectToViewport(st, region.rect);
        const div = document.createElement('div');
        div.className = 'pdf-evidence-region';
        div.style.left = `${box.x}px`;
        div.style.top = `${box.y}px`;
        div.style.width = `${box.w}px`;
        div.style.height = `${box.h}px`;
        st.evidenceLayer.appendChild(div);
      }
    }
  }

  // -----------------------------------------------------------------------
  // 保存（增量写：attach_writer → flush → product_bytes 下载）
  // -----------------------------------------------------------------------

  /** flush 事务（不下载）。加密档：flush 抛 S5 拒 → typed reason 显示。 */
  flush():
    | { ok: true; bytes_appended: number; prefix_bytes: number; bytes: string; len: number }
    | PdfFail {
    if (!this.session) {
      this.showStatus(BRIDGE_ABSENT.message, 'error');
      return BRIDGE_ABSENT;
    }
    if (!this.originalBytes) {
      return { ok: false, code: 'no_document', message: 'no document open' };
    }
    try {
      this.session.set_clock_ms(BigInt(Date.now()));
      if (!this.writerAttached) {
        // 首存懒装载写器（原字节——含安全扫描共用）；后续 flush 复用
        // 同一写器（增量链单调增长，重装原字节会丢已追加修订）。
        this.session.attach_writer(this.originalBytes.slice());
        this.writerAttached = true;
      }
      const saved = JSON.parse(this.session.flush_annotations()) as {
        bytes_appended: number;
        prefix_bytes: number;
      };
      const product = this.session.product_bytes();
      return {
        ok: true,
        bytes_appended: saved.bytes_appended,
        prefix_bytes: saved.prefix_bytes,
        bytes: bytesToBase64(product),
        len: product.length,
      };
    } catch (e) {
      const fail = wasmFail(e);
      this.showStatus(`save rejected (${fail.code}): ${fail.message}`, 'error');
      return fail;
    }
  }

  private async save(): Promise<void> {
    const r = this.flush();
    if (!r.ok) return;
    downloadBytes(base64ToBytes(r.bytes), 'application/pdf', 'annotated.pdf');
    this.showStatus(
      `saved: +${r.bytes_appended} bytes appended (prefix ${r.prefix_bytes}, product ${r.len} bytes) — incremental update`,
    );
  }

  // -----------------------------------------------------------------------
  // 状态行
  // -----------------------------------------------------------------------

  private applySecurityStatus(): void {
    this.statusEl.replaceChildren();
    const base =
      this.doc !== null
        ? this.session
          ? `open: ${this.doc.numPages} page(s), session active`
          : `open: ${this.doc.numPages} page(s)`
        : '';
    if (base) {
      const span = document.createElement('span');
      span.textContent = base;
      this.statusEl.appendChild(span);
    }
    if (this.assessment.encrypted) {
      const span = document.createElement('span');
      span.className = 'pdf-warn-line';
      span.textContent = t('pdf.readOnlyEncrypted');
      this.statusEl.appendChild(span);
    }
    for (const w of this.assessment.warnings) {
      const span = document.createElement('span');
      span.className = 'pdf-warn-line';
      span.textContent = `warning [${w.family}]: ${w.detail}`;
      this.statusEl.appendChild(span);
    }
    if (this.sessionInitFail) {
      const span = document.createElement('span');
      span.className = 'pdf-error';
      span.textContent = `session init failed (${this.sessionInitFail.code}): ${this.sessionInitFail.message} — display-only`;
      this.statusEl.appendChild(span);
    }
  }

  private showStatus(text: string, kind: 'info' | 'error' | 'warn' = 'info'): void {
    const parts = [text];
    if (this.assessment.encrypted) parts.push('read-only mode (encrypted)');
    this.statusEl.replaceChildren();
    const span = document.createElement('span');
    span.textContent = parts.join(' · ');
    if (kind === 'error') span.className = 'pdf-error';
    else if (kind === 'warn') span.className = 'pdf-warn-line';
    this.statusEl.appendChild(span);
  }

  private syncButtons(): void {
    this.saveBtn.disabled = this.doc === null || this.session === null;
    // #240 层③：打印只需显示面（display-only 降级也可打印——位图
    // 即内容，不依赖 wasm 会话）。
    const printBtn = document.getElementById('pdf-print-btn') as HTMLButtonElement | null;
    if (printBtn) printBtn.disabled = this.doc === null;
  }

  // -----------------------------------------------------------------------
  // window 探针面（测试/宿主）
  // -----------------------------------------------------------------------

  private installProbes(): void {
    window.__pdfReady = false; // setupPdfView 成功后置 true
    window.__pdfState = () => ({
      open: this.doc !== null,
      wasm: this.sessionCtor !== null,
      pages: this.doc ? this.doc.numPages : 0,
      scale: this.scale,
      encrypted: this.assessment.encrypted,
      warnings: this.assessment.warnings.map((w) => `${w.family}: ${w.detail}`),
      annots: this.annotations.length,
    });
    window.__pdfPage = (n: number) => {
      const st = this.pages[n - 1]; // 1 基
      if (!st) return { rendered: false, cached: false, lastSource: 'none' as const };
      return {
        rendered: st.rendered,
        cached: this.cache.has(this.cacheKey(n - 1)),
        lastSource: st.lastSource,
      };
    };
    window.__pdfSearch = (q: string) => this.searchOnce(q);
    window.__pdfExtractText = async () => {
      if (!this.session) return BRIDGE_ABSENT;
      await this.textInjected;
      const r = this.sessionExec(buildCmdExtractTextAll());
      if (!r.ok) return r;
      if (!('ExtractedText' in r.value)) {
        return { ok: false, code: 'outcome_shape', message: 'unexpected outcome' };
      }
      return { ok: true, value: r.value.ExtractedText.pages };
    };
    window.__pdfAnnots = (filterJson?: string) => {
      if (!this.session) return BRIDGE_ABSENT;
      let filter: AnnotFilterJson = {};
      if (filterJson) {
        try {
          filter = JSON.parse(filterJson) as AnnotFilterJson;
        } catch {
          return { ok: false, code: 'bad_filter_json', message: 'filter JSON parse failed' };
        }
      }
      return this.listAnnotations(filter);
    };
    window.__pdfAddHighlight = (page, rect) => this.addHighlightRect(page, rect);
    // #45：墨迹探针（与 finishInkStroke 同一载荷构造点 inkPayload——
    // UI 选择器状态即探针载荷，真实笔划/预览/探针三面同源）。
    window.__pdfAddInk = (page: number, strokes: PdfPoint[][]):
      PdfOk<{ id: number; annotation: AnnotationJson }> | PdfFail =>
      this.addAnnotation(page, this.inkPayload(strokes));
    window.__pdfFlush = () => this.flush();
    window.__pdfOpenBytes = (b64: string) => this.openBytes(base64ToBytes(b64), 'sample.pdf');
    window.__pdfScale = () => this.scale;
    window.__pdfSelection = () => window.getSelection()?.toString() ?? '';
    window.__pdfJumpLog = () => [...this.jumpLog];
    // #240 层③：打印态面探针（CSS 判据 + 渲染完备度；prepare =
    // 打印按钮同一路径，不调 window.print()——测试/宿主可单独驱动）。
    window.__pdfPrintInfo = () => ({
      pdfActive: document.body.dataset.pdfActive !== undefined,
      pages: this.pages.length,
      rendered: this.pages.filter((st) => st.rendered).length,
    });
    window.__pdfPrintPrepare = async () => {
      if (!this.doc) return 0;
      return this.renderAllPages();
    };
    // #475：注释列表面探针（行数/页序/开态——e2e 断言面）。
    window.__pdfAnnotsUi = () => ({
      count: this.annotations.length,
      ids: this.annotations.map((a) => a.id),
      pages: this.annotations.map((a) => a.page),
      drawerOpen: this.annotsDrawer !== null && !this.annotsDrawer.hidden,
    });

    // #466：链接面探针（逐页内/外链计数 + 策略拒绝面——e2e 断言面）。
    window.__pdfLinks = () => ({
      perPage: this.pageLinks.map((links) => ({
        total: links.length,
        internal: links.filter((l) => l.target !== null).length,
        external: links.filter((l) => l.url !== null).length,
      })),
      policySkipped: [...this.linkPolicySkipped],
    });

    // #472：属性面探针（DocProps 镜像——字段级断言 + 开态）。
    window.__pdfProps = () => ({
      open: this.docProps !== null,
      props: this.docProps
        ? {
            title: this.docProps.title,
            author: this.docProps.author,
            subject: this.docProps.subject,
            keywords: this.docProps.keywords,
            producer: this.docProps.producer,
            creator: this.docProps.creator,
            creationDate: this.docProps.creationDate,
            modDate: this.docProps.modDate,
            pdfVersion: this.docProps.pdfVersion,
            fileName: this.docProps.fileName,
            pageCount: this.docProps.pageCount,
            fileSize: this.docProps.fileSize,
            encrypted: this.docProps.encrypted,
            warnings: this.docProps.warnings,
          }
        : null,
      drawerOpen: this.propsDrawer !== null && !this.propsDrawer.hidden,
    });


    // #463：大纲面探针（先序平铺 = 树 DOM 同一序——文档序单一出口纪律）。
    window.__pdfOutline = () => {
      const flat = flattenOutline(this.outlineTree);
      return {
        present: flat.length > 0,
        count: flat.length,
        titles: flat.map((n) => n.title),
        skipped: this.outlineSkipped,
        drawerOpen: this.outlineDrawer !== null && !this.outlineDrawer.hidden,
      };
    };
    // #463：会话模型面探针（Outline 命令——ctor 注入实数据的回读验证；
    // 与显示面探针同构的先序平铺口径）。
    window.__pdfOutlineSession = () => {
      if (!this.session) return BRIDGE_ABSENT;
      const r = this.sessionExec(
        JSON.stringify({ Outline: { source: OP_SOURCE } }),
      );
      if (!r.ok) return r;
      if (!('Outline' in r.value)) {
        return {
          ok: false,
          code: 'outcome_shape',
          message: 'unexpected outcome',
        };
      }
      const tree = (r.value as { Outline: { roots: OutlineNodeJson[] } }).Outline;
      const flat = flattenOutline(tree.roots ?? []);
      return { ok: true, value: { count: flat.length, titles: flat.map((n) => n.title) } };
    };
    // 证据跳转钩子（evidence_panel 的 evidence-jump 事件——后续阶段接真
    // 数据；本阶段记录 + 状态行提示）。
    document.addEventListener('evidence-jump', (e) => {
      const detail = (e as CustomEvent).detail as { paragraph_index?: number } | undefined;
      const idx = typeof detail?.paragraph_index === 'number' ? detail.paragraph_index : -1;
      this.jumpLog.push(idx);
      this.showStatus(`evidence jump: paragraph ${idx} (PDF region hook — later stage)`);
    });
  }
}

function clampNum(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

// ---------------------------------------------------------------------------
// 接线（main.ts 调用——pdfjs + pkg_pdf 双懒加载）
// ---------------------------------------------------------------------------

/** pdf.worker 静态资产路径（build.mjs 显式 copyFile 到 dist/——变量形式
 * 避免 esbuild 的 new URL 字面量资产分析，运行时相对 dist/main.js 解析）。 */
const PDF_WORKER_PATH = './pdf.worker.min.mjs';
/** pkg_pdf wasm 胶水路径（同上——运行时相对 dist/main.js 解析；build.mjs
 * 存在性检查后整目录拷贝；缺席 = display-only 降级）。 */
const PDF_WASM_PATH = './pkg_pdf/pdf_wasm.js';

/**
 * 启动 PDF 面板。pdfjs（打包进 main.js）与 pkg_pdf（独立 wasm 包——
 * mdc-pdf-wasm 构建产物，缺席时 display-only 降级）并行解析；两侧均
 * 尝试完成后 __pdfReady = true。任何失败都不向顶层抛出。
 */
export function setupPdfView(): void {
  void (async (): Promise<void> => {
    try {
      const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs');
      pdfjs.GlobalWorkerOptions.workerSrc = new URL(PDF_WORKER_PATH, import.meta.url).href;
      let pdfWasm: PdfWasmModule | null = null;
      try {
        pdfWasm = (await import(new URL(PDF_WASM_PATH, import.meta.url).href)) as PdfWasmModule;
        if (typeof pdfWasm.default === 'function') {
          await wasmBoot(pdfWasm); // 单飞门（wasm-bindgen web-target 异步初始化——pdf_wasm 独立 glue 门）
        }
        // hook 在异步初始化之后接线（wasm 导出面就绪）。
        const initFn = (pdfWasm as unknown as { pdf_wasm_init?: () => void }).pdf_wasm_init;
        if (typeof initFn === 'function') initFn(); // panic → console（wasm trap 定位面）
      } catch {
        pdfWasm = null; // pkg_pdf 缺席——display-only（e2e SKIP 门）
      }
      const ui = new PdfViewUI(pdfjs, pdfWasm);
      window.__pdfReady = true;
      // `ui` 由 window 探针闭包持有——无 GC 顾虑
      void ui;
    } catch (e) {
      const panel = document.getElementById('pdf-panel');
      if (panel) panel.textContent = `pdf panel boot failed: ${String(e)}`;
    }
  })();
}

/** #43：批注模式名 → 词典文本（status 行与下拉同源——en 恒为裸模式名）。
 * 模块级纯函数（无 this 依赖；类体内引用前置声明提升可用）。 */
function modeLabel(mode: string): string {
  switch (mode) {
    case 'note': return t('pdf.modeNote');
    case 'freetext': return t('pdf.modeFreetext');
    case 'ink': return t('pdf.modeInk');
    default: return mode;
  }
}
