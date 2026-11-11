"""#143 — NSIS per-user（HKCU）+ 礼貌类型注册：行为结构锁。

ElyOffice `tests/scripts/windows-types.sh` 钉死的六条行为，在本仓无
Windows 真机常态的现实下，以三层钉子承载：

  ① 模板结构断言（本文件主体）——注册表面在源码层不可回退；
  ② 渲染产物断言——真实 builder 走查后对 `installer.nsi` 重跑核心断言
    （防占位符替换/构建器改动破坏结构）；
  ③ makensis 编译面（可得时执行）——Debian nsis 3.11 / choco nsis 3.x，
    BUILD-VERIFY §3 同口径；不可得时 SKIP（如实报告，不伪装通过）。

Windows 注册表实机行为（预装 .md=别的程序 → 安装后不动 等）归 R24-d
真机窗口（checklist D2 预装关联验证项，本 PR 一并登记）。

双平台可跑：纯 Python 文本分析，无 Windows 专属依赖；Linux 与
Windows CI 语义一致（本仓跨平台纪律——shell/路径差异全部收在本
文件的探测层，断言层共享）。

运行：
    python3 -m unittest packaging.tests.test_ns_polite_registration -v
（或仓根：python3 -m unittest discover -s packaging/tests -p "test_*.py"）
"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(os.environ.get("PACKAGING_UNDER_TEST", Path(__file__).resolve().parents[2]))
TPL = ROOT / "packaging" / "nsis" / "installer.nsi.tpl"


def tpl_text() -> str:
    """模板文本（存在性由 test 前置断言保证）。"""
    return TPL.read_text(encoding="utf-8")


def strip_comments(text: str) -> str:
    """剥离 NSIS 注释行（`;`/`#` 开头的行）。

    注册表面断言只作用于**指令面**——历史注释允许提及 HKLM/HKR
    等缺陷根（「为什么改」的记录价值高于字面禁忌）。本模板注释
    均为整行制（无行尾内联注释），整行剥离即零误伤。
    """
    return "\n".join(
        line for line in text.splitlines()
        if not line.lstrip().startswith((";", "#"))
    )


def section(text: str, header: str) -> str:
    """取 NSIS Section 体（含头，止于下一个 Section/Function 或 EOF）。"""
    start = text.index(header)
    rest = text[start:]
    nxt = re.search(r"^(Section|Function)\b", rest[len(header):], re.M)
    return rest if nxt is None else rest[: len(header) + nxt.start()]


class PerUserSurfaceTests(unittest.TestCase):
    """行为 0（承载面）：全程 HKCU、免管理员、per-user 落位。"""

    @classmethod
    def setUpClass(cls):
        assert TPL.is_file(), f"模板缺失：{TPL}"
        cls.text = strip_comments(tpl_text())

    def test_no_hklm_machine_level_writes(self):
        """全模板零 HKLM（安装段/卸载段/InstallDirRegKey 一并钉）。"""
        self.assertNotIn("HKLM", self.text, "HKLM 机器级写入必须清零（#143 per-user 面）")

    def test_request_execution_level_user(self):
        """免管理员：RequestExecutionLevel user（非 admin/none）。"""
        self.assertIn("RequestExecutionLevel user", self.text)
        self.assertNotIn("RequestExecutionLevel admin", self.text)
        self.assertNotIn("RequestExecutionLevel none", self.text)

    def test_install_dir_is_per_user_location(self):
        """落位 $LOCALAPPDATA\\Programs（用户级惯例位，免提权可写）。"""
        self.assertIn('InstallDir "$LOCALAPPDATA\\Programs\\${APPEXEC}"', self.text)
        self.assertNotIn("PROGRAMFILES64", self.text)
        self.assertIn('InstallDirRegKey HKCU "Software\\${APPID}" "InstallDir"', self.text)

    def test_shell_context_current(self):
        """开始菜单写当前用户 shell 上下文（安装/卸载两段都在）。"""
        self.assertEqual(self.text.count("SetShellVarContext current"), 2,
                         "安装与卸载两段各须显式钉 current")


class PoliteRegistrationTests(unittest.TestCase):
    """行为 1/2/3：礼貌关联——只在无处理程序（或已指向自己）时设默认与
    「新建」；「默认应用」面登记而不接管；他人的登记与「新建」不动。"""

    @classmethod
    def setUpClass(cls):
        assert TPL.is_file(), f"模板缺失：{TPL}"
        cls.text = strip_comments(tpl_text())

    def test_reads_existing_md_handler_before_writes(self):
        """.md 写入前必须先读现有处理程序（HKCR 合成视图 = 生效面）。"""
        read_at = self.text.index('ReadRegStr $0 HKCR ".md" ""')
        write_at = self.text.index('WriteRegStr HKCU "Software\\Classes\\.md"')
        self.assertLess(read_at, write_at, "先读后写次序是礼貌注册的地基")

    def test_takeover_guard_checks_foreign_handler(self):
        """接管守卫：空或已指向自己 → md_take；他人非空 → md_keep 不写。"""
        self.assertIn('StrCmp $0 "" md_take 0', self.text)
        self.assertIn('StrCmp $0 "${PROGID}" md_take md_keep', self.text)

    def test_md_keep_branch_writes_nothing(self):
        """md_keep 分支只 DetailPrint——不写任何注册表（他人登记不动）。"""
        m = re.search(r"^md_keep:\n(.*?)(?=^md_cap:)", self.text, re.M | re.S)
        self.assertIsNotNone(m, "md_keep 分支缺失")
        branch = m.group(1)
        self.assertNotIn("WriteRegStr", branch, "md_keep 不得写注册表")
        self.assertNotIn("ShellNew", branch, "md_keep 不得动「新建」菜单")
        self.assertIn("保留用户选择", branch)

    def test_shellnew_follows_takeover_only(self):
        """「新建」菜单（ShellNew）只在 md_take 分支内。"""
        take = re.search(r"^md_take:\n(.*?)(?=^md_keep:)", self.text, re.M | re.S)
        self.assertIsNotNone(take)
        self.assertIn("ShellNew", take.group(1), "接管分支必须含 ShellNew")
        take_end = take.end()
        shellnew_at = self.text.index("ShellNew")
        self.assertLess(shellnew_at, take_end, "ShellNew 必须位于 md_take 分支内")

    def test_default_apps_listing_without_takeover(self):
        """进「默认应用」列表（RegisteredApplications + Capabilities）而不接管。"""
        self.assertIn('WriteRegStr HKCU "Software\\RegisteredApplications"', self.text)
        self.assertIn('Capabilities\\FileAssociations" ".md" "${PROGID}"', self.text)

    def test_write_failure_aborts_install(self):
        """行为 4：写不进注册表即安装失败（IfErrors → MessageBox → Abort）。"""
        self.assertIn("IfErrors reg_write_failed", self.text)
        self.assertIn("reg_write_failed:", self.text)
        fail = re.search(r"^reg_write_failed:\n(.*?)(?=^reg_write_done:)", self.text, re.M | re.S)
        self.assertIsNotNone(fail)
        self.assertIn("Abort", fail.group(1), "失败路径必须 Abort（不静默继续）")


class PoliteUninstallTests(unittest.TestCase):
    """行为 5/6：换文件夹后旧卸载不删新的；卸载只清自己写入的键。"""

    @classmethod
    def setUpClass(cls):
        assert TPL.is_file(), f"模板缺失：{TPL}"
        cls.un = strip_comments(section(tpl_text(), 'Section "Uninstall"'))

    def test_no_hkcr_deletes(self):
        """卸载零 HKCR 裸删（旧模板 DeleteRegKey HKCR "MarkdownFile" 缺陷）。"""
        self.assertNotIn("HKCR", self.un)

    def test_md_default_deleted_only_if_still_ours(self):
        """.md 默认值仅当仍指向本程序 progid 才清理（他人接管 → 不动）。"""
        self.assertIn('ReadRegStr $0 HKCU "Software\\Classes\\.md" ""', self.un)
        self.assertIn('StrCmp $0 "${PROGID}" 0 md_keep_un', self.un)
        self.assertLess(self.un.index('StrCmp $0 "${PROGID}" 0 md_keep_un'),
                        self.un.index('DeleteRegKey HKCU "Software\\Classes\\.md"'),
                        "先守卫后删除")

    def test_uninstall_deletes_files_not_directory_tree(self):
        """载荷按文件删 + 非递归收目录（换文件夹后旧卸载不删新的）。"""
        for rel in ('Delete "$INSTDIR\\${APPEXEC}.exe"',
                    'Delete "$INSTDIR\\pdfium.dll"',
                    'Delete "$INSTDIR\\uninstall.exe"',
                    'RMDir "$INSTDIR"'):
            self.assertIn(rel, self.un)
        self.assertNotIn('RMDir /r "$INSTDIR"', self.un,
                         "RMDir /r $INSTDIR 会连带用户放入同目录的文件")

    def test_uninstall_cleans_own_keys_only(self):
        """只清自写键：软件面/Capabilities/RegisteredApplications/progid。"""
        for frag in ('DeleteRegKey HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${APPID}"',
                     'DeleteRegKey HKCU "Software\\${APPID}"',
                     'DeleteRegValue HKCU "Software\\RegisteredApplications" "${APPNAME}"',
                     'DeleteRegKey HKCU "Software\\Classes\\${PROGID}"'):
            self.assertIn(frag, self.un)


class RenderedScriptTests(unittest.TestCase):
    """② 渲染产物断言：真实 builder 走查后，核心钉子在 installer.nsi 上复验。"""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="nsi-polite-")
        cls.tree = Path(cls.tmp.name) / "checkout"
        for rel in ("packaging/nsis/build-installer.py",
                    "packaging/nsis/installer.nsi.tpl",
                    "packaging/version.py"):
            dst = cls.tree / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / rel, dst)
        (cls.tree / "Cargo.toml").write_text(
            "[workspace.package]\nversion = \"0.1.0\"\n", encoding="utf-8")
        exe = cls.tree / "mang-shell/target/release/mang-shell.exe"
        exe.parent.mkdir(parents=True)
        exe.write_bytes(b"stub exe")
        dll = cls.tree / "bin/lib/pdfium.dll"
        dll.parent.mkdir(parents=True)
        dll.write_bytes(b"stub dll")
        proc = subprocess.run(
            [sys.executable, str(cls.tree / "packaging/nsis/build-installer.py"),
             "--skip-makensis"],
            capture_output=True, text=True, encoding="utf-8", timeout=120)
        assert proc.returncode == 0, f"builder 失败：{proc.stdout}{proc.stderr}"
        cls.nsi = (cls.tree / "build/nsis/installer.nsi").read_text(encoding="utf-8-sig")

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_rendered_preserves_per_user_surface(self):
        nsi = strip_comments(self.nsi)
        self.assertNotIn("HKLM", nsi)
        self.assertIn("RequestExecutionLevel user", nsi)
        self.assertIn('InstallDir "$LOCALAPPDATA\\Programs\\${APPEXEC}"', nsi)

    def test_rendered_preserves_polite_guard(self):
        nsi = strip_comments(self.nsi)
        self.assertIn('ReadRegStr $0 HKCR ".md" ""', nsi)
        self.assertIn('StrCmp $0 "${PROGID}" md_take md_keep', nsi)
        self.assertIn("IfErrors reg_write_failed", nsi)

    def test_rendered_placeholder_zero_residue(self):
        self.assertEqual(self.nsi.count("{{"), 0, "渲染产物不得残留占位符")

    def test_makensis_compiles_rendered_script(self):
        """③ 编译面：makensis 可得时编译（Debian nsis 3.11 本地已验，
        choco nsis 3.x 为 Windows CI 面）；不可得如实 SKIP。"""
        makensis = shutil.which("makensis")
        if makensis is None:
            for cand in (r"C:\Program Files (x86)\NSIS\makensis.exe",
                         r"C:\Program Files\NSIS\makensis.exe"):
                if Path(cand).is_file():
                    makensis = cand
                    break
        if makensis is None:
            self.skipTest("makensis 不可得（安装：Debian `apt-get install nsis` / Windows `choco install nsis`）")
        proc = subprocess.run([makensis, str(self.tree / "build/nsis/installer.nsi")],
                              capture_output=True, text=True, encoding="utf-8", timeout=300)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
