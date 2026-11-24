//! Quick look on Space, as in macOS and QuickLook for Windows: a large
//! preview over the window that follows the selection. Pictures show as
//! they are, drawings by the picture inside the DWG, text by its start,
//! documents by the Windows thumbnail of their first page.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Context};

use crate::Explorer;

#[derive(Clone)]
pub(crate) enum QuickContent {
    Loading,
    Image(PathBuf),
    Text(String),
    Word(Arc<filemanager_core::word_text::Preview>),
    Message(String),
    /// A CSV file or a sheet of a workbook.
    Table(Arc<filemanager_core::table::Table>),
    /// A page of a PDF, drawn by Windows.
    Pdf { image: PathBuf, page: u32, pages: u32 },
}

pub(crate) struct QuickLook {
    pub(crate) path: PathBuf,
    pub(crate) content: QuickContent,
    /// Kept beneath the incoming body until its fade completes.
    pub(crate) previous: Option<QuickContent>,
    /// A picture or a page enlarged: 1 fits the window.
    pub(crate) zoom: f32,
    /// Where the enlarged picture's corner sits in its frame.
    pub(crate) pan: gpui::Point<f32>,
    /// The frame, as last drawn: zoom keeps the point under the cursor.
    pub(crate) frame: gpui::Bounds<gpui::Pixels>,
    /// The pointer while the picture is dragged.
    pub(crate) grab: Option<gpui::Point<gpui::Pixels>>,
    /// Unique across replaced previews so an older async load cannot win,
    /// even when the same path is opened twice.
    request_id: u64,
    transition: PreviewTransition,
    shell: crate::layout_motion::Tween,
    request_pending: bool,
    pending_content: Option<QuickContent>,
    closing: bool,
    pub(crate) previous_pose: (f32,gpui::Point<f32>),
}

const PREVIEW_FADE: Duration = crate::motion_policy::PREVIEW_FADE;
static NEXT_PREVIEW_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy)]
enum PreviewTransition {
    Stable,
    FadeIn(Instant),
}

/// `sample` is the render contract: use `shell_alpha` for backdrop/card,
/// `content_alpha` for newly loaded content, and show a small loading mark
/// when `loading` is true. Keep requesting frames while `active`; remove the
/// QuickLook state when `finished` is true.
impl QuickLook {
    pub(crate) fn sample(&mut self, now: Instant, animations: bool) -> (f32, f32, bool, bool, bool) {
        let closing=self.closing();
        let (shell_alpha,shell_active)=self.shell.sample(if closing {0.}else{1.},!animations,now,crate::motion_policy::PANE_CLOSE_MS);
        let content_active=matches!(self.transition,PreviewTransition::FadeIn(at) if animations && now.saturating_duration_since(at)<PREVIEW_FADE);
        if matches!(self.transition,PreviewTransition::FadeIn(_)) && !content_active {self.transition=PreviewTransition::Stable;}
        let active=shell_active || content_active;
        let finished=closing && !shell_active;
        let content_alpha = match self.transition {
            PreviewTransition::FadeIn(started) => if animations {
                ((now - started).as_secs_f32() / PREVIEW_FADE.as_secs_f32()).clamp(0., 1.)
            } else { 1. },
            _ => 1.,
        };
        if !matches!(self.transition, PreviewTransition::FadeIn(_)) || content_alpha >= 1. {
            self.previous = None;
        }
        (shell_alpha, content_alpha, self.request_pending, active, finished)
    }

    fn closing(&self) -> bool { self.closing }

    fn stage(&mut self,content:QuickContent) {
        if image_path(&content).is_some() {self.pending_content=Some(content);self.request_pending=true;}
        else {self.present(content,Instant::now());}
    }
    fn present(&mut self,content:QuickContent,now:Instant) {
        self.previous=if matches!(self.content,QuickContent::Loading){None}else{Some(self.content.clone())};
        self.previous_pose=(self.zoom,self.pan);
        self.content=content;self.pending_content=None;self.request_pending=false;
        self.zoom=1.;self.pan=gpui::point(0.,0.);self.grab=None;
        self.transition=PreviewTransition::FadeIn(now);
    }
    fn begin_load(&mut self) -> u64 {
        self.request_pending = true;
        self.pending_content = None;
        self.next_request()
    }
}

