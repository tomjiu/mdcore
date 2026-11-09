#!/usr/bin/env python3
"""Start the packaged Windows GPUI app outside checkout with shader source hidden.

No interactive operator, screenshot tool, registry installation, or drive mapping
is needed. A hidden real top-level window must answer WM_NULL; the application's
existing smoke actions must open Markdown/PDF and quit normally. Windows must
still provide a window station and a working Direct3D adapter. Missing graphics
capability is a failing gate with logs, never a skipped startup success.
"""
from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import uuid


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def inside(path: Path, directory: Path) -> bool:
    try:
        path.relative_to(directory)
        return True
    except ValueError:
        return False


def hidden_options() -> dict:
    info = subprocess.STARTUPINFO()
    info.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 0  # SW_HIDE, while keeping the real GUI message loop.
    return {"startupinfo": info, "creationflags": subprocess.CREATE_NO_WINDOW}


def responding_window(pid: int) -> dict | None:
    user = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    user.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user.SendMessageTimeoutW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM, wintypes.UINT, wintypes.UINT, ctypes.POINTER(ctypes.c_size_t)]
    user.SendMessageTimeoutW.restype = ctypes.c_ssize_t
    found = []

    @callback_type
    def visit(hwnd, _):
        owner = wintypes.DWORD()
        user.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value != pid:
            return True
        title = ctypes.create_unicode_buffer(512)
        user.GetWindowTextW(hwnd, title, len(title))
        if not title.value.startswith("mang"):
            return True
        result = ctypes.c_size_t()
        # WM_NULL + SMTO_ABORTIFHUNG: a HWND alone is not startup success.
        responsive = bool(user.SendMessageTimeoutW(hwnd, 0, 0, 0, 2, 1000, ctypes.byref(result)))
        kind = ctypes.create_unicode_buffer(128)
        user.GetClassNameW(hwnd, kind, len(kind))
        found.append({"handle": int(hwnd), "title": title.value, "class": kind.value, "responsive": responsive})
        return False

    user.EnumWindows(visit, 0)
    return found[0] if found else None


def small_pdf() -> bytes:
    # Self-contained one-page fixture: pdfium loads the actual packaged DLL.
    bodies = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    text = b"BT /F1 14 Tf 20 160 Td (Portable PDF smoke) Tj ET\n"
    bodies.append(b"<< /Length " + str(len(text)).encode() + b" >>\nstream\n" + text + b"endstream")
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for number, body in enumerate(bodies, 1):
        offsets.append(len(out))
        out += str(number).encode() + b" 0 obj\n" + body + b"\nendobj\n"
    startxref = len(out)
    out += b"xref\n0 6\n0000000000 65535 f \n"
    for offset in offsets:
        out += f"{offset:010d} 00000 n \n".encode()
    out += f"trailer << /Size 6 /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n".encode()
    return bytes(out)


