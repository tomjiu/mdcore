// #520: dark-label scan verification face — web host of the #502 wasm
// bridge (`dark_scan` free function, PR #504).
//
// Product story (#161 P0-2 close-out): the anonymize chain produces cleaned
// exports, but 「清理完要验证真清干净」 needs an independent check the user
// can run against ANY artifact file — external tools, older versions, manual
// processing. The verification face does not presume the artifact came from
// this engine (#502 registered host contract).
//
// Entry: toolbar 「暗标扫描」 file input (.docx / .xlsx) → bytes + kind (by
// extension) → wasm `dark_scan(bytes, kind)` → structured residual list
// rendered in the #darkscan-panel popover. Empty list = clean banner.
//
// Degrade discipline (mirrors #512 / xlsx_grid bridge_absent): a pkg build
// without dark_scan (PR #504 pending) yields a typed bridge_absent notice —
// NEVER an empty list posing as "clean" (a fake-clean verdict on a bid
// artifact is worse than an honest "not verified").

type WasmModule = typeof import('../pkg/mdcore_wasm.js');
import { wasmBoot } from './wasm_boot.js';
import { t } from './i18n/index.js';

/** #502 wasm bridge face: bytes + kind -> residual JSON array string.
 * Throws a typed JSON string ({code, message}) on bad container/kind. */
type DarkScanFn = (bytes: Uint8Array, kind: string) => string;

/** serde mirror of office-io IdentityResidual (deterministic part/field/
 * value dictionary order — display and asserts rely on it). */
type Residual = { part: string; field: string; value: string; hint: string };

/** Window probe state (tests / host shell — house __grid* pattern). */
type ScanState = {
  runs: number;
  lastKind: string | null;
  lastCount: number | null;
  lastCode: string | null;
  lastResiduals: Residual[];
};

/** Thrown wasm JSON string -> typed failure (xlsx_grid wasmFail twin). */
function scanFail(e: unknown): { code: string; message: string } {
  const raw = typeof e === 'string' ? e : (e as { message?: string })?.message ?? '';
  try {
    const parsed = JSON.parse(raw) as { code: string; message: string };
    return { code: parsed.code, message: parsed.message };
  } catch {
    return { code: 'unknown', message: raw || String(e) };
  }
}

/** Kind lens by file extension (user-declared — the picker accepts only the
 * two OOXML kinds; a forced foreign extension is a typed reject, no guess). */
function kindForName(name: string): 'docx' | 'xlsx' | null {
  const n = name.toLowerCase();
  if (n.endsWith('.docx')) return 'docx';
  if (n.endsWith('.xlsx')) return 'xlsx';
  return null;
}