impl QuickLook {
    fn new(path: PathBuf, content: QuickContent) -> Self {
        let loading = matches!(&content, QuickContent::Loading);
        let transition = if loading { PreviewTransition::Stable } else { PreviewTransition::FadeIn(Instant::now()) };
        Self { path, content, previous: None, zoom: 1., pan: gpui::point(0., 0.), frame: gpui::Bounds::default(), grab: None,
            request_id: NEXT_PREVIEW_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed), transition,
            request_pending: loading, shell:crate::layout_motion::Tween::default(), pending_content:None, closing:false,
            previous_pose:(1.,gpui::point(0.,0.)) }
    }

    fn next_request(&mut self) -> u64 {
        let id = NEXT_PREVIEW_REQUEST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.request_id = id;
        id
    }

    /// Enlarges by `factor` around `at` (window position); 1 fits again.
    pub(crate) fn zoom_at(&mut self, factor: f32, at: gpui::Point<gpui::Pixels>) {
        let zoom = (self.zoom * factor).clamp(1., 10.);
        let cursor = gpui::point((at.x - self.frame.origin.x) / gpui::px(1.), (at.y - self.frame.origin.y) / gpui::px(1.));
        // The picture point under the cursor stays under it.
        let scale = zoom / self.zoom;
        self.pan = gpui::point(cursor.x - (cursor.x - self.pan.x) * scale, cursor.y - (cursor.y - self.pan.y) * scale);
        self.zoom = zoom;
        self.clamp_pan();
    }

    pub(crate) fn drag_to(&mut self, at: gpui::Point<gpui::Pixels>) {
        let Some(from) = self.grab.replace(at) else { return };
        self.pan = gpui::point(self.pan.x + (at.x - from.x) / gpui::px(1.), self.pan.y + (at.y - from.y) / gpui::px(1.));
        self.clamp_pan();
    }

    /// The enlarged picture never leaves a gap at the frame's edges.
    fn clamp_pan(&mut self) {
        if self.zoom <= 1. { self.zoom = 1.; self.pan = gpui::point(0., 0.); return; }
        let (w, h) = (self.frame.size.width / gpui::px(1.), self.frame.size.height / gpui::px(1.));
        self.pan = gpui::point(self.pan.x.clamp(w - w * self.zoom, 0.), self.pan.y.clamp(h - h * self.zoom, 0.));
    }

    pub(crate) fn zoomable(&self) -> bool {
        matches!(self.content, QuickContent::Image(_) | QuickContent::Pdf { .. })
    }
}

const TEXT_LIMIT: usize = 64 * 1024;
/// Cells of a table shown at most.
const TABLE_ROWS: usize = 200;
const TABLE_COLUMNS: usize = 30;
/// Width a PDF page is drawn at, in pixels.
const PDF_WIDTH: u32 = 1400;

/// Page `page` of a PDF as a PNG file in the thumbnail cache.
fn pdf_page(path: &Path, page: u32) -> QuickContent {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok().hash(&mut hash);
    match filemanager_core::pdf::render_page(path, page, PDF_WIDTH) {
        Ok((pages, png)) => {
            let page = page.min(pages.saturating_sub(1));
            let folder = filemanager_core::thumbnail::cache_dir();
            let file = folder.join(format!("pdf-{:016x}-{page}.png", hash.finish()));
            match std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(&file, png)) {
                Ok(()) => QuickContent::Pdf { image: file, page, pages },
                Err(error) => QuickContent::Message(format!("Страницу не удалось сохранить: {error}")),
            }
        }
        // Without the Windows PDF engine: the first page from the thumbnail.
        Err(_) => document_thumbnail(path),
    }
}

fn document_thumbnail(path: &Path) -> QuickContent {
    std::env::current_exe()
        .and_then(|exe| filemanager_core::preview_host::isolated_thumbnail(&exe, path, 1024))
        .map(QuickContent::Image)
        .unwrap_or_else(|_| QuickContent::Message("Для этого файла Windows не дала картинку. «Подробный предпросмотр» откроет его целиком.".into()))
}

