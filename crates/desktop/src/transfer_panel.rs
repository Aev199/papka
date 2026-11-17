//! The transfer panel: a card in the lower right corner with the running
//! copy or move and, afterwards, every error in full. It covers no more
//! than its own corner and never takes the keyboard.

use filemanager_core::browser;
use filemanager_core::format::format_size;
use gpui::{AnimationExt, AnyElement, ClipboardItem, Context, MouseButton, div, prelude::*, px};
use gpui_component::tooltip::Tooltip;

use crate::Explorer;
use crate::theme::*;
use crate::transfers;
use crate::view::{ButtonKind, icon_button, text_button};

impl Explorer {
    pub(crate) fn transfer_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.transfers.panel_visible() { return None; }
        let mut card = div().id("transfer-panel").occlude().absolute().right(px(12.)).bottom(px(44.)).w(px(380.))
            .p_3().flex().flex_col().gap_2().rounded(px(6.))
            .bg(rgb(RAISED)).border_1().border_color(rgb(BORDER_STRONG)).text_size(px(13.)).text_color(rgb(TEXT))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let dim = |text: String| div().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(text);

        if let Some(live) = &self.transfers.live {
            let paused = self.transfers.control.as_ref().is_some_and(|control| control.is_paused());
            let fraction = self.transfers.fraction.unwrap_or(0.0).clamp(0.0, 1.0) as f32;
            let target = live.target.display().to_string();
            let heading = if live.phase.is_empty() { format!("{}…", live.action) } else { live.phase.clone() };
            card = card.child(div().flex().items_center().gap_2()
                .child(div().flex_1().min_w_0().truncate().font_weight(gpui::FontWeight::SEMIBOLD).child(heading))
                .child(div().flex_none().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(format!("{}%", (fraction * 100.).floor() as u32)))
                .child(icon_button("transfer-fold", "fm/chevron-down.svg", "Свернуть: перенос продолжится, ход виден в строке внизу", false, true)
                    .on_click(cx.listener(|this, _, _, cx| { this.transfers.panel_hidden = true; cx.notify(); }))));
            card = card.child(div().id("transfer-target").min_w_0().truncate().text_size(px(12.)).text_color(rgb(TEXT_MUTED))
                .child(format!("в «{}»", browser::display_name(&live.target)))
                .tooltip(move |window, cx| Tooltip::new(target.clone()).build(window, cx)));
            if let Some(current) = &live.current {
                let full = current.display().to_string();
                card = card.child(div().id("transfer-current").min_w_0().truncate().child(browser::display_name(current))
                    .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)));
            }
            card = card.child(div().h(px(6.)).w_full().rounded_full().bg(rgb(BORDER)).overflow_hidden()
                .child(div().h_full().rounded_full().bg(rgb(if paused { TEXT_DIM } else { ACCENT })).w(gpui::relative(self.motion.bar().unwrap_or(fraction)))));
            // The item being copied counts: "1 из 3" while the first one runs.
            let mut facts = if live.total > 0 { vec![format!("{} из {}", (live.processed + 1).min(live.total), live.total)] } else { Vec::new() };
            if live.total_bytes > 0 { facts.push(format!("{} из {}", format_size(live.bytes), format_size(live.total_bytes))); }
            else if live.bytes > 0 { facts.push(format_size(live.bytes)); }
            if let Some(left) = &live.left { facts.push(format!("осталось {left}")); }
            let queued = self.transfers.queue.len();
            if queued > 0 { facts.push(format!("в очереди: {queued}")); }
            card = card.child(dim(facts.join(" · ")));
            card = card.child(div().flex().gap_2().justify_end()
                .child(text_button("transfer-pause", if paused { "Продолжить" } else { "Пауза" }, ButtonKind::Ghost)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_copy_pause(cx))))
                .child(text_button("transfer-cancel", "Отменить", ButtonKind::Ghost)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_copy(cx)))));
        }

        if let Some(report) = &self.transfers.report {
            let failed = !report.failures.is_empty();
            let id = report.id;
            if self.transfers.live.is_some() { card = card.child(div().h(px(1.)).bg(rgb(BORDER))); }
            let target = report.target.display().to_string();
            card = card.child(div().flex().items_center().gap_2()
                .child(div().flex_1().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(if failed { DANGER } else { TEXT })).child(report.title.clone()))
                .child(icon_button("transfer-close", "fm/x.svg", "Закрыть", false, true)
                    .on_click(cx.listener(move |this, _, _, cx| { this.transfers.dismiss(id); cx.notify(); }))));
            card = card.child(div().id("transfer-report-target").min_w_0().truncate().text_size(px(12.)).text_color(rgb(TEXT_MUTED))
                .child(format!("в «{}»", browser::display_name(&report.target)))
                .tooltip(move |window, cx| Tooltip::new(target.clone()).build(window, cx)));
            if failed {
                // Every error in full: the text wraps, the list scrolls.
                let mut list = div().id("transfer-errors").max_h(px(220.)).overflow_y_scroll().flex().flex_col().gap_2();
                for (index, failure) in report.failures.iter().take(200).enumerate() {
                    let full = failure.path.display().to_string();
                    list = list.child(div().flex().flex_col()
                        .child(div().id(("transfer-error", index)).min_w_0().truncate().child(browser::display_name(&failure.path))
                            .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)))
                        .child(div().text_size(px(12.)).text_color(rgb(TEXT_MUTED)).child(failure.error.clone())));
                }
                if report.failures.len() > 200 {
                    list = list.child(dim(format!("…и ещё {}", report.failures.len() - 200)));
                }
                card = card.child(list);
                let text = transfers::failures_text(&report.title, &report.failures);
                let retry = self.transfers.retry.as_ref().map(|(items, _, _)| items.len()).filter(|_| !self.transfers.copying);
                card = card.child(div().flex().gap_2().justify_end()
                    .child(text_button("transfer-copy-errors", "Скопировать текст", ButtonKind::Ghost)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            this.status = "Текст ошибок скопирован".into();
                            cx.notify();
                        })))
                    .when_some(retry, |row, count| row.child(text_button("transfer-retry", format!("Повторить сбойные ({count})"), ButtonKind::Primary)
                        .on_click(cx.listener(move |this, _, _, cx| { this.transfers.dismiss(id); this.retry_failed(cx); })))));
            }
        }
        // Rises into place when a copy starts; progress updates do not replay it.
        Some(card.with_animation("transfer-appear", crate::theme::motion(160),
            |card, delta| card.opacity(delta).bottom(px(44. - 8. * (1. - delta)))).into_any_element())
    }
}
