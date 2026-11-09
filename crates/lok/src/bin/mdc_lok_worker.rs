//! mdc-lok-worker —— LOK 隔离 worker 进程入口（#633）。
//!
//! 用法：
//! - 真实模式：`mdc-lok-worker <program_dir> <profile_dir>`
//! - mock 模式：`mdc-lok-worker --mock - <profile_dir>`
//!
//! 宿主（client.rs）负责 spawn 与帧协议；本进程 stderr 留给崩溃
//! 取证（宿主保留尾部行）。退出恒走 `_exit`（见 worker::serve）。

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut mock = false;
    let mut positional: Vec<String> = Vec::new();
    for a in args.iter().skip(1) {
        if a == "--mock" {
            mock = true;
        } else {
            positional.push(a.clone());
        }
    }
    if positional.len() < 2 {
        eprintln!(
            "用法: mdc-lok-worker [--mock] <program_dir|-> <profile_dir>\n\
             真实模式 program_dir = LibreOffice program 目录（如 /usr/lib/libreoffice/program）"
        );
        std::process::exit(2);
    }
    let program_dir = if positional[0] == "-" {
        None
    } else {
        Some(PathBuf::from(&positional[0]))
    };
    let profile_dir = PathBuf::from(&positional[1]);

    if !mock {
        // unipoll 必须先于 hook_2（lo_initialize 读 SAL_LOK_OPTIONS）。
        // 宿主 client.rs 已设；此处兜底手动运行场景（幂等：已含则不动）。
        let opts = std::env::var("SAL_LOK_OPTIONS").unwrap_or_default();
        if !opts.split(':').any(|o| o == "unipoll") {
            std::env::set_var("SAL_LOK_OPTIONS", "unipoll");
        }
    }

    if let Err(e) = mdc_lok::worker::serve(mock, program_dir, profile_dir) {
        eprintln!("mdc-lok-worker: 初始化失败：{e}");
        // 初始化失败：非零退出 + stderr 详情；宿主侧表现为
        // WorkerDied（含 stderr 尾部），错误信息不丢。
        std::process::exit(1);
    }
}
