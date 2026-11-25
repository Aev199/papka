//! The transfer panel: a card in the lower right corner with the running
//! copy or move and, afterwards, every error in full. It covers no more
//! than its own corner and never takes the keyboard.

use filemanager_core::browser;
use filemanager_core::format::format_size;
use gpui::{AnyElement, ClipboardItem, Context, MouseButton, div, prelude::*, px};
use gpui_component::tooltip::Tooltip;

use crate::Explorer;
use crate::theme::*;
use crate::ui_tokens as ui;
use crate::transfers;
use crate::view::{ButtonKind, icon_button, text_button};

/// Compact immutable panel state. Construct when the panel presentation
/// changes and retain it in popup presence; its `Clone` is cheap for closing
/// frames, including reports with many failures.
#[derive(Clone, Default)]
pub(crate) struct TransferPanelPresentation {
    live: Vec<LivePresentation>,
    report: Option<ReportPresentation>,
}

#[derive(Clone)]
struct LivePresentation {
    id:u64,
    action: &'static str,
    target: std::path::PathBuf,
    phase: String,
    processed: usize,
    total: usize,
    bytes: u64,
    total_bytes: u64,
    left: Option<String>,
    current: Option<std::path::PathBuf>,
    paused: bool,
    fraction: f32,
    queued: usize,
}

#[derive(Clone)]
struct ReportPresentation {
    id: u64,
    title: String,
    targets: std::sync::Arc<[std::path::PathBuf]>,
    failures: std::sync::Arc<[crate::transfers::Failure]>,
    retry_count: usize,
}

fn report_presentation(
    report: &crate::transfers::Report,
    retry_count: usize,
    previous: Option<&ReportPresentation>,
) -> ReportPresentation {
    let reusable = previous.filter(|cached| cached.id == report.id);
    ReportPresentation {
        id: report.id,
        title: report.title.clone(),
        targets: reusable.map_or_else(|| std::sync::Arc::from(report.targets.clone()), |cached| cached.targets.clone()),
        failures: reusable.map_or_else(|| std::sync::Arc::from(report.failures.clone()), |cached| cached.failures.clone()),
        retry_count,
    }
}

impl Explorer {
    /// Capture presentation once per transfer state change (not once per
    /// render frame). The parent retains the last snapshot for fade-out.
    pub(crate) fn transfer_panel_presentation(&self, previous: Option<&TransferPanelPresentation>) -> Option<TransferPanelPresentation> {
        if !self.transfers.panel_visible() { return None; }
        let mut live:Vec<_>=self.transfers.running.iter().map(|job|LivePresentation{
            id:job.id,action:job.live.action,target:job.live.target.clone(),phase:job.live.phase.clone(),
            processed:job.live.processed,total:job.live.total,bytes:job.live.bytes,total_bytes:job.live.total_bytes,
            left:job.live.left.clone(),current:job.live.current.clone(),paused:job.control.is_paused(),
            fraction:job.fraction.clamp(0.,1.) as f32,queued:0,
        }).collect();
        if live.is_empty() {
            if let Some(job)=&self.transfers.live {live.push(LivePresentation{
                id:0,action:job.action,target:job.target.clone(),phase:job.phase.clone(),
                processed:job.processed,total:job.total,bytes:job.bytes,total_bytes:job.total_bytes,
                left:job.left.clone(),current:job.current.clone(),paused:self.transfers.all_paused(),
                fraction:self.transfers.fraction.unwrap_or(0.).clamp(0.,1.) as f32,queued:0,
            });}
        }
        if let Some(first)=live.first_mut(){first.queued=self.transfers.queue.len();}
        let previous_report = previous.and_then(|presentation| presentation.report.as_ref());
        let report = self.transfers.report.as_ref().map(|report| {
            report_presentation(report, self.transfers.retry_count(), previous_report)
        });
        Some(TransferPanelPresentation { live, report })
    }

