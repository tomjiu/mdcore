//! mdc-lok-smoke —— LOK 真实运行时冒烟（#633 验收链）。
//!
//! 用法：`mdc-lok-smoke [--fixture <docx>] [--out <dir>] [--worker <bin>]`
//! 默认夹具 = crate tests/fixtures/smoke-chars-img-table.docx（中文+图片+表格）。
//! 缺运行时时退出码 3 并给补救指引（测试层会转 SKIP 横幅）。

use std::path::PathBuf;

fn main() {
    let mut fixture: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut worker: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--fixture" => fixture = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--worker" => worker = args.next().map(PathBuf::from),
            _ => {}
        }
    }
    let fixture = fixture.unwrap_or_else(mdc_lok::smoke_fixture_path);
    let out = out.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("mdc-lok-smoke-{}", std::process::id()))
    });
    if !fixture.is_file() {
        eprintln!("[smoke] 夹具不存在：{fixture:?}（--fixture 指定）");
        std::process::exit(3);
    }
    println!("[smoke] 夹具：{fixture:?}\n[smoke] 诊断目录：{out:?}");
    match mdc_lok::smoke::run(&fixture, &out, worker.as_deref()) {
        Ok(report) if report.ok => {
            println!("[smoke] PASS（诊断目录：{}）", report.out_dir.display());
            std::process::exit(0);
        }
        Ok(report) => {
            eprintln!(
                "[smoke] FAIL（失败步骤见 {}）",
                report.out_dir.join("report.json").display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("[smoke] 链路中止：{e}");
            std::process::exit(2);
        }
    }
}
