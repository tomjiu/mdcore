//! Home-only Shell hooks. Existing file/tree/tab operation semantics stay intact.
use super::*;

impl ShellApp {
    pub(super) fn activate_home_action(
        &mut self,
        action: home_view::HomeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            home_view::HomeAction::OpenFile => self.prompt_open_file(window, cx),
            home_view::HomeAction::OpenFolder => self.prompt_open_workspace(window, cx),
            home_view::HomeAction::NewMarkdown => self.prompt_new_markdown(window, cx),
        }
    }

    fn prompt_new_markdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self
            .workspace
            .as_ref()
            .map(|tree| tree.read(cx).root().to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let rx = cx.prompt_for_new_path(&directory, Some("untitled.md"));
        cx.spawn_in(window, async move |this, cx| {
            match rx.await {
                Ok(Ok(Some(path))) => {
                    let _ = this.update_in(cx, |shell, window, cx| {
                        shell.create_home_markdown(path, window, cx);
                    });
                }
                Ok(Err(err)) => {
                    let _ = this.update(cx, |shell, cx| {
                        shell.show_notice(format!("新建 Markdown 选择器不可用：{err}"), cx);
                    });
                }
                // Cancel and dropped prompt channels are silent no-ops.
                Ok(Ok(None)) | Err(_) => {}
            }
        })
        .detach();
    }

    fn create_home_markdown(
        &mut self,
        mut path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None => {
                path.set_extension("md");
            }
            Some("md" | "markdown") => {}
            _ => {
                self.show_notice(
                    "新建 Markdown 失败：请选择 .md 或 .markdown 文件名".into(),
                    cx,
                );
                return;
            }
        }
        // Atomic exclusive creation: no exists-then-truncate race; reject files,
        // directories and symlinks already occupying the selected destination.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                drop(file);
                self.sync_tree_after_fs(&path, false, cx);
                self.open_document(path, window, cx);
            }
            Err(err) => self.show_notice(
                format!("新建 Markdown 失败（{}）：{err}", path.display()),
                cx,
            ),
        }
    }
}
