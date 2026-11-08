// #479 — 专职导入 Worker（worker 侧入口）。
//
// 设计源：docs/async-import-design.md §3 方案 A + #47（评估轮交付物）。
// 本文件跑在 module worker 里（esbuild 识别 `new Worker(new URL(...))`
// 入口自动单独打包为 dist/import-worker.js——与 build.mjs 既有约定一致，
// wasm 资产经 glue 的 `new URL('mdcore_wasm_bg.wasm', import.meta.url)`
// 解析到 dist/ 同目录，与 main.js 共用同一份 .wasm）。
//
// 协议（与主线程侧 web/src/import_async.ts 成对——单一真源在彼处注释）：
//   worker → 主线程  { type: 'ready' }                        init 完成
//   主线程 → worker  { type: 'import', format, bytes }        请求导入
//   worker → 主线程  { type: 'result', ok:true, markdown, stats }
//                  | { type: 'result', ok:false, code, message }
//
// 语义要点：
// - wasm 导入链零 DOM 依赖（#47 代码级核查）——本 worker 只做
//   init + 分发 + 回传，不含任何 UI 面；
// - **init 前抵达的导入请求排队**（客户端在 worker 构造后立即 post，
//   与 init 完成存在竞态——wasm 未实例化时调用 import_docx 会抛
//   uninitialized）；ready 之后逐个补跑；
// - 引擎侧 typed 错误（容器拒/解析失败）恒以 result 消息回传
//   （ok:false + code/message），**不**视为 worker 基础设施失败——
//   主线程按业务失败展示，不回落（回落仅限 init/协议层失败）；
// - worker 生命周期 = 单次导入（主线程 finally terminate——大档
//   峰值内存即时释放，设计文档 §3 内存面结论）；取消 = 主线程直接
//   terminate，无需协作消息。

import init, { import_docx, import_xlsx } from '../pkg/mdcore_wasm.js';

interface ImportRequest {
  format: 'docx' | 'xlsx';
  bytes: ArrayBuffer;
}

/** 引擎返回 JSON 的统一解包（与 main.ts docxImport/xlsxImport 同构：
 * 成功 = {markdown, stats}）。 */
function unwrap(raw: string):
  | { ok: true; markdown: string; stats: unknown }
  | { ok: false; code: string; message: string } {
  try {
    const payload = JSON.parse(raw) as { markdown: string; stats: unknown };
    return { ok: true, markdown: payload.markdown, stats: payload.stats };
  } catch {
    return { ok: false, code: 'worker_protocol', message: 'non-JSON payload from engine' };
  }
}

/** wasm 面异常（Rust Err(JsValue::from_str(json)) → JS 串异常）→ typed。 */
function errorOf(e: unknown): { ok: false; code: string; message: string } {
  const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
  try {
    const parsed = JSON.parse(raw) as { code: string; message: string };
    return { ok: false, code: parsed.code, message: parsed.message };
  } catch {
    return { ok: false, code: 'unknown', message: raw || String(e) };
  }
}

function post(msg: unknown): void {
  (self as unknown as Worker).postMessage(msg);
}

/** 执行一次导入并回传 result（同步段——wasm 调用本体阻塞 worker 线程，
 * 主线程不受影响）。 */
function runImport(req: ImportRequest): void {
  try {
    const u8 = new Uint8Array(req.bytes);
    const raw = req.format === 'docx' ? import_docx(u8) : import_xlsx(u8);
    post({ type: 'result', ...unwrap(raw) });
  } catch (e: unknown) {
    post({ type: 'result', ...errorOf(e) });
  }
}

// ---- init 前请求排队（竞态面见模块文档）----
const queue: ImportRequest[] = [];
let wasmReady = false;

self.addEventListener('message', (ev: MessageEvent) => {
  const data = ev.data as { type?: string; format?: string; bytes?: ArrayBuffer };
  if (data?.type !== 'import') return; // 未知消息零动作（协议面恒显式）
  if ((data.format !== 'docx' && data.format !== 'xlsx') || !(data.bytes instanceof ArrayBuffer)) {
    post({
      type: 'result',
      ok: false,
      code: 'worker_protocol',
      message: `bad import request (format=${String(data.format)})`,
    });
    return;
  }
  const req: ImportRequest = { format: data.format, bytes: data.bytes };
  if (!wasmReady) {
    queue.push(req);
    return;
  }
  runImport(req);
});

void init().then(
  () => {
    wasmReady = true;
    post({ type: 'ready' });
    for (const req of queue.splice(0)) runImport(req);
  },
  (e: unknown) => {
    // init 失败：回 result 形 infra 错误（比主线程等 ready 超时快得多）。
    post({
      type: 'result',
      ok: false,
      code: 'worker_init_failed',
      message: `wasm init failed in worker: ${String(e)}`,
    });
  },
);
