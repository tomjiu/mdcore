#!/usr/bin/env python3
"""#485 e2e — DOCX footnote import (GFM footnote syntax): a real DOCX with
a footnote (written by office-io RunSpec::footnote via the fixture
generator) imports through the real file-input path -> the editor markdown
carries the `[^1]` reference marker AND the `[^1]: content` definition
block; stats surface footnotes/footnote_refs; status line shows the
footnote count; no-footnote docs (import_sample.docx control) keep their
existing exact output (zero-regression lock).

Fixture = web/tests/fixtures/import_footnote.docx (provenance: office-io
DocxWriter via `cargo run -p mdcore-wasm --example gen_docx_footnote_fixture`).
Evidence -> results/results_docx_footnote.json
"""

import http.server
import json
import socketserver
import sys
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

WEB = Path(__file__).resolve().parent.parent
FIXTURE = WEB / "tests" / "fixtures" / "import_footnote.docx"
CONTROL = WEB / "tests" / "fixtures" / "import_sample.docx"
OUT = WEB / "tests" / "results" / "results_docx_footnote.json"
PORT = 8945

Handler = http.server.SimpleHTTPRequestHandler
Handler.extensions_map.update({".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm"})


class QuietServer(Handler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(WEB / "dist"), **kw)

    def log_message(self, *a):
        pass


socketserver.TCPServer.allow_reuse_address = True

# 对照组（无脚注）：docx_import.py 的 EXPECTED_MD 逐字节面——此处抽查
# 其关键片段（标题/表格/列表/页眉页脚）以锁「脚注分支零变化」。
CONTROL_EXPECTED_SNIPPETS = [
    "<!-- mdcore-import: header:default -->",
    "# Imported Title",
    "| Name | Value |",
    "1. First step",
    "- Bullet one",
    "<!-- mdcore-import: footer:default -->",
]


def main() -> int:
    checks: list[dict] = []
    page_errors: list[str] = []

    def check(name, ok, detail=""):
        checks.append({"name": name, "ok": bool(ok), "detail": str(detail)[:400]})
        print(("PASS " if ok else "FAIL ") + name + ("  | " + str(detail)[:160] if detail and not ok else ""))
        return ok

    with socketserver.TCPServer(("127.0.0.1", PORT), QuietServer) as srv:
        t = threading.Thread(target=srv.serve_forever, daemon=True)
        t.start()
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            page = browser.new_page()
            page.on("pageerror", lambda e: page_errors.append(str(e)))
            page.goto(f"http://127.0.0.1:{PORT}/index.html")
            page.wait_for_function("window.__ready === true", timeout=30000)

            # ---- 1. 快乐路径：脚注夹具经真实 file input ----
            page.set_input_files("#docx-file", str(FIXTURE))
            page.wait_for_function("window.__lastDocxImport && window.__lastDocxImport.ok === true",
                                   timeout=30000)
            md = page.evaluate("window.__md()")
            check("footnote_ref_marker_in_md", "Body text with a footnote [^1]" in md, repr(md[:160]))
            check("footnote_definition_in_md", "[^1]: Fixture footnote content." in md, repr(md[-160:]))
            check("footnote_body_intact", "# Footnote Fixture" in md and "Trailing paragraph" in md, repr(md[:80]))
            # 定义块成为真实段落块（mdcore 行级解析承载——块序最后）。
            blocks = page.evaluate("window.__blocks().map((b) => b.type + '|' + (b.text || '').slice(0, 40))")
            check("footnote_definition_is_block",
                  any(b.startswith("paragraph|[^1]:") for b in blocks), json.dumps(blocks))

            # ---- 2. stats 面 ----
            stats = page.evaluate("window.__lastDocxImport.stats")
            check("footnote_stats",
                  stats.get("footnotes") == 1 and stats.get("footnote_refs") == 1
                  and stats.get("footnote_blocks_dropped") == 0,
                  json.dumps(stats))

            # ---- 3. 状态行可见性（脚注引用数）----
            status_text = page.evaluate("document.getElementById('docx-status').textContent")
            check("footnote_status_line", "1 个脚注引用" in status_text or "footnote ref" in status_text,
                  status_text)

            # ---- 4. 对照组：无脚注文档输出零变化（回归锁）----
            page.set_input_files("#docx-file", str(CONTROL))
            page.wait_for_function(
                "window.__lastDocxImport && window.__lastDocxImport.ok === true"
                " && window.__md().includes('Imported Title')", timeout=30000)
            c_md = page.evaluate("window.__md()")
            snippets_ok = all(s in c_md for s in CONTROL_EXPECTED_SNIPPETS)
            no_footnote = "[^" not in c_md
            check("control_doc_no_footnote_output", snippets_ok and no_footnote, repr(c_md[:120]))
            c_stats = page.evaluate("window.__lastDocxImport.stats")
            check("control_stats_zero_footnotes",
                  c_stats.get("footnote_refs") == 0 and c_stats.get("footnotes") == 0,
                  json.dumps({k: c_stats.get(k) for k in ("footnotes", "footnote_refs")}))

            # ---- 5. 零 page error ----
            check("no_page_errors", len(page_errors) == 0, "; ".join(page_errors[:3]))

            browser.close()

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"suite": "docx_footnote", "checks": checks}, ensure_ascii=False, indent=1))
    failed = [c for c in checks if not c["ok"]]
    print(f"\n{len(checks) - len(failed)}/{len(checks)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