fn table(path: &Path, sheet: usize) -> QuickContent {
    match filemanager_core::table::read(path, sheet, TABLE_ROWS, TABLE_COLUMNS) {
        Ok(table) if table.rows.is_empty() => QuickContent::Message("Лист пуст".into()),
        Ok(table) => QuickContent::Table(Arc::new(table)),
        Err(error) => QuickContent::Message(format!("Таблицу прочитать не удалось: {error}")),
    }
}

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn is_picture(path: &Path) -> bool {
    matches!(extension(path).as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp")
}

/// Documents that have a preview window too.
pub(crate) fn is_document(path: &Path) -> bool {
    matches!(extension(path).as_str(), "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "rtf" | "odt" | "ods" | "odp")
}

/// Text when the start of the file reads as text, else `None`.
fn read_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(TEXT_LIMIT as u64).read_to_end(&mut bytes).ok()?;
    if bytes.contains(&0) { return None; }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let odd = text.chars().filter(|c| *c == '\u{FFFD}' || (c.is_control() && !c.is_whitespace())).count();
    (odd * 50 <= text.chars().count().max(1)).then_some(text)
}

fn load(path: &Path) -> QuickContent {
    if path.is_dir() {
        let count = std::fs::read_dir(path).map(|items| items.count()).unwrap_or(0);
        return QuickContent::Message(format!("Папка, элементов: {count}"));
    }
    if extension(path) == "dwg" {
        return match filemanager_core::thumbnail::thumbnail(path, 1024) {
            Ok(png) => QuickContent::Image(png),
            Err(error) => QuickContent::Message(format!("Картинку чертежа прочитать не удалось: {error}")),
        };
    }
    if filemanager_core::table::is_table(path) {
        return table(path, 0);
    }
    if extension(path) == "pdf" {
        return pdf_page(path, 0);
    }
    if filemanager_core::word_text::supports(path) {
        return match filemanager_core::word_text::read(path) {
            Ok(preview) => QuickContent::Word(Arc::new(preview)),
            Err(error) => QuickContent::Message(format!("Текст документа прочитать не удалось: {error}. «Подробный предпросмотр» использует обработчик Windows.")),
        };
    }
    if is_document(path) || matches!(extension(path).as_str(), "tif" | "tiff" | "heic" | "psd" | "svg" | "mp4" | "mov" | "avi" | "mkv") {
        return document_thumbnail(path);
    }
    match read_text(path) {
        Some(text) => QuickContent::Text(text),
        None => QuickContent::Message("Быстрый просмотр для этого типа файлов недоступен.".into()),
    }
}

fn image_path(content:&QuickContent)->Option<&PathBuf> {
    match content {QuickContent::Image(path)=>Some(path),QuickContent::Pdf{image,..}=>Some(image),_=>None}
}

impl Explorer {
    /// Use the same async asset/cache as `img`: a path alone is not a ready picture.
    pub(crate) fn prepare_quick_look(&mut self,window:&mut gpui::Window,cx:&mut Context<Self>) {
        let Some(look)=self.quick_look.as_mut().filter(|look|!look.closing()) else{return};
        let Some(path)=look.pending_content.as_ref().and_then(image_path) else{return};
        let resource=gpui::Resource::Path(path.clone().into());
        let Some(result)=window.use_asset::<gpui::ImgResourceLoader>(&resource,cx) else{return};
        let content=match result {
            Ok(_)=>look.pending_content.take().unwrap(),
            Err(_)=>QuickContent::Message("Изображение прочитать не удалось: файл повреждён или недоступен.".into()),
        };
        look.present(content,Instant::now());
    }

    /// Space: open the quick look for the selection, or close it.
    pub(crate) fn toggle_quick_look(&mut self, cx: &mut Context<Self>) {
        if self.quick_look.as_ref().is_some_and(|look|!look.closing()) { self.close_quick_look(cx); return; }
        if let Some(path) = self.selected.clone() { self.show_quick_look(path, cx); }
    }

    /// Starts the closing fade. The renderer removes the state when
    /// `QuickLook::sample` reports `finished`.
    pub(crate) fn close_quick_look(&mut self, cx: &mut Context<Self>) {
        if let Some(look) = self.quick_look.as_mut() {
            if look.closing() { return; }
            look.next_request(); // invalidate any load still in flight
            look.closing = true;
            look.pending_content = None;
            look.request_pending = false;
            cx.notify();
        }
    }

    /// Keeps the quick look on the selected file as the selection moves.
    pub(crate) fn follow_quick_look(&mut self, cx: &mut Context<Self>) {
        let Some(look) = &self.quick_look else { return };
        if look.closing() { return; }
        match self.selected.clone() {
            Some(path) if path != look.path => self.show_quick_look(path, cx),
            Some(_) => {}
            None => self.close_quick_look(cx),
        }
    }

    /// Whether ← → turn pages now.
    pub(crate) fn quick_look_paged(&self) -> bool {
        self.quick_look.as_ref().is_some_and(|look| !look.closing() && !look.request_pending && matches!(look.content, QuickContent::Pdf { pages, .. } if pages > 1))
    }

    /// PageDown / PageUp and the arrows under a PDF: another page.
    pub(crate) fn quick_look_page(&mut self, delta: i64, cx: &mut Context<Self>) {
        let Some(look) = &self.quick_look else { return };
        if look.closing() || look.request_pending { return; }
        let QuickContent::Pdf { page, pages, .. } = look.content else { return };
        let next = (page as i64 + delta).clamp(0, pages.saturating_sub(1) as i64) as u32;
        if next == page { return; }
        let path = look.path.clone();
        self.reload_quick_look(path, move |path| pdf_page(path, next), cx);
    }

    /// A click on a sheet name under a table.
    pub(crate) fn quick_look_sheet(&mut self, sheet: usize, cx: &mut Context<Self>) {
        let Some(look) = self.quick_look.as_ref().filter(|look| !look.closing() && !look.request_pending) else { return };
        let path = look.path.clone();
        self.reload_quick_look(path, move |path| table(path, sheet), cx);
    }

    /// Loads other content for the shown file, keeping the old one meanwhile.
    fn reload_quick_look(&mut self, path: PathBuf, load: impl FnOnce(&Path) -> QuickContent + Send + 'static, cx: &mut Context<Self>) {
        let target = path.clone();
        let request_id = self.quick_look.as_mut().expect("quick look exists").begin_load();
        let task = cx.background_spawn(async move { load(&target) });
        cx.spawn(async move |this, cx| {
            let content = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(look) = this.quick_look.as_mut().filter(|look| look.path == path && look.request_id == request_id) {
                    look.stage(content);
                    cx.notify();
                }
            });
        }).detach();
    }

    fn show_quick_look(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let request_id = if let Some(look) = self.quick_look.as_mut() {
            look.path = path.clone();
            look.closing = false;
            // Neither pose nor the visible crossfade changes until replacement is ready.
            look.begin_load()
        } else {
            self.quick_look = Some(QuickLook::new(path.clone(), QuickContent::Loading));
            self.quick_look.as_ref().unwrap().request_id
        };
        if is_picture(&path) {
            self.quick_look.as_mut().unwrap().stage(QuickContent::Image(path));
            cx.notify();return;
        }
        let target = path.clone();
        let task = cx.background_spawn(async move { load(&target) });
        cx.spawn(async move |this, cx| {
            let content = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(look) = this.quick_look.as_mut().filter(|look| look.path == path && look.request_id == request_id) {
                    look.stage(content);
                    cx.notify();
                }
            });
        }).detach();
        cx.notify();
    }
}

