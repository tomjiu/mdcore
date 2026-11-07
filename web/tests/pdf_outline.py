#!/usr/bin/env python3
"""#463 web PDF 大纲树 e2e——pdf.js getOutline → 树 UI + 点击跳页。

夹具 web/tests/fixtures/sample_outline.pdf（gen_pdf_outline_fixture.py 生成：
3 页 + 3 节点大纲 Chapter 1 / Section 1.1 / Chapter 2）。空态对照 =
sample.pdf（无大纲件）。

驱动面（与 pdf_view.py 同构）：
  - __pdfOpenBytes / __pdfState / __pdfPage / __pdfOutline /
    __pdfOutlineSession（模型面：会话 Outline 命令回读）；
  - 手势级：page.click('.pdf-outline-label') / page.click(
    '.pdf-outline-toggle')——点击真实 DOM（非探针直调跳页）。

结果 -> tests/results/results_pdf_outline.json
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
FIXTURE = WEB / "tests" / "fixtures" / "sample_outline.pdf"
GEN = ["python3", "tests/gen_pdf_outline_fixture.py"]
PLAIN = WEB / "tests" / "fixtures" / "sample.pdf"
GEN_PLAIN = ["python3", "tests/gen_pdf_fixture.py"]
OUT = WEB / "tests" / "results" / "results_pdf_outline.json"
PORT = 8941

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
    if not PLAIN.exists():
        subprocess.run(GEN_PLAIN, check=True, cwd=str(WEB))

    httpd = socketserver.TCPServer(("127.0.0.1", PORT), QuietServer)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()

    failures = 0
    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1400, "height": 900})
        page.goto(f"http://127.0.0.1:{PORT}/index.html")
        page.wait_for_function("window.__pdfReady === true", timeout=20000)
        page.click("#tab-pdf")
        page.wait_for_selector("#pdf-panel:not([hidden])", state="visible")

        state0 = page.evaluate("window.__pdfState()")
        if not state0["wasm"]:
            check("outline_ready", False, "SKIP 门:pkg_pdf 缺席(display-only)——本 e2e 需会话面")
            failures += 1
        else:
            # 1. 打开带大纲夹具 → 树在场 + 抽屉自动展开 + 标题逐字
            r = page.evaluate("window.__pdfOpenBytes('%s')" % b64(FIXTURE))
            check("outline_open", r.get("ok") is True and r.get("value", {}).get("pages") == 3,
                  json.dumps(r))
            page.wait_for_function("window.__pdfState().open === true", timeout=10000)
            ol = page.evaluate("window.__pdfOutline()")
            check(
                "outline_tree_present",
                ol["present"] and ol["count"] == 3 and ol["skipped"] == 0
                and ol["titles"] == ["Chapter 1", "Section 1.1", "Chapter 2"]
                and ol["drawerOpen"],
                json.dumps(ol),
            )

            # 2. 会话模型面：Outline 命令回读（ctor 注入实数据——原恒空树）
            ses = page.evaluate("window.__pdfOutlineSession()")
            check(
                "outline_session_model",
                ses.get("ok") is True
                and ses.get("value", {}).get("count") == 3
                and ses.get("value", {}).get("titles") == ["Chapter 1", "Section 1.1", "Chapter 2"],
                json.dumps(ses),
            )

            # 3. 树 DOM（手势级）——先序文档序 + 折叠件在场
            labels = page.eval_on_selector_all(
                ".pdf-outline-label", "els => els.map(e => e.textContent)")
            check("outline_dom_order", labels == ["Chapter 1", "Section 1.1", "Chapter 2"],
                  json.dumps(labels))
            toggles = page.query_selector_all(".pdf-outline-toggle")
            check("outline_dom_toggle", len(toggles) == 1, f"toggles={len(toggles)}")

            # 4. 点击跳页（Section 1.1 → 第 2 页）：页码行 + 目标页渲染
            page.click('.pdf-outline-label:has-text("Section 1.1")')
            page.wait_for_function(
                "document.getElementById('pdf-page-label').textContent === '2 / 3'",
                timeout=5000,
            )
            label = page.text_content("#pdf-page-label")
            pg2 = page.evaluate("window.__pdfPage(2)")
            check("outline_jump_to_page", label == "2 / 3" and pg2["rendered"],
                  f"label={label} page2={json.dumps(pg2)}")

            # 5. 折叠/展开（toggle 不触发跳页——页码不变）
            page.click('.pdf-outline-toggle')
            hidden_child = page.eval_on_selector(
                '.pdf-outline-item[data-title="Chapter 1"] > div:last-child',
                "el => el.hidden")
            label_after_toggle = page.text_content("#pdf-page-label")
            page.click('.pdf-outline-toggle')
            shown_child = page.eval_on_selector(
                '.pdf-outline-item[data-title="Chapter 1"] > div:last-child',
                "el => el.hidden")
            check(
                "outline_collapse_expand",
                hidden_child is True and shown_child is False and label_after_toggle == "2 / 3",
                f"collapsed={hidden_child} expanded={shown_child} label={label_after_toggle}",
            )

            # 6. 空态对照：无大纲件 → 按钮禁用 + 抽屉隐藏 + 树空
            r2 = page.evaluate("window.__pdfOpenBytes('%s')" % b64(PLAIN))
            check("outline_plain_open", r2.get("ok") is True, json.dumps(r2))
            page.wait_for_function("window.__pdfState().open === true", timeout=10000)
            ol2 = page.evaluate("window.__pdfOutline()")
            btn_disabled = page.eval_on_selector("#pdf-outline-btn", "el => el.disabled")
            check(
                "outline_empty_doc_hidden",
                ol2["present"] is False and ol2["drawerOpen"] is False and btn_disabled is True,
                json.dumps({"ol": ol2, "btn_disabled": btn_disabled}),
            )

            # 7. 空态诚实面：disabled 按钮不派发 click（浏览器语义）——
            #    抽屉保持隐藏 + 空态文案元素在场（不是死 DOM）
            drawer_hidden = page.eval_on_selector("#pdf-outline-drawer", "el => el.hidden")
            empty_el = page.query_selector(".pdf-outline-empty")
            btn_disabled2 = page.eval_on_selector("#pdf-outline-btn", "el => el.disabled")
            check(
                "outline_disabled_button_keeps_hidden",
                drawer_hidden is True and empty_el is not None and btn_disabled2 is True,
                f"drawer_hidden={drawer_hidden} btn_disabled={btn_disabled2}",
            )

        browser.close()

    httpd.shutdown()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    passed = sum(1 for c in CHECKS if c["ok"])
    OUT.write_text(json.dumps({
        "meta": {
            "gate": "#463 web pdf outline e2e",
            "dist": str(WEB / "dist"),
            "fixture": str(FIXTURE),
            "plain": str(PLAIN),
        },
        "tests": CHECKS,
    }, ensure_ascii=False, indent=1))
    print(f"\n{passed}/{len(CHECKS)} checks passed -> {OUT}")
    return 1 if failures or passed < len(CHECKS) else 0


if __name__ == "__main__":
    sys.exit(main())
