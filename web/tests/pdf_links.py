#!/usr/bin/env python3
"""#466 web PDF 链接点击导航 e2e——内链跳页 + 外链安全策略。

夹具 web/tests/fixtures/sample_links.pdf（gen_pdf_links_fixture.py：3 页 +
第 1 页 3 条 Link——内链→第 3 页 / 白名单外链 / javascript: 恶意 scheme）。

驱动面：__pdfOpenBytes / __pdfState / __pdfLinks（逐页计数 + 策略拒绝面）
+ 手势级 page.click('.pdf-link-hotspot')（真实 <a> 热区）。

结果 -> tests/results/results_pdf_links.json
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
FIXTURE = WEB / "tests" / "fixtures" / "sample_links.pdf"
GEN = ["python3", "tests/gen_pdf_links_fixture.py"]
OUT = WEB / "tests" / "results" / "results_pdf_links.json"
PORT = 8942

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

        r = page.evaluate("window.__pdfOpenBytes('%s')" % b64(FIXTURE))
        check("links_open", r.get("ok") is True and r.get("value", {}).get("pages") == 3,
              json.dumps(r))
        page.wait_for_function("window.__pdfState().open === true", timeout=10000)

        # 1. 链接面计数：第 1 页内 1 外 1（恶意 scheme 不渲染）；策略拒绝 1 条
        links = page.evaluate("window.__pdfLinks()")
        p1 = links["perPage"][0] if links["perPage"] else {}
        check(
            "links_counts",
            p1.get("total") == 2 and p1.get("internal") == 1 and p1.get("external") == 1
            and links["policySkipped"] == ["javascript"],
            json.dumps(links),
        )

        # 2. 热区 DOM：恰 2 个 <a>（恶意 scheme 不在场）
        hotspots = page.query_selector_all('.pdf-page[data-page="0"] .pdf-link-hotspot')
        check("links_dom_two_hotspots", len(hotspots) == 2, f"count={len(hotspots)}")

        # 3. 外链属性（白名单 href + _blank + noopener noreferrer——不真点，
        #    属性即合同；真点会开新页，headless 断言不必要）
        ext = page.query_selector('.pdf-page[data-page="0"] .pdf-link-hotspot[href^="https://"]')
        ext_ok = ext is not None and ext.get_attribute("target") == "_blank" \
            and "noopener" in (ext.get_attribute("rel") or "") \
            and ext.get_attribute("href") == "https://example.com/mdcore-link"
        check("links_external_attrs", ext_ok,
              f"href={ext.get_attribute('href') if ext else None} "
              f"target={ext.get_attribute('target') if ext else None} "
              f"rel={ext.get_attribute('rel') if ext else None}")

        # 4. 内链手势级点击 → 跳第 3 页（页码行 + 目标页渲染收敛）
        page.click('.pdf-page[data-page="0"] .pdf-link-hotspot[href="#"]')
        page.wait_for_function(
            "document.getElementById('pdf-page-label').textContent === '3 / 3'",
            timeout=5000,
        )
        try:
            page.wait_for_function("window.__pdfPage(3).rendered === true", timeout=5000)
        except Exception:
            pass  # IO 异步渲染未收敛不判死——下一行的 rendered 断言兜底
        label = page.text_content("#pdf-page-label")
        pg3 = page.evaluate("window.__pdfPage(3)")
        check("links_internal_jump", label == "3 / 3" and pg3["rendered"],
              f"label={label} page3={json.dumps(pg3)}")

        # 5. 恶意 scheme 热区不渲染（DOM 无 javascript: href）
        bad = page.query_selector_all('.pdf-page[data-page="0"] a[href^="javascript"]')
        check("links_malicious_not_rendered", len(bad) == 0, f"count={len(bad)}")

        # 6. 缩放后热区重投影（relayout——热区仍在场且计数不变）
        page.click("#pdf-zoom-in")
        page.wait_for_timeout(300)
        links2 = page.evaluate("window.__pdfLinks()")
        hotspots2 = page.query_selector_all('.pdf-page[data-page="0"] .pdf-link-hotspot')
        check("links_reproject_after_zoom",
              links2["perPage"][0]["total"] == 2 and len(hotspots2) == 2,
              f"links={json.dumps(links2['perPage'])} dom={len(hotspots2)}")

        # 7. 空态：无链接件 → 全零 + 空层（sample.pdf 复用 gen_pdf_fixture）
        plain = WEB / "tests" / "fixtures" / "sample.pdf"
        if not plain.exists():
            subprocess.run(["python3", "tests/gen_pdf_fixture.py"], check=True, cwd=str(WEB))
        r2 = page.evaluate("window.__pdfOpenBytes('%s')" % b64(plain))
        check("links_plain_open", r2.get("ok") is True, json.dumps(r2))
        page.wait_for_function("window.__pdfState().open === true", timeout=10000)
        links3 = page.evaluate("window.__pdfLinks()")
        hotspots3 = page.query_selector_all(".pdf-link-hotspot")
        check("links_empty_doc",
              all(pp["total"] == 0 for pp in links3["perPage"])
              and links3["policySkipped"] == [] and len(hotspots3) == 0,
              json.dumps(links3))

        browser.close()

    httpd.shutdown()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    passed = sum(1 for c in CHECKS if c["ok"])
    OUT.write_text(json.dumps({
        "meta": {
            "gate": "#466 web pdf links e2e",
            "dist": str(WEB / "dist"),
            "fixture": str(FIXTURE),
        },
        "tests": CHECKS,
    }, ensure_ascii=False, indent=1))
    print(f"\n{passed}/{len(CHECKS)} checks passed -> {OUT}")
    return 0 if passed == len(CHECKS) else 1


if __name__ == "__main__":
    sys.exit(main())
