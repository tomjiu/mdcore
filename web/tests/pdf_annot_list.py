#!/usr/bin/env python3
"""#475 web PDF 注释列表面板 e2e——AnnotListed 可视化 + 点击跳页。

夹具 = sample.pdf(3 页,程序化加批注:页 1 高亮 + 页 2 高亮 + 页 2 墨迹)。
驱动面:__pdfAddHighlight / __pdfAddInk / __pdfAnnotsUi / __pdfState +
手势级 page.click('#pdf-annots-btn') 与 .pdf-annots-item 行点击。

结果 -> tests/results/results_pdf_annot_list.json
"""

import base64
import http.server
import json
import socketserver
import subprocess
import sys
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

WEB = Path(__file__).resolve().parent.parent
FIXTURE = WEB / "tests" / "fixtures" / "sample.pdf"
GEN = ["python3", "tests/gen_pdf_fixture.py"]
OUT = WEB / "tests" / "results" / "results_pdf_annot_list.json"
PORT = 8944

Handler = http.server.SimpleHTTPRequestHandler
Handler.extensions_map.update(
    {".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm",
     ".mjs": "text/javascript"}
)


class QuietServer(Handler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(WEB / "dist"), **kw)

    def log_message(self, *a):
        pass


socketserver.TCPServer.allow_reuse_address = True

CHECKS: list[dict] = []


def check(name: str, ok: bool, detail: str = "") -> bool:
    CHECKS.append({"name": name, "ok": bool(ok), "skip": False, "detail": detail[:400]})
    print(("PASS " if ok else "FAIL ") + name + (f"  {detail[:180]}" if detail and not ok else ""))
    return ok


def b64(p: Path) -> str:
    return base64.b64encode(p.read_bytes()).decode()


def main() -> int:
    if not FIXTURE.exists():
        subprocess.run(GEN, check=True, cwd=str(WEB))

    httpd = socketserver.TCPServer(("127.0.0.1", PORT), QuietServer)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()

    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1400, "height": 900})
        page.goto(f"http://127.0.0.1:{PORT}/index.html")
        page.wait_for_function("window.__pdfReady === true", timeout=20000)
        page.click("#tab-pdf")
        page.wait_for_selector("#pdf-panel:not([hidden])", state="visible")

        state0 = page.evaluate("window.__pdfState()")
        if not state0["wasm"]:
            check("annots_list_ready", False, "SKIP 门:pkg_pdf 缺席——本 e2e 需会话面")
        else:
            # 1. 打开 → 零批注空态(按钮使能 + 空态文案在场)
            r = page.evaluate("window.__pdfOpenBytes('%s')" % b64(FIXTURE))
            check("annots_open", r.get("ok") is True, json.dumps(r))
            page.wait_for_function("window.__pdfState().open === true", timeout=10000)
            page.click("#pdf-annots-btn")
            page.wait_for_selector("#pdf-annots-drawer:not([hidden])", state="visible")
            ui0 = page.evaluate("window.__pdfAnnotsUi()")
            empty_visible = page.eval_on_selector(".pdf-annots-empty", "el => !el.hidden")
            check("annots_empty_state",
                  ui0["count"] == 0 and empty_visible is True,
                  json.dumps({"ui": ui0, "empty": empty_visible}))

            # 2. 程序化加 3 条批注(页0 高亮 / 页1 高亮 / 页1 墨迹)→ 面板 3 行
            r1 = page.evaluate(
                "([p, rect]) => window.__pdfAddHighlight(p, rect)",
                [0, {"x": 72, "y": 700, "w": 200, "h": 24}])
            r2 = page.evaluate(
                "([p, rect]) => window.__pdfAddHighlight(p, rect)",
                [1, {"x": 72, "y": 700, "w": 200, "h": 24}])
            r3 = page.evaluate(
                "([p, strokes]) => window.__pdfAddInk(p, strokes)",
                [1, [[[72, 100], [120, 110], [180, 105]], [[72, 130], [160, 125]]]])
            added = all(x.get("ok") for x in (r1, r2, r3))
            ui = page.evaluate("window.__pdfAnnotsUi()")
            rows = page.query_selector_all(".pdf-annots-item")
            kinds = [r.get_attribute("data-kind") for r in rows]
            check("annots_rows_after_add",
                  added and ui["count"] == 3 and len(rows) == 3
                  and ui["pages"] == [0, 1, 1] and kinds == ["Highlight", "Highlight", "Ink"],
                  json.dumps({"ui": ui, "kinds": kinds}))

            # 3. 徽标与页号文本(DOM 逐字——locale 无关取 kind 英文名)
            badges = [r.query_selector(".pdf-annots-badge").text_content().strip() for r in rows]
            pages_txt = [r.query_selector(".pdf-annots-page").text_content().strip() for r in rows]
            check("annots_badge_page_text",
                  badges == ["highlight", "highlight", "ink"]
                  and pages_txt == ["p.1", "p.2", "p.2"],
                  json.dumps({"badges": badges, "pages": pages_txt}))

            # 4. 点击第 2 行(页1=0 基)→ 跳到第 2 页(页码行 2 / 3)
            page.click('.pdf-annots-item[data-page="1"]')
            page.wait_for_function(
                "document.getElementById('pdf-page-label').textContent === '2 / 3'",
                timeout=5000,
            )
            label = page.text_content("#pdf-page-label")
            check("annots_click_jump", label == "2 / 3", f"label={label}")

            # 5. 换开新档(零批注)→ 面板清空 + 空态回显
            r4 = page.evaluate("window.__pdfOpenBytes('%s')" % b64(FIXTURE))
            page.wait_for_function(
                "window.__pdfAnnotsUi().count === 0", timeout=10000)
            ui2 = page.evaluate("window.__pdfAnnotsUi()")
            empty2 = page.eval_on_selector(".pdf-annots-empty", "el => !el.hidden")
            check("annots_reopen_reset",
                  r4.get("ok") is True and ui2["count"] == 0 and empty2 is True,
                  json.dumps({"ui": ui2, "empty": empty2}))

        browser.close()

    httpd.shutdown()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    passed = sum(1 for c in CHECKS if c["ok"])
    OUT.write_text(json.dumps({
        "meta": {
            "gate": "#475 web pdf annot list e2e",
            "dist": str(WEB / "dist"),
            "fixture": str(FIXTURE),
        },
        "tests": CHECKS,
    }, ensure_ascii=False, indent=1))
    print(f"\n{passed}/{len(CHECKS)} checks passed -> {OUT}")
    return 0 if passed == len(CHECKS) else 1


if __name__ == "__main__":
    sys.exit(main())
