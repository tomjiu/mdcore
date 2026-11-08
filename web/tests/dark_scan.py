#!/usr/bin/env python3
"""#520 dark-label scan e2e (web host of the #502 wasm bridge — toolbar
「暗标扫描」entry): any .docx/.xlsx artifact → dark_scan → structured
residual list. Locks: entry presence (bilingual label/title + accept face),
dirty xlsx (injected real-name creator/lastModifiedBy → 2 residuals with
part/field/value triple), clean xlsx (anonymize canonical core.xml → 0
residuals + clean banner), dirty docx (injected comment author/initials +
creator → 3 residuals across two parts), not-a-zip container → typed
`dark_scan` reject, foreign extension → typed `unsupported_kind` reject
(no guessing), contract intact + zero pageerror. Fixtures are crafted at
runtime (zip part rewrite — deterministic, no binary fixtures in-tree).
Evidence -> results/results_dark_scan.json
"""

import json
import socketserver
import http.server
import sys
import tempfile
import zipfile
from pathlib import Path

from playwright.sync_api import sync_playwright

WEB = Path(__file__).resolve().parent.parent
OUT = WEB / "tests" / "results" / "results_dark_scan.json"

# office-io anonymize.rs ANON_CORE_XML（匿名规范形——清理面常量真源镜像）
ANON_CORE = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"'
    ' xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"'
    ' xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
    '<dcterms:created xsi:type="dcterms:W3CDTF">1970-01-01T00:00:00Z</dcterms:created>'
    '<dc:creator>unknown</dc:creator><cp:lastModifiedBy>unknown</cp:lastModifiedBy>'
    '<dcterms:modified xsi:type="dcterms:W3CDTF">1970-01-01T00:00:00Z</dcterms:modified>'
    '<cp:revision>1</cp:revision></cp:coreProperties>'
)
EMPTY_CUSTOM = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
    '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"'
    ' xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"/>'
)
INJECTED_COMMENT = (
    '<w:comment w:id="9001" w:author="E2E 批注人" w:initials="EB"'
    ' w:date="2026-01-01T00:00:00Z"><w:p/></w:comment>'
)


def rewrite_zip(src: Path, dst: Path, replace: dict) -> None:
    """zip 部件级改写（其余部件字节透传——夹具构造面，与引擎无耦合）。"""
    with zipfile.ZipFile(src) as zin, zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = replace[item.filename].encode("utf-8") if item.filename in replace else zin.read(item.filename)
            zout.writestr(item.filename, data)


def craft_fixtures(tmp: Path) -> dict:
    xlsx_src = WEB / "tests" / "fixtures" / "grid_sample.xlsx"
    docx_src = WEB / "tests" / "fixtures" / "import_sample.docx"

    # 脏 xlsx：实名 creator / lastModifiedBy（基线夹具两者皆空——空值非残留）
    with zipfile.ZipFile(xlsx_src) as z:
        core = z.read("docProps/core.xml").decode("utf-8")
    dirty_core = (core
                  .replace("<dc:creator></dc:creator>", "<dc:creator>E2E 审稿人</dc:creator>")
                  .replace("<cp:lastModifiedBy></cp:lastModifiedBy>", "<cp:lastModifiedBy>E2E 编辑</cp:lastModifiedBy>"))
    assert "E2E 审稿人" in dirty_core, "xlsx core 注入失败（夹具形态漂移）"
    dirty_xlsx = tmp / "darkscan_dirty.xlsx"
    rewrite_zip(xlsx_src, dirty_xlsx, {"docProps/core.xml": dirty_core})

    # 净 xlsx：匿名规范形（unknown + epoch + revision=1）
    clean_xlsx = tmp / "darkscan_clean.xlsx"
    rewrite_zip(xlsx_src, clean_xlsx, {"docProps/core.xml": ANON_CORE})

    # 脏 docx：实名 creator + 注入批注（作者/缩写）+ custom.xml 清空（基线变差消除）
    with zipfile.ZipFile(docx_src) as z:
        dcore = z.read("docProps/core.xml").decode("utf-8")
        dcomments = z.read("word/comments.xml").decode("utf-8")
    dcore = dcore.replace("<dc:creator>unknown</dc:creator>", "<dc:creator>E2E 作者</dc:creator>")
    assert "E2E 作者" in dcore, "docx core 注入失败"
    if "</w:comments>" in dcomments:
        dcomments = dcomments.replace("</w:comments>", INJECTED_COMMENT + "</w:comments>")
    else:
        # 空容器自闭合形态（<w:comments ... />）——展开后注入
        assert dcomments.rstrip().endswith("/>"), "docx comments 容器形态漂移"
        dcomments = dcomments.rstrip()[:-2] + ">" + INJECTED_COMMENT + "</w:comments>"
    dirty_docx = tmp / "darkscan_dirty.docx"
    rewrite_zip(docx_src, dirty_docx, {
        "docProps/core.xml": dcore,
        "docProps/custom.xml": EMPTY_CUSTOM,
        "word/comments.xml": dcomments,
    })

    notzip = tmp / "darkscan_notzip.xlsx"
    notzip.write_bytes(b"this is not a zip container at all - typed reject expected")
    foreign = tmp / "darkscan_foreign.txt"
    foreign.write_text("foreign extension — unsupported kind lens")

    return {"dirty_xlsx": dirty_xlsx, "clean_xlsx": clean_xlsx,
            "dirty_docx": dirty_docx, "notzip": notzip, "foreign": foreign}


