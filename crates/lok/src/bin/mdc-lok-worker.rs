//! mdc-lok-worker —— LOK 隔离 worker 进程入口（#633）。
//!
//! 由 [`crate::client::WorkerClient`] spawn；不直接人工运行。
//! 参数：
//! - `--session-dir <dir>`：会话目录（profile/ 与 tile-*.bmp side file 落点）
//! - `--kernel-lib <path>` / `--program-dir <dir>` / `--kernel-sha256 <hex>`：
//!   宿主发现的运行时指纹透传（发现序见 [`crate::runtime`]）
//! - `--fake <echo|hang|die>`：协议自测模式（无 LOK，CI/集成测试恒跑）

use std::path::PathBuf;

fn main() -> ! {
    let mut session_dir: Option<PathBuf> = None;
    let mut kernel_lib: Option<PathBuf> = None;
    let mut program_dir: Option<PathBuf> = None;
    let mut kernel_sha = String::new();
    let mut fake: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--session-dir" => session_dir = args.next().map(PathBuf::from),
            "--kernel-lib" => kernel_lib = args.next().map(PathBuf::from),
            "--program-dir" => program_dir = args.next().map(PathBuf::from),
            "--kernel-sha256" => kernel_sha = args.next().unwrap_or_default(),
            "--fake" => fake = args.next(),
            _ => {}
        }
    }

    let boot = match (session_dir, kernel_lib, program_dir) {
        (Some(dir), Some(lib), Some(program)) => Some(mdc_lok::worker::WorkerBoot::new(
            mdc_lok::runtime::LokRuntime {
                program_dir: program,
                kernel_lib: lib,
                kernel_sha256: kernel_sha,
            },
            &dir,
        )),
        (Some(_), _, _) if fake.is_some() => None, // fake 模式允许无运行时
        _ => {
            eprintln!(
                "mdc-lok-worker：缺少参数（--session-dir 必需；真实模式还需 \
                 --kernel-lib/--program-dir；由 WorkerClient spawn，不直接运行）"
            );
            mdc_lok::worker::hard_exit(64);
        }
    };
    mdc_lok::worker::worker_main(boot, fake.as_deref())
}