    pub(crate) fn transfer_panel(&self, presentation: &TransferPanelPresentation, popup: crate::popup_motion::Phase, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div().id("transfer-panel").occlude().absolute().right(px(12.)).bottom(px(44.)).w(px(380.))
            .p_3().flex().flex_col().gap_2().max_h(gpui::relative(0.8)).overflow_y_scroll().rounded(px(ui::PANEL_RADIUS))
            .bg(rgb(RAISED)).border_1().border_color(rgb(BORDER_STRONG)).text_size(px(ui::BODY_TEXT)).text_color(rgb(TEXT))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let dim = |text: String| div().text_size(px(ui::SECONDARY_TEXT)).text_color(rgb(TEXT_MUTED)).child(text);

        if !presentation.live.is_empty(){card=card.child(div().flex().items_center().gap_2()
            .child(div().flex_1().font_weight(gpui::FontWeight::SEMIBOLD).child(format!("Передачи: {}",presentation.live.len())))
            .child(icon_button("transfer-fold","fm/chevron-down.svg","Свернуть: передачи продолжатся",false,true)
                .on_click(cx.listener(|this,_,_,cx|{this.transfers.panel_hidden=true;cx.notify();}))));}
        let mut list=div().id("active-transfers").max_h(px(360.)).overflow_y_scroll().flex().flex_col().gap_3();
        for live in &presentation.live {
            let id=live.id;
            let mut item=div().id(gpui::SharedString::from(format!("transfer-job-{id}"))).flex().flex_col().gap_2().flex_none();
            let paused = live.paused;
            let fraction = live.fraction;
            let target = live.target.display().to_string();
            let heading = if live.phase.is_empty() { format!("{}…", live.action) } else { live.phase.clone() };
            item = item.child(div().flex().items_center().gap_2()
                .child(div().flex_1().min_w_0().truncate().font_weight(gpui::FontWeight::SEMIBOLD).child(heading))
                .child(div().flex_none().text_size(px(ui::SECONDARY_TEXT)).text_color(rgb(TEXT_MUTED)).child(format!("{}%", (fraction * 100.).floor() as u32))));
            item = item.child(div().id(gpui::SharedString::from(format!("transfer-target-{id}"))).min_w_0().truncate().text_size(px(ui::SECONDARY_TEXT)).text_color(rgb(TEXT_MUTED))
                .child(format!("в «{}»", browser::display_name(&live.target)))
                .tooltip(move |window, cx| Tooltip::new(target.clone()).build(window, cx)));
            if let Some(current) = &live.current {
                let full = current.display().to_string();
                item = item.child(div().id(gpui::SharedString::from(format!("transfer-current-{id}"))).min_w_0().truncate().child(browser::display_name(current))
                    .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)));
            }
            item = item.child(div().h(px(6.)).w_full().rounded_full().bg(rgb(BORDER)).overflow_hidden()
                .child(div().h_full().rounded_full().bg(rgb(if paused { TEXT_DIM } else { ACCENT })).w(gpui::relative(fraction))));
            // The item being copied counts: "1 из 3" while the first one runs.
            let mut facts = if live.total > 0 { vec![format!("{} из {}", (live.processed + 1).min(live.total), live.total)] } else { Vec::new() };
            if live.total_bytes > 0 { facts.push(format!("{} из {}", format_size(live.bytes), format_size(live.total_bytes))); }
            else if live.bytes > 0 { facts.push(format_size(live.bytes)); }
            if let Some(left) = &live.left { facts.push(format!("осталось {left}")); }
            let queued = live.queued;
            if queued > 0 { facts.push(format!("в очереди: {queued}")); }
            item = item.child(dim(facts.join(" · ")));
            item = item.child(div().flex().gap_2().justify_end()
                .child(text_button(gpui::SharedString::from(format!("transfer-pause-{id}")), if paused { "Продолжить" } else { "Пауза" }, ButtonKind::Ghost)
                    .on_click(cx.listener(move|this, _, _, cx| this.toggle_transfer_pause(id,cx))))
                .child(text_button(gpui::SharedString::from(format!("transfer-cancel-{id}")), "Отменить", ButtonKind::Ghost)
                    .on_click(cx.listener(move|this, _, _, cx| this.cancel_transfer(id,cx)))));
            list=list.child(item);
        }
        card=card.child(list);

        if let Some(report) = &presentation.report {
            let failed = !report.failures.is_empty();
            let id = report.id;
            if !presentation.live.is_empty() { card = card.child(div().h(px(1.)).bg(rgb(BORDER))); }
            let target = report.targets.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("\n");
            let target_label = if report.targets.len() == 1 { format!("в «{}»", browser::display_name(&report.targets[0])) }
                else { format!("Папок назначения: {}", report.targets.len()) };
            card = card.child(div().flex().items_center().gap_2()
                .child(div().flex_1().min_w_0().font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(if failed { DANGER } else { TEXT })).child(report.title.clone()))
                .child(icon_button("transfer-close", "fm/x.svg", "Закрыть", false, true)
                    .on_click(cx.listener(move |this, _, _, cx| { this.transfers.dismiss(id); cx.notify(); }))));
            card = card.child(div().id("transfer-report-target").min_w_0().truncate().text_size(px(ui::SECONDARY_TEXT)).text_color(rgb(TEXT_MUTED))
                .child(target_label)
                .tooltip(move |window, cx| Tooltip::new(target.clone()).build(window, cx)));
            if failed {
                // Every error in full: the text wraps, the list scrolls.
                let mut list = div().id("transfer-errors").max_h(px(220.)).overflow_y_scroll().flex().flex_col().gap_2();
                for (index, failure) in report.failures.iter().take(200).enumerate() {
                    let full = failure.path.display().to_string();
                    list = list.child(div().flex().flex_col()
                        .child(div().id(("transfer-error", index)).min_w_0().truncate().child(browser::display_name(&failure.path))
                            .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)))
                        .child(div().text_size(px(ui::SECONDARY_TEXT)).text_color(rgb(TEXT_MUTED)).child(failure.error.clone())));
                }
                if report.failures.len() > 200 {
                    list = list.child(dim(format!("…и ещё {}", report.failures.len() - 200)));
                }
                card = card.child(list);
                let text = transfers::failures_text(&report.title, &report.failures);
                let retry = Some(report.retry_count).filter(|count| *count > 0 && !self.transfers.busy && self.transfers.pending.is_none());
                card = card.child(div().flex().gap_2().justify_end()
                    .child(text_button("transfer-copy-errors", "Скопировать текст", ButtonKind::Ghost)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            this.status = "Текст ошибок скопирован".into();
                            cx.notify();
                        })))
                    .when_some(retry, |row, count| row.child(text_button("transfer-retry", format!("Повторить сбойные ({count})"), ButtonKind::Primary)
                        .on_click(cx.listener(move |this, _, _, cx| { this.retry_failed(cx); })))));
            }
        }
        // Only the retiring card blocks input; the file panes remain usable.
        let layer = div().id("transfer-panel-layer").absolute().inset_0()
            .opacity(popup.alpha)
            .child(card.bottom(px(44. + 8. * (1. - popup.alpha)))
                .when(popup.closing, |card| card.child(crate::popup_motion::shield())));
        layer.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u64, name: &str) -> crate::transfers::Report {
        crate::transfers::Report {
            id,
            title: format!("{name}"),
            targets: vec![name.into()],
            failures: vec![crate::transfers::Failure { path: name.into(), error: "failed".into() }],
        }
    }

    #[test]
    fn report_snapshot_reuses_heavy_slices_only_for_same_id() {
        let first_report = report(1, "first");
        let first = report_presentation(&first_report, 0, None);
        let updated = report_presentation(&first_report, 2, Some(&first));
        assert!(std::sync::Arc::ptr_eq(&first.failures, &updated.failures));
        assert!(std::sync::Arc::ptr_eq(&first.targets, &updated.targets));
        assert_eq!(updated.retry_count, 2);

        let next_report = report(2, "second");
        let next = report_presentation(&next_report, 0, Some(&updated));
        assert!(!std::sync::Arc::ptr_eq(&updated.failures, &next.failures));
        assert_eq!(next.title, "second");
    }
}
