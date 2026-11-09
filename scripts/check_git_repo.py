"""Run production Git repository tests with rustc and Git on either platform."""

import os
from pathlib import Path
import subprocess
import tempfile


def main():
    repo = Path(__file__).resolve().parents[1]
    environment = os.environ.copy()
    environment["CARGO_MANIFEST_DIR"] = str(repo / "mang-shell")

    def run(*arguments):
        subprocess.run(
            [str(argument) for argument in arguments],
            cwd=repo,
            env=environment,
            check=True,
            timeout=180,
        )

    run("git", "--version")
    with tempfile.TemporaryDirectory(prefix="mdcore-git-tests-") as directory:
        temporary = Path(directory)
        suffix = ".exe" if os.name == "nt" else ""
        module = repo / "mang-shell/src/git_repo.rs"
        unit = temporary / f"git-unit{suffix}"
        run("rustc", "--edition=2024", "--test", module, "-o", unit)
        run(unit, "--test-threads=1", "--nocapture")

        # Compile this exact production module without the native UI dependency.
        fence = "#"
        while f'"{fence}' in module.as_posix():
            fence += "#"
        library_source = temporary / "lib.rs"
        library_source.write_text(
            f'#[path = r{fence}"{module.as_posix()}"{fence}]\npub mod git_repo;\n',
            encoding="utf-8",
        )
        library = temporary / "libmang_shell.rlib"
        run("rustc", "--edition=2024", "--crate-name=mang_shell", "--crate-type=rlib",
            library_source, "-o", library)
        integration = temporary / f"git-integration{suffix}"
        run("rustc", "--edition=2024", "--test",
            repo / "mang-shell/tests/git_diff_real_repo.rs",
            "--extern", f"mang_shell={library}", "-o", integration)
        run(integration, "--test-threads=1", "--nocapture")


if __name__ == "__main__":
    main()
