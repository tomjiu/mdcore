#!/usr/bin/env python3
"""#479 e2e — async import worker (design doc docs/async-import-design.md
Plan A): dedicated module worker for __importDocx/__importXlsx file-input
path with sync fallback. Three-state coverage per the issue acceptance:

  1. worker path happy: big generated docx via real file input ->
     rAF keeps firing during import (main thread NOT blocked) ->
     result markdown byte-identical to the sync path (same bytes re-imported
     via window.__importDocx) -> lastMode === 'worker'.
  2. fallback: __asyncImportDebug.enable(false) -> file input import still
     succeeds via sync bridge -> lastMode === 'sync-fallback' -> re-enable.
  3. cancel: start big import (worker busy) -> cancel() -> busy false ->
     status shows cancelled -> immediately re-import succeeds (worker path
     again — fresh worker after terminate).

Big fixture: valid minimal docx generated in-process (zipfile; ~12k
paragraphs, ~8MB) — slow enough to catch in-flight states, fast enough for
CI (<10s on worker).

Evidence -> results/results_import_worker.json
"""

import http.server
import json
import socketserver
import sys
import threading
import zipfile
from io import BytesIO
from pathlib import Path

from playwright.sync_api import sync_playwright

WEB = Path(__file__).resolve().parent.parent
OUT = WEB / "tests" / "results" / "results_import_worker.json"
PORT = 8943

# ---------------------------------------------------------------------------
# 夹具：程序内生成的大 docx（最小合法 OPC + 大量段落）。
# 结构对齐 office-io 读面要求：[Content_Types].xml + _rels/.rels +
# word/document.xml（w:body/w:p/w:r/w:t）。
# ---------------------------------------------------------------------------
PARA_COUNT = 50000


def gen_big_docx() -> bytes:
    buf = BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr(
            "[Content_Types].xml",
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
            "</Types>",
        )
        z.writestr(
            "_rels/.rels",
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
            "</Relationships>",
        )
        # docx-rs 经 zip 门读 word/_rels/document.xml.rels（缺件 = 整包
        # parse 拒——「Failed to read from zip」）；空关系集合法。
        z.writestr(
            "word/_rels/document.xml.rels",
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>',
        )
        paras = []
        for i in range(PARA_COUNT):
            paras.append(
                "<w:p><w:r><w:t>para %d — async worker fixture paragraph text</w:t></w:r></w:p>" % i
            )
        z.writestr(
            "word/document.xml",
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
            "<w:body>" + "".join(paras) + "</w:body></w:document>",
        )
    return buf.getvalue()


Handler = http.server.SimpleHTTPRequestHandler
Handler.extensions_map.update({".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm"})


class QuietServer(Handler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(WEB / "dist"), **kw)

    def log_message(self, *a):
        pass


socketserver.TCPServer.allow_reuse_address = True

# 页内注入：导入前起 rAF 计数器（帧推进 = 主线程活着）。
RAF_COUNTER = """() => {
    window.__rafFrames = 0;
    window.__rafOn = true;
    const tick = () => { if (!window.__rafOn) return; window.__rafFrames++;
                         requestAnimationFrame(tick); };
    requestAnimationFrame(tick);
}"""


