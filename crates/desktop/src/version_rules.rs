//! «Шаблоны версий»: which name suffixes mark versions of one document.
//! Opened from the settings; saved at once, apart from them.

use gpui::{div, prelude::*, px, AnyElement, Context, Focusable, MouseButton, Window};
use gpui_component::input::Input;

use crate::theme::*;
use crate::view::{text_button, ButtonKind};
use crate::Explorer;

pub(crate) const TEMPLATES_KEY: &str = "version_templates";
pub(crate) const BUILTIN_KEY: &str = "version_builtin";

/// The editor while it is open.
pub(crate) struct Editing {
    pub builtin: bool,
    pub error: Option<String>,
}

impl Explorer {
    /// Reads the saved rules and hands them to the version parser.
    pub(crate) fn load_version_rules(&mut self) {
        let store = self.workspaces.as_ref();
        self.version_templates = store.and_then(|s| s.value(TEMPLATES_KEY).ok().flatten()).unwrap_or_default();
        self.version_builtin = store.and_then(|s| s.value(BUILTIN_KEY).ok().flatten()).is_none_or(|flag| flag != "0");
        filemanager_core::versions::set_rules(self.version_builtin,
            &filemanager_core::versions::split_templates(&self.version_templates));
    }

    pub(crate) fn open_version_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.version_editing = Some(Editing { builtin: self.version_builtin, error: None });
        let text = self.version_templates.clone();
        self.version_input.update(cx, |input, cx| input.set_value(text, window, cx));
        let focus = self.version_input.focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn close_version_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.version_editing = None;
        // Back to the settings underneath, which keep their own keys.
        if let Some(settings) = &self.settings { window.focus(&settings.focus, cx); }
        cx.notify();
    }

    pub(crate) fn save_version_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.version_editing.as_mut() else { return };
        let text = self.version_input.read(cx).value().to_string();
        let templates = filemanager_core::versions::split_templates(&text);
        let errors = filemanager_core::versions::set_rules(editing.builtin, &templates);
        if !errors.is_empty() {
            editing.error = Some(format!("Не понял: {}. В шаблоне нужен ровно один из {{N}}, {{A}} или {{дата}}.", errors.join("; ")));
            // Keep the rules that were in force.
            filemanager_core::versions::set_rules(self.version_builtin,
                &filemanager_core::versions::split_templates(&self.version_templates));
            cx.notify();
            return;
        }
        self.version_builtin = editing.builtin;
        self.version_templates = templates.join("; ");
        if let Some(store) = &self.workspaces {
            let _ = store.set_value(TEMPLATES_KEY, &self.version_templates);
            let _ = store.set_value(BUILTIN_KEY, if self.version_builtin { "1" } else { "0" });
        }
        // Every listing regroups its versions.
        self.stacks_rev += 1;
        self.status = "Шаблоны версий сохранены".into();
        self.close_version_rules(window, cx);
    }

    /// The settings row: what is in force, and the way to change it.
    pub(crate) fn version_rules_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut summary = Vec::new();
        if self.version_builtin { summary.push("_v_1_6, v2, rev3, в1".to_owned()); }
        if !self.version_templates.is_empty() { summary.push(self.version_templates.clone()); }
        let summary = if summary.is_empty() { "не распознаются".to_owned() } else { summary.join("; ") };
        div().h(px(30.)).px_2().flex().items_center().gap_2()
            .child(div().flex_none().child("Версии файлов"))
            .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(summary))
            .child(text_button("version-rules", "Шаблоны…", ButtonKind::Ghost)
                .on_click(cx.listener(|this, _, window, cx| this.open_version_rules(window, cx))))
    }

    pub(crate) fn version_rules_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editing = self.version_editing.as_ref()?;
        let builtin = editing.builtin;
        let example = |pattern: &'static str, name: &'static str| div().flex().gap_3().text_size(px(12.))
            .child(div().w(px(110.)).flex_none().text_color(rgb(TEXT)).child(pattern))
            .child(div().text_color(rgb(TEXT_MUTED)).child(name));
        let card = div().id("version-rules-dialog").key_context("VersionRules").w(px(520.)).max_w_full().p_4()
            .flex().flex_col().gap_3().rounded(px(6.)).bg(rgb(RAISED)).border_1().border_color(rgb(BORDER_STRONG))
            .text_size(px(13.)).text_color(rgb(TEXT))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(15.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Шаблоны версий"))
            .child(div().text_size(px(12.)).text_color(rgb(TEXT_MUTED))
                .child("Файлы одного документа с разными суффиксами показываются одной строкой — последней версией. \
                    Шаблон — то, что стоит после имени документа."))
            .child(div().id("version-builtin").flex().items_center().gap_2().cursor_pointer()
                .child(div().size(px(16.)).flex_none().rounded(px(2.)).flex().items_center().justify_center()
                    .when(builtin, |b| b.bg(rgb(ACCENT)).child(gpui::svg().path("fm/check.svg").size(px(12.)).text_color(rgb(SELECTION_TEXT))))
                    .when(!builtin, |b| b.border_1().border_color(rgb(BORDER_STRONG))))
                .child("Стандартные: _v_1_6, _v1.6, v2, ver3, rev2, _в1_2")
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(editing) = this.version_editing.as_mut() { editing.builtin = !editing.builtin; cx.notify(); }
                })))
            .child(div().flex().flex_col().gap_1()
                .child(div().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child("Свои шаблоны, через «;»"))
                .child(Input::new(&self.version_input)))
            .child(div().flex().flex_col().gap_1()
                .child(example("_изм{N}", "Отчёт_изм2.docx, Отчёт изм.3.docx"))
                .child(example("({N})", "Отчёт (2).docx"))
                .child(example("_rev{A}", "Схема_revB.dwg, Схема_rev.Б.dwg"))
                .child(example("_{дата}", "План_2026-10-28.pdf, План_28.10.2026.pdf"))
                .child(div().text_size(px(11.)).text_color(rgb(TEXT_DIM))
                    .child("{N} — число (2, 1.6, 1_6), {A} — буква, {дата} — дата. Знаки _ - . и пробел взаимозаменяемы, регистр не важен.")))
            .when_some(editing.error.clone(), |card, error| card.child(div().text_size(px(12.)).text_color(rgb(DANGER)).child(error)))
            .child(div().flex().justify_end().gap_2()
                .child(text_button("version-cancel", "Отмена", ButtonKind::Ghost)
                    .on_click(cx.listener(|this, _, window, cx| this.close_version_rules(window, cx))))
                .child(text_button("version-save", "Сохранить", ButtonKind::Primary)
                    .on_click(cx.listener(|this, _, window, cx| this.save_version_rules(window, cx)))));
        Some(div().id("version-rules-backdrop").absolute().inset_0().flex().items_center().justify_center()
            .bg(gpui::rgba(0x00000066))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(card)
            .into_any_element())
    }
}