#[cfg(test)]mod motion_tests {
    use super::*;
    #[test]fn close_continues_from_visible_shell_and_disabled_motion_finishes(){
        let mut look=QuickLook::new(PathBuf::from("a.txt"),QuickContent::Text("a".into()));let now=Instant::now();
        assert_eq!(look.sample(now,true).0,0.);
        let mid=now+Duration::from_millis(50);let before=look.sample(mid,true).0;
        look.closing=true;assert_eq!(look.sample(mid,true).0,before);
        let end=look.sample(mid+Duration::from_millis(150),true);assert_eq!(end.0,0.);assert!(end.4);
        assert!(look.sample(mid,false).4);
    }
    #[test]fn image_loading_keeps_visible_content_and_pose_until_ready(){
        let mut look=QuickLook::new(PathBuf::from("a.txt"),QuickContent::Text("old".into()));
        look.zoom=2.;look.pan=gpui::point(-40.,-30.);let now=Instant::now();
        look.begin_load();look.stage(QuickContent::Image(PathBuf::from("large.png")));
        assert!(matches!(&look.content,QuickContent::Text(t) if t=="old"));assert!(look.request_pending);
        assert_eq!(look.zoom,2.);assert_eq!(look.pan,gpui::point(-40.,-30.));
        let ready=look.pending_content.take().unwrap();look.present(ready,now);
        assert_eq!(look.previous_pose,(2.,gpui::point(-40.,-30.)));assert_eq!(look.zoom,1.);assert!(!look.request_pending);
        look.sample(now+PREVIEW_FADE,true);assert!(look.previous.is_none());
    }
    #[test]fn closing_does_not_interrupt_incoming_crossfade(){
        let mut look=QuickLook::new(PathBuf::from("a.txt"),QuickContent::Text("old".into()));let now=Instant::now();
        look.present(QuickContent::Text("new".into()),now);let mid=now+PREVIEW_FADE/2;
        let before=look.sample(mid,true).1;look.closing=true;
        assert_eq!(look.sample(mid,true).1,before);assert!(look.previous.is_some());
        look.closing=false;assert!(!look.sample(mid,true).4);
    }
    #[test]fn previous_content_survives_incoming_fade_and_request_ids_are_unique(){
        let mut a=QuickLook::new(PathBuf::from("a.txt"),QuickContent::Text("a".into()));
        let b=QuickLook::new(PathBuf::from("a.txt"),QuickContent::Loading);assert_ne!(a.next_request(),b.request_id);
        let now=Instant::now();a.previous=Some(QuickContent::Text("old".into()));a.transition=PreviewTransition::FadeIn(now);
        assert_eq!(a.sample(now,true).1,0.);assert!(a.previous.is_some());
        assert_eq!(a.sample(now+PREVIEW_FADE,true).1,1.);assert!(a.previous.is_none());
    }
}
