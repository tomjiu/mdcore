"""Windows packaging CLI/process regressions; fixture hosts do not test GPUI."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(os.environ.get("PACKAGING_UNDER_TEST", Path(__file__).resolve().parents[2]))

HOST_SOURCE = r'''
using System;
using System.IO;
using System.Text;
using System.Windows.Forms;
public static class FixtureHost {
    [STAThread] public static int Main(string[] args) {
        Console.OutputEncoding = new UTF8Encoding(false);
        if (Array.IndexOf(args, "--version") >= 0) { Console.WriteLine("mang-shell 0.1.0"); return 0; }
        if (Array.IndexOf(args, "--check-pdfium") >= 0) { Console.WriteLine("fixture deployment check"); return 0; }
        if (Environment.GetEnvironmentVariable("PORTABLE_FIXTURE_MODE") == "needs-source" &&
            !File.Exists(Environment.GetEnvironmentVariable("PORTABLE_FIXTURE_SHADER"))) {
            Console.Error.WriteLine("fixture shader path unavailable (os error 3)"); return 101;
        }
        int secondsAt = Array.IndexOf(args, "--smoke-sec");
        int millis = (int)(double.Parse(args[secondsAt + 1], System.Globalization.CultureInfo.InvariantCulture) * 1000);
        Form form = new Form(); form.Text = "mang"; form.Width = 320; form.Height = 200;
        Timer opened = new Timer(); opened.Interval = 200;
        opened.Tick += delegate {
            opened.Stop(); int count = 0;
            for (int i = 0; i + 1 < args.Length; ++i) if (args[i] == "--smoke-open") {
                string path = args[++i]; byte[] contents = File.ReadAllBytes(path);
                if (contents.Length == 0) throw new Exception("empty fixture");
                Console.Error.WriteLine("open: " + path);
                if (path.EndsWith(".pdf")) Console.Error.WriteLine("pdf: 1 页 / 17 字符 / fixture host");
                Console.Error.WriteLine("tabs: open → " + (++count) + " 开（active 0）");
            }
            Console.Error.WriteLine("tabs: cycle → active 0/2（fixture）");
            Console.Error.WriteLine("tabs: close → 1 开（active 0）");
        };
        Timer quit = new Timer(); quit.Interval = millis;
        quit.Tick += delegate { quit.Stop(); form.Close(); };
        form.Shown += delegate { opened.Start(); if (Environment.GetEnvironmentVariable("PORTABLE_FIXTURE_MODE") != "never-exits") quit.Start(); };
        Application.Run(form); Console.WriteLine("smoke: quit reached"); return 0;
    }
}
'''


@unittest.skipUnless(os.name == "nt", "Windows process/window tests")
class PortableInstallerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="portable-package-tests-")
        cls.fixture_root = Path(cls.temporary.name)
        cls.host = cls.fixture_root / "fixture-host.exe"
        source = cls.fixture_root / "FixtureHost.cs"
        source.write_text(HOST_SOURCE, encoding="utf-8-sig")
        compiler = Path(os.environ["WINDIR"]) / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
        compiled = subprocess.run([str(compiler), "/nologo", "/target:exe", "/reference:System.Windows.Forms.dll", "/out:" + str(cls.host), str(source)], capture_output=True, text=True)
        if compiled.returncode:
            raise RuntimeError(compiled.stdout + compiled.stderr)

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def setUp(self):
        self.case = Path(tempfile.mkdtemp(prefix="case-", dir=self.fixture_root))
        self.checkout = self.case / "checkout"
        self.shader_dir = self.checkout / "third_party/zed/crates/gpui_windows/src"
        self.shader_dir.mkdir(parents=True)
        self.shader = self.shader_dir / "shaders.hlsl"
        self.shader.write_text("test source", encoding="utf-8")
        self.payload = self.case / "payload"
        self.payload.mkdir()
        shutil.copyfile(self.host, self.payload / "mdcore.exe")
        (self.payload / "pdfium.dll").write_bytes(b"fixture DLL - process host controls deployment result")

    def smoke(self, mode="portable", installer=None):
        script = ROOT / "packaging/nsis/portable-smoke.py"
        self.assertTrue(script.is_file(), "installer needs a real relocated process/window smoke gate")
        output = self.case / "evidence"
        command = [sys.executable, str(script), "--checkout-root", str(self.checkout), "--expected-version", "0.1.0", "--output", str(output), "--smoke-seconds", "1", "--timeout", "3"]
        command += ["--installer", str(installer), "--expected-payload", str(self.payload)] if installer else ["--payload", str(self.payload)]
        env = dict(os.environ, PORTABLE_FIXTURE_MODE=mode, PORTABLE_FIXTURE_SHADER=str(self.shader))
        result = subprocess.run(command, env=env, capture_output=True, text=True, encoding="utf-8", timeout=15)
        self.assertTrue(self.shader.is_file(), "source must be restored even after startup/timeout failure")
        self.assertEqual(self.shader.read_text(), "test source")
        report = json.loads((output / "report.json").read_text(encoding="utf-8"))
        return result, report

    def builder_tree(self):
        tree = self.case / "builder"
        for relative in ["packaging/nsis/build-installer.py", "packaging/nsis/installer.nsi.tpl", "packaging/version.py"]:
            destination = tree / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, destination)
        (tree / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n', encoding="utf-8")
        executable = tree / "mang-shell/target/release/mang-shell.exe"
        executable.parent.mkdir(parents=True)
        shutil.copyfile(self.host, executable)
        dll = tree / "bin/lib/pdfium.dll"
        dll.parent.mkdir(parents=True)
        shutil.copyfile(self.payload / "pdfium.dll", dll)
        return tree

    def test_builder_default_stages_nested_release_bytes(self):
        tree = self.builder_tree()
        result = subprocess.run([sys.executable, str(tree / "packaging/nsis/build-installer.py"), "--skip-makensis"], capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((tree / "build/nsis/payload/mdcore.exe").read_bytes(), self.host.read_bytes())
        self.assertIn('APPVERSION "0.1.0"', (tree / "build/nsis/installer.nsi").read_text(encoding="utf-8-sig"))

    def test_responding_window_and_clean_exit_from_relocated_payload(self):
        result, report = self.smoke()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(report["passed"])
        self.assertTrue(report["window"]["responsive"])
        self.assertTrue(report["source_hidden"])
        self.assertEqual(report["exit_code"], 0)

    def test_source_dependent_binary_is_rejected_and_source_restored(self):
        result, report = self.smoke("needs-source")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(report["passed"])
        self.assertEqual(report["exit_code"], 101)
        self.assertIn("os error 3", (self.case / "evidence/app.stderr.log").read_text(encoding="utf-8"))

    def test_nonterminating_process_is_rejected_and_cleaned_up(self):
        result, report = self.smoke("never-exits")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(report["passed"])
        self.assertIn("timeout", report["failure"].lower())
        self.assertIsNotNone(report["exit_code"])

    def test_real_nsis_extracts_exact_payload_then_launches_it(self):
        makensis = Path(r"C:\Program Files (x86)\NSIS\makensis.exe")
        if not makensis.is_file():
            self.skipTest("NSIS unavailable; staged payload process tests still run")
        tree = self.builder_tree()
        built = subprocess.run([sys.executable, str(tree / "packaging/nsis/build-installer.py"), "--makensis", str(makensis)], capture_output=True, text=True, encoding="utf-8")
        self.assertEqual(built.returncode, 0, built.stdout + built.stderr)
        result, report = self.smoke(installer=tree / "build/nsis/mdcore-setup-0.1.0.exe")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(report["payload_matches_stage"])
        self.assertTrue(report["passed"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