export function setupDarkScan(lazyMod: () => Promise<WasmModule>): void {
  const input = document.getElementById('darkscan-file') as HTMLInputElement | null;
  const statusEl = document.getElementById('darkscan-status');
  const panel = document.getElementById('darkscan-panel');
  if (!input || !statusEl || !panel) return; // page without the entry (bench) — inert

  let modPromise: Promise<WasmModule> | null = null;
  const mod = (): Promise<WasmModule> => (modPromise ??= lazyMod());

  const state: ScanState = { runs: 0, lastKind: null, lastCount: null, lastCode: null, lastResiduals: [] };
  window.__darkscanState = (): ScanState => ({
    runs: state.runs,
    lastKind: state.lastKind,
    lastCount: state.lastCount,
    lastCode: state.lastCode,
    lastResiduals: structuredClone(state.lastResiduals),
  });

  const setStatus = (text: string, err: boolean): void => {
    statusEl.textContent = text;
    statusEl.classList.toggle('grid-error', err);
  };

  const showError = (code: string, message: string): void => {
    state.runs += 1;
    state.lastCode = code;
    state.lastCount = null;
    state.lastResiduals = [];
    panel.replaceChildren();
    const head = document.createElement('div');
    head.className = 'ds-head';
    const badge = document.createElement('span');
    badge.className = 'ds-count dirty';
    badge.textContent = t('darkscan.scanFailed', { code, message });
    head.append(badge);
    panel.append(head);
    setStatus(t('darkscan.scanFailed', { code, message }), true);
  };

  const renderResiduals = (kind: string, residuals: Residual[]): void => {
    state.runs += 1;
    state.lastCode = null;
    state.lastCount = residuals.length;
    state.lastResiduals = residuals;
    panel.replaceChildren();

    const head = document.createElement('div');
    head.className = 'ds-head';
    const title = document.createElement('span');
    title.textContent = t('darkscan.title');
    const badge = document.createElement('span');
    badge.className = 'ds-count' + (residuals.length > 0 ? ' dirty' : '');
    badge.textContent = residuals.length > 0
      ? t('darkscan.found', { n: residuals.length })
      : t('darkscan.clean');
    const close = document.createElement('button');
    close.type = 'button';
    close.className = 'ds-close';
    close.textContent = '×';
    close.setAttribute('aria-label', t('darkscan.close'));
    close.addEventListener('click', () => {
      panel.hidden = true;
    });
    head.append(title, badge, close);

    if (residuals.length === 0) {
      const clean = document.createElement('div');
      clean.className = 'ds-clean';
      clean.textContent = t('darkscan.clean');
      panel.append(head, clean);
    } else {
      const list = document.createElement('div');
      for (const r of residuals) {
        const row = document.createElement('div');
        row.className = 'ds-row';
        const where = document.createElement('div');
        where.className = 'ds-where';
        where.textContent = `${r.part} · ${r.field}`;
        const value = document.createElement('div');
        value.className = 'ds-value';
        value.textContent = r.value;
        const hint = document.createElement('div');
        hint.className = 'ds-hint';
        hint.textContent = r.hint;
        row.append(where, value, hint);
        list.append(row);
      }
      panel.append(head, list);
    }
    panel.hidden = false;
    setStatus(
      residuals.length > 0
        ? t('darkscan.found', { n: residuals.length }) + ` · ${kind}`
        : t('darkscan.clean'),
      residuals.length > 0,
    );
  };

  input.addEventListener('change', async () => {
    const file = input.files?.[0];
    input.value = ''; // re-selecting the same file must re-trigger change
    if (!file) return;

    const kind = kindForName(file.name);
    if (!kind) {
      // typed reject — the scan never guesses a lens for a foreign extension
      const msg = t('darkscan.unsupportedKind', { name: file.name });
      showError('unsupported_kind', msg);
      state.lastKind = null;
      return;
    }
    state.lastKind = kind;

    try {
      const m = await mod();
      await wasmBoot(m);
      const face = (m as unknown as { dark_scan?: DarkScanFn }).dark_scan;
      if (typeof face !== 'function') {
        // pkg predates #504 — honest degrade, never a fake-clean verdict
        showError('bridge_absent', t('darkscan.bridgeAbsent'));
        return;
      }
      const bytes = new Uint8Array(await file.arrayBuffer());
      let raw: string;
      try {
        raw = face.call(m, bytes, kind);
      } catch (e) {
        const f = scanFail(e);
        if (f.code === 'unknown' && /not a function/i.test(f.message)) {
          showError('bridge_absent', t('darkscan.bridgeAbsent'));
          return;
        }
        showError(f.code, f.message);
        return;
      }
      let residuals: Residual[];
      try {
        residuals = JSON.parse(raw) as Residual[];
      } catch {
        showError('bad_payload', raw.slice(0, 120));
        return;
      }
      renderResiduals(kind, residuals);
    } catch (e) {
      // wasm boot failure (glue broken / fetch failed) — typed, no throw-out
      showError('boot_failed', typeof e === 'string' ? e : (e as { message?: string })?.message ?? String(e));
    }
  });
}

declare global {
  interface Window {
    __darkscanState?: () => ScanState;
  }
}