def run_cli(executable: Path, flag: str, directory: Path, environment: dict, output: Path, label: str) -> subprocess.CompletedProcess:
    result = subprocess.run([str(executable), flag], cwd=directory, env=environment, capture_output=True, timeout=20, **hidden_options())
    (output / f"{label}.stdout.log").write_bytes(result.stdout)
    (output / f"{label}.stderr.log").write_bytes(result.stderr)
    if result.returncode:
        raise RuntimeError(f"{label} failed with exit {result.returncode}")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--installer", type=Path)
    source.add_argument("--payload", type=Path, help="Local development only; CI validates the finished installer")
    parser.add_argument("--expected-payload", type=Path, help="Compare the extracted exe/DLL hashes with staging")
    parser.add_argument("--checkout-root", type=Path, required=True)
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--output", type=Path, default=Path("build/nsis/portable-smoke"))
    parser.add_argument("--smoke-seconds", type=float, default=8)
    parser.add_argument("--timeout", type=float, default=45)
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("the real window startup gate requires Windows")
    if args.smoke_seconds <= 0 or args.timeout <= args.smoke_seconds:
        parser.error("timeout must exceed the positive smoke duration")

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    checkout = args.checkout_root.resolve(strict=True)
    shader_source = (checkout / "third_party/zed/crates/gpui_windows/src").resolve(strict=True)
    hidden_source = shader_source.with_name("src-portable-smoke-" + uuid.uuid4().hex)
    if not inside(shader_source, checkout) or not inside(hidden_source, checkout):
        parser.error("shader source rename must remain within the explicit checkout")
    temporary = Path(tempfile.mkdtemp(prefix="mdcore-portable-")).resolve()
    process = None
    report = {"passed": False, "head": os.environ.get("GITHUB_SHA"), "version_expected": args.expected_version, "source_hidden": False, "shader_source": str(shader_source), "run_directory": str(temporary), "window": None, "exit_code": None, "payload_matches_stage": False}
    try:
        if inside(temporary, checkout):
            raise RuntimeError("portable launch directory must be outside checkout")
        package = temporary / "app"
        package.mkdir()
        if args.installer:
            candidates = [shutil.which("7z"), r"C:\Program Files\7-Zip\7z.exe"]
            archive_tool = next((p for p in candidates if p and Path(p).is_file()), None)
            if archive_tool is None:
                raise RuntimeError("7-Zip is required to inspect the finished NSIS payload")
            extract = subprocess.run([archive_tool, "x", "-y", "-o" + str(package), str(args.installer.resolve(strict=True))], capture_output=True, timeout=60, **hidden_options())
            (output / "extract.log").write_bytes(extract.stdout + extract.stderr)
            if extract.returncode:
                raise RuntimeError(f"installer extraction failed with exit {extract.returncode}")
            report["installer_sha256"] = sha256(args.installer)
        else:
            for name in ["mdcore.exe", "pdfium.dll"]:
                shutil.copyfile(args.payload / name, package / name)
        executables = list(package.rglob("mdcore.exe"))
        if len(executables) != 1:
            raise RuntimeError(f"expected exactly one packaged mdcore.exe, got {len(executables)}")
        executable = executables[0]
        dll = executable.with_name("pdfium.dll")
        if not dll.is_file():
            raise RuntimeError("packaged pdfium.dll must be beside mdcore.exe")
        report["exe_sha256"], report["pdfium_sha256"] = sha256(executable), sha256(dll)
        if args.expected_payload:
            for name, actual in [("mdcore.exe", executable), ("pdfium.dll", dll)]:
                if sha256(args.expected_payload / name) != sha256(actual):
                    raise RuntimeError(f"installer payload differs from staging: {name}")
            report["payload_matches_stage"] = True

        markdown = temporary / "portable-smoke.md"
        pdf = temporary / "portable-smoke.pdf"
        markdown.write_text("# Portable startup smoke\n\nMarkdown opened outside checkout.\n", encoding="utf-8")
        pdf.write_bytes(small_pdf())
        environment = dict(os.environ, XDG_CONFIG_HOME=str(temporary / "config"), ZED_LOG="gpui_windows=info")
        # Force pdfium's deployment lookup to use the packaged exe directory.
        environment.pop("MDC_PDFIUM_LIB", None)
        shader_source.rename(hidden_source)
        report["source_hidden"] = True
        version = run_cli(executable, "--version", temporary, environment, output, "version")
        report["version_actual"] = version.stdout.decode("utf-8", "replace").strip()
        if report["version_actual"] != "mang-shell " + args.expected_version:
            raise RuntimeError("package executable version differs from installer version")
        run_cli(executable, "--check-pdfium", temporary, environment, output, "pdfium")
        arguments = [str(executable), "--smoke-sec", str(args.smoke_seconds), "--smoke-open", str(markdown), "--smoke-open", str(pdf), "--smoke-tab-keys", "--no-rpc"]
        report["arguments"] = arguments
        with (output / "app.stdout.log").open("wb") as stdout, (output / "app.stderr.log").open("wb") as stderr:
            process = subprocess.Popen(arguments, cwd=temporary, env=environment, stdout=stdout, stderr=stderr, **hidden_options())
            report["pid"] = process.pid
            deadline = time.monotonic() + args.timeout
            while process.poll() is None:
                window = responding_window(process.pid)
                if window and window["responsive"]:
                    report["window"] = window
                if time.monotonic() >= deadline:
                    raise RuntimeError(f"startup timeout after {args.timeout}s")
                time.sleep(0.05)
            report["exit_code"] = process.returncode
        if process.returncode != 0:
            raise RuntimeError(f"application startup/smoke failed with exit {process.returncode}")
        if report["window"] is None:
            raise RuntimeError("application never established a responding mang window")
        stdout_text = (output / "app.stdout.log").read_text(encoding="utf-8", errors="replace")
        stderr_text = (output / "app.stderr.log").read_text(encoding="utf-8", errors="replace")
        required = ["open: " + str(markdown), "open: " + str(pdf), "tabs: open → 2 开", "tabs: cycle →", "tabs: close →"]
        if "smoke: quit reached" not in stdout_text or any(marker not in stderr_text for marker in required):
            raise RuntimeError("normal smoke exit or Markdown/PDF/tab-action evidence is missing")
        if re.search(r"pdf:\s+[1-9]\d*\s+页", stderr_text) is None:
            raise RuntimeError("packaged PDF open did not report a readable page")
        report["passed"] = True
    except Exception as error:
        report["failure"] = str(error)
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        if process is not None:
            report["exit_code"] = process.returncode
        if hidden_source.exists():
            if shader_source.exists():
                report["passed"] = False
                report["failure"] = "refusing to overwrite recreated shader source during restoration"
            else:
                hidden_source.rename(shader_source)
        report["source_restored"] = shader_source.is_dir() and not hidden_source.exists()
        if not report["source_restored"]:
            report["passed"] = False
        shutil.rmtree(temporary)
        (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    raise SystemExit(main())