class QuietServer(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=str(WEB / "dist"), **kw)

    def log_message(self, *a):
        pass


socketserver.TCPServer.allow_reuse_address = True


def main() -> int:
    checks = []
    page_errors: list[str] = []
    skipped = 0

    def check(name, ok, detail="", skip=False):
        nonlocal skipped
        checks.append({"name": name, "ok": bool(ok), "skip": bool(skip), "detail": str(detail)[:400]})
        tag = "SKIP " if skip else ("PASS " if ok else "FAIL ")
        if skip:
            skipped += 1
        print(tag + name + ("  | " + str(detail)[:160] if detail and not ok and not skip else ""))
        return ok

    def run_scan(page, path, prev_runs):
        """喂文件 → 等扫描完成（runs 递增）→ 返回探针状态。"""
        page.set_input_files("#darkscan-file", str(path))
        page.wait_for_function(f"window.__darkscanState().runs >= {prev_runs + 1}", timeout=15000)
        return page.evaluate("window.__darkscanState()")

    with tempfile.TemporaryDirectory() as td:
        fixtures = craft_fixtures(Path(td))

        with socketserver.TCPServer(("127.0.0.1", 8941), QuietServer) as srv:
            import threading
            threading.Thread(target=srv.serve_forever, daemon=True).start()
            with sync_playwright() as pw:
                browser = pw.chromium.launch()
                page = browser.new_page()
                page.on("pageerror", lambda e: page_errors.append(str(e)))
                page.goto("http://127.0.0.1:8941/index.html")
                page.wait_for_function("window.__ready === true", timeout=30000)

                # ---- 1. 入口在场（input + 双语标签 + title + accept 面）----
                boot = page.evaluate("""() => {
                    const input = document.getElementById('darkscan-file');
                    const label = document.querySelector('label[for=darkscan-file]');
                    return { input: !!input, accept: input ? input.accept : '',
                             label: label ? (label.textContent || '').trim() : '',
                             title: label ? (label.title || '').trim() : '' };
                }""")
                check("darkscan_entry_present",
                      boot["input"] and ".docx" in boot["accept"] and ".xlsx" in boot["accept"]
                      and len(boot["label"]) > 0 and len(boot["title"]) > 0,
                      json.dumps(boot, ensure_ascii=False))

                # ---- 2. 脏 xlsx：2 处残留（字典序 cp:lastModifiedBy < dc:creator）----
                st = run_scan(page, fixtures["dirty_xlsx"], 0)
                dom = page.evaluate("""() => ({
                    hidden: document.getElementById('darkscan-panel').hidden,
                    rows: document.querySelectorAll('#darkscan-panel .ds-row').length,
                    badge: (document.querySelector('#darkscan-panel .ds-count') || {}).textContent || '',
                    values: Array.from(document.querySelectorAll('#darkscan-panel .ds-value'))
                                  .map(el => el.textContent),
                })""")
                r = st["lastResiduals"]
                ok_dirty = (
                    st["lastKind"] == "xlsx" and st["lastCode"] is None
                    and st["lastCount"] == 2 and len(r) == 2
                    and r[0]["part"] == "docProps/core.xml" and r[0]["field"] == "cp:lastModifiedBy"
                    and r[0]["value"] == "E2E 编辑" and r[0]["hint"]
                    and r[1]["part"] == "docProps/core.xml" and r[1]["field"] == "dc:creator"
                    and r[1]["value"] == "E2E 审稿人"
                    and dom["hidden"] is False and dom["rows"] == 2
                    and "2" in dom["badge"] and "E2E 审稿人" in dom["values"]
                )
                check("darkscan_dirty_xlsx", ok_dirty,
                      json.dumps({"st": st, "dom": dom}, ensure_ascii=False))

                # ---- 3. 净 xlsx：匿名规范形 → 0 残留 + 干净横幅 ----
                st = run_scan(page, fixtures["clean_xlsx"], st["runs"])
                dom = page.evaluate("""() => ({
                    rows: document.querySelectorAll('#darkscan-panel .ds-row').length,
                    clean: !!document.querySelector('#darkscan-panel .ds-clean'),
                    badge: (document.querySelector('#darkscan-panel .ds-count') || {}).textContent || '',
                })""")
                ok_clean = (
                    st["lastKind"] == "xlsx" and st["lastCount"] == 0
                    and st["lastResiduals"] == [] and st["lastCode"] is None
                    and dom["rows"] == 0 and dom["clean"] and "0" in dom["badge"]
                )
                check("darkscan_clean_xlsx", ok_clean,
                      json.dumps({"st": st, "dom": dom}, ensure_ascii=False))

                # ---- 4. 脏 docx：3 处残留跨两部件（core + comments）----
                st = run_scan(page, fixtures["dirty_docx"], st["runs"])
                r = st["lastResiduals"]
                got = {(x["part"], x["field"], x["value"]) for x in r}
                ok_docx = (
                    st["lastKind"] == "docx" and st["lastCount"] == 3 and len(r) == 3
                    and ("docProps/core.xml", "dc:creator", "E2E 作者") in got
                    and ("word/comments.xml", "w:author", "E2E 批注人") in got
                    and ("word/comments.xml", "w:initials", "EB") in got
                )
                check("darkscan_dirty_docx", ok_docx,
                      json.dumps({"st": st}, ensure_ascii=False))

                # ---- 5. 非 zip 容器 → typed 拒（wasm dark_scan 码——fail-closed）----
                st = run_scan(page, fixtures["notzip"], st["runs"])
                dom_err = page.evaluate("""() => ({
                    err: document.getElementById('darkscan-status').classList.contains('grid-error'),
                    badge: (document.querySelector('#darkscan-panel .ds-count.dirty') || {}).textContent || '',
                })""")
                check("darkscan_notzip_typed_reject",
                      st["lastCode"] == "dark_scan" and st["lastCount"] is None
                      and bool(st.get("lastCode")) and dom_err["err"] and dom_err["badge"],
                      json.dumps({"st": st, "dom": dom_err}, ensure_ascii=False))

                # ---- 6. 他类扩展名 → typed 拒（web 层 unsupported_kind——不猜透镜）----
                st = run_scan(page, fixtures["foreign"], st["runs"])
                check("darkscan_foreign_ext_typed_reject",
                      st["lastCode"] == "unsupported_kind" and st["lastKind"] is None
                      and st["lastCount"] is None,
                      json.dumps({"st": st}, ensure_ascii=False))

                # ---- 7. 契约面 + 零页错 ----
                final = page.evaluate("""() => ({
                    ready: window.__ready === true,
                    contract: window.__contractVersion,
                    probe: typeof window.__darkscanState === 'function',
                })""")
                check("darkscan_contract_no_pageerror",
                      final["ready"] and final["probe"] and len(page_errors) == 0,
                      json.dumps({"final": final, "pageErrors": page_errors[:3]}))

                browser.close()
            srv.shutdown()

    OUT.parent.mkdir(parents=True, exist_ok=True)
    failed = [c for c in checks if not c["ok"]]
    OUT.write_text(json.dumps({
        "meta": {"gate": "#520 dark-label scan e2e (web host of #502 wasm bridge)",
                 "dist": str(WEB / "dist")},
        "tests": checks,
        "summary": {"total": len(checks), "failed": len(failed), "skipped": skipped,
                    "pass": not failed, "failures": [c["name"] for c in failed]},
    }, ensure_ascii=False, indent=2))
    print(f"\n{len(checks) - len(failed)}/{len(checks)} checks passed"
          + (f" ({skipped} skipped)" if skipped else "")
          + f" -> {OUT}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
