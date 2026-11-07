#!/usr/bin/env python3
"""#472 web PDF 文档属性面板 e2e——pdf.js getMetadata /Info 字段展示。

夹具 web/tests/fixtures/sample_props.pdf(全字段 /Info);空态对照 =
sample.pdf(无 /Info——字段「—」诚实空态)。

驱动面:__pdfOpenBytes / __pdfProps(字段镜像 + 开态)+ 手势级
page.click('#pdf-props-btn') 与 .pdf-props-row[data-field] DOM 断言。

结果 -> tests/results/results_pdf_props.json
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
FIXTURE = WEB / "tests" / "fixtures" / "sample_props.pdf"
GEN = ["python3", "tests/gen_pdf_props_fixture.py"]
PLAIN = WEB / "tests" / "fixtures" / "sample.pdf"
GEN_PLAIN = ["python3", "tests/gen_pdf_fixture.py"]
OUT = WEB / "tests" / "results" / "results_pdf_props.json"
PORT = 8943

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


def field_value(page, key: str) -> str:
    el = page.query_selector(f'.pdf-props-row[data-field="{key}"] dd')
    return (el.text_content() or "").strip() if el else "<missing>"


def main() -> int:
    if not FIXTURE.exists():
        subprocess.run(GEN, check=True, cwd=str(WEB))
    if not PLAIN.exists():
        subprocess.run(GEN_PLAIN, check=True, cwd=str(WEB))

    httpd = socketserver.TCPServer(("127.0.0.1", PORT), QuietServer)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()

    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 1400, "height": 900})
        page.goto(f"http://127.0.0.1:{PORT}/index.html")
        page.wait_for_function("window.__pdfReady === true", timeout=20000)
        page.click("#tab-pdf")
        page.wait_for_selector("#pdf-panel:not([hidden])", state="visible")

        # 1. 打开全字段夹具 → 属性数据在场(按钮使能,抽屉默认隐藏)
        r = page.evaluate("window.__pdfOpenBytes('%s')" % b64(FIXTURE))
        check("props_open", r.get("ok") is True and r.get("value", {}).get("pages") == 3,
              json.dumps(r))
        page.wait_for_function("window.__pdfState().open === true", timeout=10000)
        pr = page.evaluate("window.__pdfProps()")
        expect = {
            "title": "mdcore 属性面板夹具", "author": "张三",
            "subject": "招标文件核对样张", "keywords": "标书, 属性, 面板",
            "producer": "gen_pdf_props_fixture.py", "creator": "mdcore fixture family",
            "creationDate": "D:20260101080000+08'00'",
            "modDate": "D:20260102090000+08'00'",
        }
        ok_fields = all(pr["props"] and pr["props"][k] == v for k, v in expect.items())
        check("props_info_fields",
              pr["open"] and ok_fields and pr["props"]["pageCount"] == 3
              and pr["props"]["encrypted"] is False and pr["props"]["warnings"] == 0
              and pr["props"]["fileSize"] > 0 and pr["drawerOpen"] is False,
              json.dumps(pr, ensure_ascii=False))

        # 2. 手势级:点「属性」→ 抽屉展开 → 字段表逐字断言(DOM 面)
        # (DOM 行键 = propFields 的 created/modified——与探针的
        #  creationDate/modDate 同源两键名,断言各自对齐)
        page.click("#pdf-props-btn")
        page.wait_for_selector("#pdf-props-drawer:not([hidden])", state="visible")
        dom_expect = {
            "title": expect["title"], "author": expect["author"],
            "subject": expect["subject"], "keywords": expect["keywords"],
            "producer": expect["producer"], "creator": expect["creator"],
            "created": expect["creationDate"], "modified": expect["modDate"],
        }
        dom_ok = all(field_value(page, k) == v for k, v in dom_expect.items())
        enc = field_value(page, "encrypted")
        check("props_dom_fields",
              dom_ok and field_value(page, "pages") == "3"
              and enc in ("否", "no"),  # locale 无关(zh-CN/en 两词典)
              "title=" + field_value(page, "title") + " author=" + field_value(page, "author"))
        check("props_pdf_version", field_value(page, "pdfVersion") == "1.4",
              field_value(page, "pdfVersion"))

        # 3. 空态对照:无 /Info 件 → 字段「—」诚实显示,不报错
        r2 = page.evaluate("window.__pdfOpenBytes('%s')" % b64(PLAIN))
        check("props_plain_open", r2.get("ok") is True, json.dumps(r2))
        page.wait_for_function("window.__pdfState().open === true", timeout=10000)
        page.click("#pdf-props-btn")  # 抽屉应仍开(用户选择保持)——数据换新
        page.wait_for_timeout(200)
        pr2 = page.evaluate("window.__pdfProps()")
        check("props_empty_info",
              pr2["open"] and pr2["props"]["title"] is None
              and pr2["props"]["author"] is None and pr2["props"]["pageCount"] == 3,
              json.dumps(pr2, ensure_ascii=False))
        check("props_empty_dom_mark",
              field_value(page, "title") == "—" and field_value(page, "author") == "—",
              f"title={field_value(page, 'title')} author={field_value(page, 'author')}")

        # 4. 抽屉开合:再点按钮 → 隐藏
        page.click("#pdf-props-btn")
        hidden = page.eval_on_selector("#pdf-props-drawer", "el => el.hidden")
        check("props_toggle_close", hidden is True, f"hidden={hidden}")

        browser.close()

    httpd.shutdown()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    passed = sum(1 for c in CHECKS if c["ok"])
    OUT.write_text(json.dumps({
        "meta": {
            "gate": "#472 web pdf props e2e",
            "dist": str(WEB / "dist"),
            "fixture": str(FIXTURE),
        },
        "tests": CHECKS,
    }, ensure_ascii=False, indent=1))
    print(f"\n{passed}/{len(CHECKS)} checks passed -> {OUT}")
    return 0 if passed == len(CHECKS) else 1


if __name__ == "__main__":
    sys.exit(main())
