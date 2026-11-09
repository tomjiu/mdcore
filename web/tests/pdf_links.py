#!/usr/bin/env python3
"""#466 web PDF 链接点击导航 e2e + #557 外链输入策略面 + #601 mailto 附件键拒绝。

夹具 web/tests/fixtures/sample_links.pdf（gen_pdf_links_fixture.py：3 页；
第 1 页 3 条 Link——内链→第 3 页 / 白名单外链 / javascript: 恶意 scheme；
第 2 页 6 条 #557/#601 策略探针——2049 超限 / 恰 2048 / %E2%80%8B 编码零宽 /
%25E2%80%8B 双重编码 / mailto / mailto?attach= 附件键。原始控制码形态不经
夹具——pdf.js bytesToString 对 /URI 无 BOM 解码，由 __pdfLinkPolicy 探针
单测直测）。

驱动面：__pdfOpenBytes / __pdfState / __pdfLinks（逐页计数 + 策略拒绝面）
+ __pdfLinkPolicy（#557 策略纯函数单测——不经夹具直测 reason 语义）
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


# ---- #557 策略纯函数单测（__pdfLinkPolicy 探针——不经夹具）----
BASE = "https://example.com/"
POLICY_CASES: list[tuple[str, str, str]] = [
    # (raw, expect_ok, expect_reason|url 判定串)
    ("https://example.com/x", "ok", "url=https://example.com/x"),
    ("http://a.b/c?d=e#f", "ok", "url=http://a.b/c?d=e#f"),
    ("mailto:user@example.com", "ok", "url=mailto:user@example.com"),
    # #601：mailto 附件参数拒绝（attach/insert——本机邮件客户端文件外带面）
    ("mailto:user@example.com?attach=/etc/passwd", "rej", "mailto-attachment"),
    ("mailto:a@b.c?ATTACH=C:/doc.pdf", "rej", "mailto-attachment"),  # 键大小写不敏感
    ("mailto:a@b.c?InSeRt=C:%5Cdoc.pdf", "rej", "mailto-attachment"),  # insert 键 + 编码值
    ("mailto:a@b.c?%61ttach=/x", "rej", "mailto-attachment"),  # 百分号编码键（单次解码）
    ("mailto:a@b.c?attach=", "rej", "mailto-attachment"),  # 空值也拒（键在场即附件语义）
    ("mailto:a@b.c?subject=x&attach=/f&body=y", "rej", "mailto-attachment"),  # 合法键混入附件键
    ("mailto:a@b.c?attach=/f&attach=/g", "rej", "mailto-attachment"),  # 重复附件键逐一枚举
    ("mailto:user@example.com?subject=Hi&body=text", "ok", "url~subject=Hi"),  # 合法参数保留
    ("mailto:a@b.c?cc=other@x.y&to=user@z.w", "ok", "url~cc=other@x.y"),  # cc/to 合法
    ("mailto:a@b.c?%2561ttach=/x", "ok", "url~%2561ttach"),  # 双重编码不二次解码（字面键非 attach）
    ("HTTPS://EXAMPLE.COM/X", "ok", "url=https://example.com/X"),  # 规范化大小写
    ("https://例え.jp/パス", "ok", "url~xn--"),  # 中文 IDN → punycode 规范化
    (BASE + "b" * (2048 - len(BASE)), "ok", "len=2048"),  # 预算边界恰好通过
    ("https://x/%25E2%80%8Bp", "ok", "url=https://x/%25E2%80%8Bp"),  # 双重编码不二次解码
    ("https://x/%E2%80", "ok", "url=https://x/%E2%80"),  # 截断序列字面保留不抛异常
    (BASE + "a" * (2049 - len(BASE)), "rej", "url-over-budget"),  # 原始 2049 超限
    ("https://x/" + "あ" * 700, "rej", "url-over-budget"),  # 原始 720≤2048 但规范化膨胀超限
    ("javascript:alert(1)", "rej", "javascript"),
    ("ftp://host/x", "rej", "ftp"),
    ("example.com/x", "rej", "relative-url"),  # 相对串无基准
    ("", "rej", "relative-url"),  # 空串不抛异常
    ("https://x/‌p", "rej", "zero-width-bidi"),  # 原始 U+200B
    ("https://x/‮evil", "rej", "zero-width-bidi"),  # 原始 U+202E Bidi 覆盖
    ("https://x/⁦abc", "rej", "zero-width-bidi"),  # 原始 U+2066 Bidi 隔离
    ("https://x/﻿p", "rej", "zero-width-bidi"),  # 原始 U+FEFF BOM
    ("https://x/%E2%80%8Bp", "rej", "zero-width-bidi-encoded"),  # 编码零宽
    ("https://x/%E2%80%AEevil", "rej", "zero-width-bidi-encoded"),  # 编码 Bidi 覆盖
    ("https://x/%ZZ%E2%80%8B", "rej", "zero-width-bidi-encoded"),  # 无效段字面+有效段仍捕获
    # #568：无效 UTF-8 字节不得掩蔽同一百分号段内的合法控制码。
    ("https://x/%FF%E2%80%8B", "rej", "zero-width-bidi-encoded"),
    ("https://x/%E2%80%AE%FF", "rej", "zero-width-bidi-encoded"),
    ("https://x/%E2%80%8B%80", "rej", "zero-width-bidi-encoded"),
    ("https://x/%E2%80%FF%E2%80%8B", "rej", "zero-width-bidi-encoded"),  # 损坏序列后仍识别控制码
    ("https://x/%E2%80%8B%E2%80", "rej", "zero-width-bidi-encoded"),  # 控制码后的截断不遮蔽
    ("https://x/%ef%bb%bf%FF", "rej", "zero-width-bidi-encoded"),  # BOM 必须保留在检查面
    ("https://x/%FF%EF%BB%BF", "rej", "zero-width-bidi-encoded"),
    ("https://x/%25%E2%80%8B", "rej", "zero-width-bidi-encoded"),  # 字面百分号 + 真控制码不是双编
    ("https://x/%FF", "ok", "url=https://x/%FF"),  # 单纯坏字节不扩大拒绝面，URL 原样规范化
    ("https://x/%E2%80%FF", "ok", "url=https://x/%E2%80%FF"),
    ("https://x/%FF%25E2%80%8B", "ok", "url=https://x/%FF%25E2%80%8B"),  # 混合坏字节仍不二次解码
    ("https://x/%25E2%2580%258B", "ok", "url=https://x/%25E2%2580%258B"),
]


def run_policy_units(page) -> bool:
    raws = [c[0] for c in POLICY_CASES]
    results = page.evaluate(
        "(raws) => raws.map((r) => window.__pdfLinkPolicy(r))", raws
    )
    all_ok = True
    for (raw, expect, want), got in zip(POLICY_CASES, results):
        if expect == "ok":
            if want.startswith("url="):
                ok = got["ok"] is True and got["url"] == want[4:] and got["reason"] is None
            elif want.startswith("url~"):
                ok = got["ok"] is True and want[4:] in (got["url"] or "")
            elif want.startswith("len="):
                ok = got["ok"] is True and len(got["url"] or "") == int(want[4:])
            else:
                ok = False
        else:
            ok = got["ok"] is False and got["url"] is None and got["reason"] == want
        if not ok:
            all_ok = False
            check(f"policy_unit[{raw[:28]!r}]", False, f"expect={want} got={json.dumps(got, ensure_ascii=False)[:160]}")
    check("policy_units", all_ok, f"{len(POLICY_CASES)} 例")
    return all_ok


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

        # 0. #557/#568 策略纯函数单测（既有边界 + 混合 UTF-8 反例）
        run_policy_units(page)

        r = page.evaluate("window.__pdfOpenBytes('%s')" % b64(FIXTURE))
        check("links_open", r.get("ok") is True and r.get("value", {}).get("pages") == 3,
              json.dumps(r))
        page.wait_for_function("window.__pdfState().open === true", timeout=10000)

        # 1. 链接面计数：第 1 页内 1 外 1（恶意 scheme 不渲染）；策略拒绝面
        #    （第 1 页 javascript + 第 2 页 3 项 #557/#601 拒绝）
        links = page.evaluate("window.__pdfLinks()")
        p1 = links["perPage"][0] if links["perPage"] else {}
        check(
            "links_counts",
            p1.get("total") == 2 and p1.get("internal") == 1 and p1.get("external") == 1
            and links["policySkipped"] == [
                "javascript",
                "url-over-budget",
                "zero-width-bidi-encoded",
                "mailto-attachment",
            ],
            json.dumps(links),
        )

        # 2. 热区 DOM：第 1 页恰 2 个 <a>（恶意 scheme 不在场）
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

        # 6. #557/#601 策略面 e2e（第 2 页=data-page"1"：6 条注释 → 3 热区 + 3 拒绝）
        p2 = links["perPage"][1] if len(links["perPage"]) > 1 else {}
        check("policy_page2_counts",
              p2.get("total") == 3 and p2.get("internal") == 0 and p2.get("external") == 3,
              json.dumps(links["perPage"]))

        spots2 = page.query_selector_all('.pdf-page[data-page="1"] .pdf-link-hotspot')
        check("policy_page2_three_hotspots", len(spots2) == 3, f"count={len(spots2)}")

        hrefs2 = [s.get_attribute("href") or "" for s in spots2]
        # 6a. 恰 2048 边界热区在场（href 长恰 2048 + noopener 合同）
        at_budget = [h for h in hrefs2 if len(h) == 2048]
        rels2 = [s.get_attribute("rel") or "" for s in spots2]
        check("policy_budget_edge_hotspot",
              len(at_budget) == 1 and all("noopener" in r for r in rels2),
              f"at_budget={len(at_budget)} rels={rels2}")

        # 6b. 超预算热区不渲染（无 href 超 2048）
        check("policy_over_budget_not_rendered",
              all(len(h) <= 2048 for h in hrefs2),
              f"max={max((len(h) for h in hrefs2), default=0)}")

        # 6c. 编码零宽被拒（%E2%80%8B 单编形态不在 href；双编 %25 形态在场）
        check("policy_encoded_zw_split",
              all("%E2%80%8Bpath" not in h or "%25E2%80%8Bpath" in h for h in hrefs2)
              and any(h == "https://example.com/%25E2%80%8Bpath" for h in hrefs2),
              f"hrefs2={[h[:60] for h in hrefs2]}")

        # 6d. mailto 白名单热区（href + noopener）
        mailto = [h for h in hrefs2 if h.startswith("mailto:")]
        check("policy_mailto_hotspot", len(mailto) == 1, f"mailto={mailto}")

        # 6f. #601：mailto attach 被拒——无 href 含 attach 键；合法 mailto
        #     热区仍完整在场（同页合法/拒绝共存）
        check("policy_mailto_attach_denied",
              all("attach" not in h for h in hrefs2) and len(mailto) == 1
              and all("/etc/passwd" not in h for h in hrefs2),
              f"hrefs2={[h[:48] for h in hrefs2]}")

        # 6e. 全部 href 无控制码残留（含允许面——双编/边界/mailto 均洁净）
        check("policy_raw_controls_not_rendered",
              all("\u202e" not in h and "\u200b" not in h for h in hrefs2),
              f"hrefs2={[h[:40] for h in hrefs2]}")

        # 7. 缩放后热区重投影（relayout——热区仍在场且计数不变）
        page.click("#pdf-zoom-in")
        page.wait_for_timeout(300)
        links2 = page.evaluate("window.__pdfLinks()")
        hotspots2 = page.query_selector_all('.pdf-page[data-page="0"] .pdf-link-hotspot')
        check("links_reproject_after_zoom",
              links2["perPage"][0]["total"] == 2 and len(hotspots2) == 2,
              f"links={json.dumps(links2['perPage'])} dom={len(hotspots2)}")

        # 8. 空态：无链接件 → 全零 + 空层（sample.pdf 复用 gen_pdf_fixture）
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
            "gate": "#466+#557 web pdf links e2e",
            "dist": str(WEB / "dist"),
            "fixture": str(FIXTURE),
        },
        "tests": CHECKS,
    }, ensure_ascii=False, indent=1))
    print(f"\n{passed}/{len(CHECKS)} checks passed -> {OUT}")
    return 0 if passed == len(CHECKS) else 1


if __name__ == "__main__":
    sys.exit(main())
