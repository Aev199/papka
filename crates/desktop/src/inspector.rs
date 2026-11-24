use crate::*;

/// What the details panel shows for the selected item. All of it is read
/// off the UI thread; `request` tells a late answer from the current one.
#[derive(Default)]
pub(crate) struct InspectorState {
    pub(crate) path: Option<PathBuf>,
    pub(crate) request: u64,
    pub(crate) loading: bool,
    pub(crate) dirty: bool,
    pub(crate) preview: Option<(String, String)>,
    /// Shell thumbnail (PNG in the cache) for the selected file, if any.
    pub(crate) thumbnail: Option<PathBuf>,
    pub(crate) details_busy: bool,
    pub(crate) thumbnail_busy: bool,
    pub(crate) history: Vec<Event>,
    /// Other files that are versions of the selected document, newest first.
    pub(crate) versions: Vec<filemanager_core::versions::Member>,
}

impl Explorer {
    pub(crate) fn load_selected_thumbnail(&mut self, cx: &mut Context<Self>) {
        if self.inspector.thumbnail_busy || self.inspector.thumbnail.is_some() {
            return;
        }
        let Some(path) = self.selected.clone().filter(|p| wants_shell_thumbnail(p) && !filemanager_core::archive::is_inside(p)) else {
            return;
        };
        let request = self.inspector.request;
        self.inspector.thumbnail_busy = true;
        let selected = path.clone();
        let task = cx.background_spawn(async move {
            std::env::current_exe().and_then(|exe| {
                filemanager_core::preview_host::isolated_thumbnail(&exe, &path, 320)
            })
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.inspector.thumbnail_busy = false;
                if this.inspector.request != request || this.selected.as_ref() != Some(&selected) {
                    this.load_selected_thumbnail(cx);
                    return;
                }
                this.inspector.thumbnail = result.ok();
                if this.inspector.thumbnail.is_some() { this.motion.picture_arrived(format!("inspector:{}", selected.display())); }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn open_document_preview(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.preview_launch_busy {
            return;
        }
        self.document_preview.take();
        self.preview_launch_busy = true;
        self.preview_launch_generation = self.preview_launch_generation.wrapping_add(1);
        let generation = self.preview_launch_generation;
        self.status = "Открывается предпросмотр документа…".into();
        let task = cx.background_spawn(async move {
            std::env::current_exe().and_then(|exe| {
                filemanager_core::preview_host::PreviewProcess::document(&exe, &path)
            })
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.preview_launch_busy = false;
                if this.preview_launch_generation != generation {
                    return;
                }
                match result {
                    Ok(child) => {
                        this.document_preview = Some(child);
                        this.status =
                            "Предпросмотр открыт в отдельном окне. Его можно закрыть здесь.".into();
                        this.watch_document_preview(generation, cx);
                    }
                    Err(error) => this.status = format!("Предпросмотр недоступен: {error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// Notices when the user closes the preview window themselves (or the
    /// handler fails), so the panel stops offering "close" for a dead window.
    fn watch_document_preview(&mut self, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                let keep_watching = weak
                    .update(cx, |this, cx| {
                        if this.preview_launch_generation != generation {
                            return false;
                        }
                        let Some(preview) = this.document_preview.as_mut() else {
                            return false;
                        };
                        match preview.exited() {
                            Ok(None) => true,
                            Ok(Some(status)) => {
                                this.document_preview = None;
                                this.status = if status.success() {
                                    "Окно предпросмотра закрыто".into()
                                } else {
                                    "Предпросмотр завершился с ошибкой обработчика; файл можно открыть обычной командой".into()
                                };
                                cx.notify();
                                false
                            }
                            Err(error) => {
                                this.document_preview = None;
                                this.status = format!("Состояние предпросмотра неизвестно: {error}");
                                cx.notify();
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !keep_watching {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_document_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_launch_generation = self.preview_launch_generation.wrapping_add(1);
        self.document_preview.take();
        self.status = "Предпросмотр закрыт".into();
        cx.notify();
    }
}
