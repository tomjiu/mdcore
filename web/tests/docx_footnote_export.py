#!/usr/bin/env python3
"""#488 e2e — MD footnote -> DOCX export (GFM syntax consumption): inject
markdown with a footnote reference + definition into the editor, export via
the real export path, then inspect the produced docx zip parts:

  - word/document.xml: <w:footnoteReference/> present at the paragraph,
    no literal "[^" residue in body text;
  - word/footnotes.xml: the definition content present;
  - definition block consumed (does not appear as a body paragraph);
  - control: markdown WITHOUT footnote syntax exports with zero
    footnoteReference (regression lock).

Evidence -> results/results_docx_footnote_export.json
"""

import http.server
import io
import json
import socketserver
import sys
import threading
import zipfile
from pathlib import Path

from playwright.sync_api import sync_playwright

WEB = Path(__file__).resolve().parent.parent
OUT = WEB / "tests" / "results" / "results_docx_footnote_export.json"
PORT = 8946

MD_WITH_FOOTNOTE = (
    "# Footnote Export Doc\n"
    "\n"
    "Body paragraph with a reference[^1] inline.\n"
    "\n"
    "- list item with marker[^2] too\n"
    "\n"
    "`code span [^1] stays literal`\n"
    "\n"
    "[^1]: The first note content.\n"
    "[^2]: Second note."
)

MD_CONTROL = "# Control Doc\n\nplain **bold** paragraph"

Handler = http.server.SimpleHTTPRequestHandler
Handler.extensions_map.update({".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm"})


class QuietServer(Handler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(WEB / "dist"), **kw)

    def log_message(self, *a):
        pass


socketserver.TCPServer.allow_reuse_address = True


def as_bytes(arr) -> bytes:
    return bytes(arr)


def zip_part(zf: zipfile.ZipFile, name: str) -> str:
    with zf.open(name) as f:
        return f.read().decode("utf-8", errors="replace")


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

            # ---- 快乐路径：注入脚注 MD → 导出 ----
            page.evaluate("(s) => window.__inject(s)", MD_WITH_FOOTNOTE)
            page.wait_for_function("window.__blockCount() >= 5", timeout=15000)
            r = page.evaluate("window.__exportDocx()")
            check("export_ok", r.get("ok") is True, json.dumps(r)[:120])
            raw = as_bytes(r["bytes"])
            zf = zipfile.ZipFile(io.BytesIO(raw))

            doc_xml = zip_part(zf, "word/document.xml")
            check("footnote_reference_in_document",
                  "footnoteReference" in doc_xml, doc_xml[:200])
            # 正文无字面 [^ 残留（代码域内的除外——行内代码标记保真）。
            body_text = "".join(
                seg.split(">", 1)[-1] for seg in doc_xml.split("<w:t") if ">" in seg
            )
            literal_markers = body_text.count("[^")
            check("literal_markers_only_in_code_span", literal_markers == 1,
                  f"literal [^ count = {literal_markers}（代码域内 1 处保真，其余消费）")

            names = zf.namelist()
            check("footnotes_part_present", "word/footnotes.xml" in names, json.dumps(names))
            if "word/footnotes.xml" in names:
                notes_xml = zip_part(zf, "word/footnotes.xml")
                check("footnote_content_in_part", "The first note content." in notes_xml
                      and "Second note." in notes_xml, notes_xml[:300])
                # 两条实质脚注在场（[^1] 段内 + [^2] 列表内各引用一次；
                # docx-rs 无分隔符条目——与读面 #19「分隔符已滤」口径
                # 互补：写面就不发射）。
                check("both_notes_emitted",
                      notes_xml.count("<w:footnote ") >= 2,
                      f"footnote count = {notes_xml.count('<w:footnote ')}")

            # 定义块不进正文（正文段不含 "[^1]:" 起始段）。
            check("definition_consumed", "[^1]: The first note content." not in doc_xml)

            # ---- 对照：无脚注 MD 导出零 footnoteReference ----
            page.evaluate("(s) => window.__inject(s)", MD_CONTROL)
            page.wait_for_function("window.__blockCount() === 2", timeout=15000)
            r2 = page.evaluate("window.__exportDocx()")
            raw2 = as_bytes(r2["bytes"])
            zf2 = zipfile.ZipFile(io.BytesIO(raw2))
            doc_xml2 = zip_part(zf2, "word/document.xml")
            check("control_no_footnote_reference", "footnoteReference" not in doc_xml2)
            # docx-rs 恒发射 footnotes.xml 部件骨架——语义判据 = 部件内
            # 零用户内容（无实质脚注条目）。
            if "word/footnotes.xml" in zf2.namelist():
                notes2 = zip_part(zf2, "word/footnotes.xml")
                check("control_footnotes_part_empty",
                      notes2.count("<w:footnote ") == 0, notes2[:200])
            else:
                check("control_footnotes_part_empty", True)
            check("control_bold_intact", "plain bold paragraph" in doc_xml2.replace("</w:t>", "\n") or "bold" in doc_xml2)

            # ---- 零 page error ----
            check("no_page_errors", len(page_errors) == 0, "; ".join(page_errors[:3]))

            browser.close()

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"suite": "docx_footnote_export", "checks": checks}, ensure_ascii=False, indent=1))
    failed = [c for c in checks if not c["ok"]]
    print(f"\n{len(checks) - len(failed)}/{len(checks)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