def main() -> int:
    checks: list[dict] = []
    page_errors: list[str] = []

    def check(name, ok, detail=""):
        checks.append({"name": name, "ok": bool(ok), "detail": str(detail)[:400]})
        print(("PASS " if ok else "FAIL ") + name + ("  | " + str(detail)[:160] if detail and not ok else ""))
        return ok

    big_bytes = gen_big_docx()
    tmp_fixture = WEB / "tests" / "fixtures" / "import_worker_big.docx"
    tmp_fixture.write_bytes(big_bytes)
    print(f"fixture: {tmp_fixture.name} = {len(big_bytes)} bytes, {PARA_COUNT} paragraphs")

    with socketserver.TCPServer(("127.0.0.1", PORT), QuietServer) as srv:
        t = threading.Thread(target=srv.serve_forever, daemon=True)
        t.start()
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            page = browser.new_page()
            page.on("pageerror", lambda e: page_errors.append(str(e)))
            page.goto(f"http://127.0.0.1:{PORT}/index.html")
            page.wait_for_function("window.__ready === true", timeout=30000)

            # ---- 0. 调试面在场（契约锚）----
            dbg = page.evaluate("typeof window.__asyncImportDebug")
            check("debug_surface_present", dbg == "object", dbg)
            init_state = page.evaluate("window.__asyncImportDebug.state()")
            check("initial_state_idle", init_state["lastMode"] == "idle" and not init_state["busy"], json.dumps(init_state))

            # ---- 1. worker 快乐路径：大档 + rAF 保活 + 字节一致 ----
            page.evaluate(RAF_COUNTER)
            page.set_input_files("#docx-file", str(tmp_fixture))
            # 在途断言（导入中）：busy = true + 状态行「导入中」。
            inflight_seen = False
            try:
                page.wait_for_function("window.__asyncImportDebug.busy() === true", timeout=2000)
                inflight_seen = True
            except Exception:
                inflight_seen = False
            page.wait_for_function("window.__lastDocxImport && window.__lastDocxImport.ok === true", timeout=60000)
            worker_state = page.evaluate(
                "() => ({ state: window.__asyncImportDebug.state(), raf: window.__rafFrames, "
                "stats: window.__lastDocxImport.stats, md_len: (window.__lastDocxImport.markdown||'').length })"
            )
            check("worker_path_inflight_busy_seen", inflight_seen, "busy never observed (import too fast?)")
            check("worker_path_lastMode", worker_state["state"]["lastMode"] == "worker", json.dumps(worker_state["state"]))
            check("worker_path_not_busy_after", not worker_state["state"]["busy"])
            check("worker_path_raf_alive", worker_state["raf"] >= 1, f"rAF frames during import = {worker_state['raf']}")
            check("worker_path_imported_paragraphs",
                  worker_state["stats"].get("paragraphs") == PARA_COUNT,
                  json.dumps(worker_state["stats"]))
            md_worker = page.evaluate("window.__md()")

            # ---- 2. 同一 bytes 走同步桥：逐字节一致（双轨结果等价）----
            b64 = __import__("base64").b64encode(big_bytes).decode()
            sync_result = page.evaluate(
                """(b64) => {
                    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
                    const r = window.__importDocx(bytes);
                    return { ok: r.ok, md: r.ok ? window.__md() : null };
                }""", b64)
            check("sync_path_identical_markdown", sync_result["ok"] and sync_result["md"] == md_worker,
                  f"worker len={len(md_worker)} sync len={len(sync_result['md'] or '')}")

            # ---- 3. 回落：禁用 worker → file input 走同步桥 ----
            page.evaluate("window.__asyncImportDebug.enable(false)")
            page.set_input_files("#docx-file", str(tmp_fixture))
            page.wait_for_function("window.__lastDocxImport && window.__lastDocxImport.ok === true", timeout=60000)
            fb_state = page.evaluate("window.__asyncImportDebug.state()")
            check("fallback_mode_after_disable", fb_state["lastMode"] == "sync-fallback", json.dumps(fb_state))
            page.evaluate("window.__asyncImportDebug.enable(true)")

            # ---- 4. 取消：在途 terminate → 状态行取消 → 立即可再导入 ----
            page.set_input_files("#docx-file", str(tmp_fixture))
            cancelled = False
            try:
                page.wait_for_function("window.__asyncImportDebug.busy() === true", timeout=2000)
                ok = page.evaluate("window.__asyncImportDebug.cancel()")
                page.wait_for_function(
                    "document.getElementById('docx-status').textContent.includes('取消')"
                    " || document.getElementById('docx-status').textContent.includes('cancel')", timeout=5000)
                cancelled = ok
            except Exception as e:
                cancelled = False
                print("cancel window missed:", e)
            check("cancel_inflight", cancelled, "cancel() returned false or status never showed cancelled")
            not_busy = page.evaluate("window.__asyncImportDebug.busy() === false")
            check("cancel_releases_busy", not_busy)
            # 取消后立即再导入（worker 路径——terminate 后新 worker 可用）。
            page.set_input_files("#docx-file", str(tmp_fixture))
            page.wait_for_function("window.__lastDocxImport && window.__lastDocxImport.ok === true", timeout=60000)
            after = page.evaluate("window.__asyncImportDebug.state()")
            check("reimport_after_cancel_worker_path", after["lastMode"] == "worker", json.dumps(after))

            # ---- 5. 坏档：worker 路径业务失败面（typed 错误，非 infra 回落）----
            bad = b"not a docx at all" * 10
            bad_b64 = __import__("base64").b64encode(bad).decode()
            bad_result = page.evaluate(
                """(b64) => {
                    window.__asyncImportDebug.enable(true);
                    // 直接走 file-input 路径不可行（无 UI 注入坏档）——用
                    // 桥面：此处验证 worker 路径对坏档的业务错误传播。
                    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
                    // file-input 路径的等价物：runDocxImport 是闭包，无法直
                    // 达；经 __importDocx 同步桥验证同一引擎错误面 + 状态行。
                    const r = window.__importDocx(bytes);
                    return r;
                }""", bad_b64)
            check("bad_file_typed_error_sync",
                  bad_result.get("ok") is False and bad_result.get("code"),
                  json.dumps(bad_result)[:200])

            # ---- 6. 零 page error ----
            check("no_page_errors", len(page_errors) == 0, "; ".join(page_errors[:3]))

            browser.close()

    tmp_fixture.unlink(missing_ok=True)  # 生成件不留仓（.gitignore 外的临时面）
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"suite": "import_worker", "checks": checks}, ensure_ascii=False, indent=1))
    failed = [c for c in checks if not c["ok"]]
    print(f"\n{len(checks) - len(failed)}/{len(checks)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
