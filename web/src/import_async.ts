// #479 — 异步导入客户端（主线程侧）。
//
// 设计源：docs/async-import-design.md §3 方案 A + #47（评估轮）。
// 与 web/src/import-worker.ts（worker 侧入口）成对——协议单一真源：
//
//   worker → 主线程  { type: 'ready' }
//   主线程 → worker  { type: 'import', format: 'docx'|'xlsx', bytes }
//                    （bytes 为**拷贝**非 transfer——设计文档原案 transferable
//                    零拷贝，但 #42 XLSX 联动面（gridOpenBytesWhenReady）
//                    需要主线程保留同一份 bytes 灌 Grid 会话；50MB 档
//                    slice 拷贝 ~数十 ms，远小于导入本体耗时，登记偏离理由）
//   worker → 主线程  { type: 'result', ok:true, markdown, stats }
//                  | { type: 'result', ok:false, code, message }
//
// 生命周期纪律（设计文档 §3 内存面）：
// - **单次导入单 worker**：每次 asyncImport 新建 worker，finally 里
//   terminate——大档 wasm 峰值内存（50MB 档 ~888MB）导入毕即释放；
// - **取消/顶替 = terminate**：在途导入被新导入顶替或显式取消时直接
//   terminate worker，等待方收到 {ok:false, code:'import_cancelled'}——
//   terminate 后不会再有任何 result 消息抵达（无晚到竞态）；
// - **回落判据**（infra 错误码三枚，与引擎业务错误码不相交——业务
//   失败按 result 原样上抛，不回落）：worker_init_failed /
//   worker_timeout（ready 握手超时）/ worker_error（构造失败/协议异常）。
//   调用方（main.ts 导入 handler）对 infra 码回落同步路径
//   （window.__importDocx / __importXlsx——CONTRACT 同步桥恒保留）。

/** worker 基础设施失败码（回落同步路径的判据——见模块文档）。
 * 注意：`import_cancelled` **不在**此集合——取消是调用方意图，不是
 * 基础设施失败；若取消边回落同步重导入，则取消语义自我矛盾
 * （用户按 Esc = 不想导入，而不是「换同步再导一次」）。 */
export const WORKER_INFRA_CODES: ReadonlySet<string> = new Set([
  'worker_init_failed',
  'worker_timeout',
  'worker_error',
]);

/** 导入结果（与同步桥 window.__importDocx 的返回形状同构——
 * 调用方用同一应用面，零第二真相）。S = stats 的调用方侧形状
 *（默认 unknown；消息边界后由调用方断言——与同步路径 JSON.parse
 * 的 as 断言同一信任级）。 */
export type AsyncImportResult<S = unknown> =
  | { ok: true; markdown: string; stats: S }
  | { ok: false; code: string; message: string };

/** ready 握手默认超时：worker init = fetch wasm（浏览器 HTTP 缓存内）
 * + 实例化。冷启动最坏面（首次无缓存 + 慢盘）给 15s。 */
const READY_TIMEOUT_MS = 15_000;

/** 在途请求的记账（单一在途——新导入顶替旧导入）。settle 取
 * AsyncImportResult<never>：never 可赋给任意 S（协变收口——失败分支
 * 无 stats，成功分支不经过 teardown）。 */
interface Inflight {
  worker: Worker;
  settle: (r: AsyncImportResult<never>) => void;
  done: boolean;
}

let inflight: Inflight | null = null;

/** 终结在途请求（顶替/取消共用）：terminate + 回 import_cancelled。
 * 已 done（result 已抵达）则只清理引用。 */
function teardown(reason: 'superseded' | 'cancelled' | 'settled'): void {
  const cur = inflight;
  inflight = null;
  if (!cur) return;
  if (!cur.done) {
    cur.done = true;
    cur.worker.terminate();
    if (reason !== 'settled') {
      cur.settle({
        ok: false,
        code: 'import_cancelled',
        message: reason === 'superseded' ? 'superseded by a newer import' : 'cancelled',
      });
    }
  } else {
    // result 已回、仅 terminate 未做（正常路径也在 finally terminate——
    // 这里是防御性收口，不重复 settle）。
    cur.worker.terminate();
  }
}

/** 新建导入 worker。构造失败 = 环境不支持 module worker（旧浏览器/
 * CSP 限制）——调用方回落同步路径。 */
function spawnWorker(): Worker {
  return new Worker(new URL('./import-worker.js', import.meta.url), { type: 'module' });
}

/**
 * 经专职 worker 导入（docx/xlsx）。
 *
 * - 任何时刻至多一个在途导入：新调用先顶替（terminate）旧在途；
 * - infra 失败（WORKER_INFRA_CODES）回 {ok:false, code:infra 码}，
 *   调用方回落同步路径；
 * - 引擎业务失败（容器拒/解析失败）原样回传——与同步路径同一错误面。
 */
export function asyncImport<S = unknown>(format: 'docx' | 'xlsx', bytes: Uint8Array): Promise<AsyncImportResult<S>> {
  teardown('superseded'); // 单在途纪律：新导入顶替旧导入
  let worker: Worker;
  try {
    worker = spawnWorker();
  } catch (e: unknown) {
    return Promise.resolve({
      ok: false,
      code: 'worker_error',
      message: `worker construction failed: ${String(e)}`,
    });
  }
  return new Promise<AsyncImportResult<S>>((resolve) => {
    const record: Inflight = { worker, settle: resolve, done: false };
    inflight = record;
    const infra = (code: 'worker_timeout' | 'worker_error', message: string): void => {
      if (record.done) return;
      record.done = true;
      worker.terminate();
      if (inflight === record) inflight = null;
      resolve({ ok: false, code, message });
    };
    const readyTimer = window.setTimeout(() => {
      infra('worker_timeout', `worker not ready within ${READY_TIMEOUT_MS}ms`);
    }, READY_TIMEOUT_MS);
    worker.onmessage = (ev: MessageEvent) => {
      const data = ev.data as { type?: string } & AsyncImportResult;
      if (data?.type === 'ready') {
        window.clearTimeout(readyTimer);
        return; // 握手完成——等待 result
      }
      if (data?.type === 'result') {
        if (record.done) return;
        record.done = true;
        window.clearTimeout(readyTimer);
        worker.terminate();
        if (inflight === record) inflight = null;
        resolve(
          data.ok
            ? { ok: true, markdown: data.markdown, stats: data.stats as S }
            : { ok: false, code: data.code, message: data.message },
        );
        return;
      }
      // 未知消息 = 协议面异常（worker 产物与客户端版本不匹配等）。
      infra('worker_error', `unexpected worker message: ${JSON.stringify(data)?.slice(0, 200)}`);
    };
    worker.onerror = (e: ErrorEvent) => {
      window.clearTimeout(readyTimer);
      infra('worker_error', `worker error: ${e.message || 'unknown'}`);
    };
    worker.onmessageerror = () => {
      window.clearTimeout(readyTimer);
      infra('worker_error', 'worker messageerror (unserializable payload)');
    };
    // bytes 拷贝（非 transfer）——理由见模块文档（#42 Grid 联动保留主线程侧）。
    worker.postMessage({ type: 'import', format, bytes: bytes.slice().buffer });
  });
}

/** 显式取消在途导入（测试面/键盘 Esc 共用）。无在途 = false。 */
export function cancelAsyncImport(): boolean {
  if (!inflight) return false;
  teardown('cancelled');
  return true;
}

/** 是否有在途导入（状态行/调试面）。 */
export function asyncImportBusy(): boolean {
  return inflight !== null;
}
