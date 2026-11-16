//! Explorer's own commands for the chosen files — "Open with", "Send to",
//! archivers, "Properties" and whatever is installed on the computer —
//! listed in Papka's context menu, below its own commands.

use std::path::{Path, PathBuf};

use filemanager_core::shell_items::Item;
use gpui::{Context, ElementId, SharedString, Stateful, Div, div, prelude::*, px};
use gpui_component::tooltip::Tooltip;

use crate::Explorer;
use crate::theme::*;
use crate::view::small_icon;

/// The Explorer part of an open context menu.
#[derive(Clone, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum ShellActions {
    /// Not for this menu (a tab, a place, a file inside an archive).
    #[default]
    None,
    /// Read on request: asking Explorer about a network file can take
    /// seconds, and the menu must not wait for that every time.
    Offer(Vec<PathBuf>),
    Loading(Vec<PathBuf>),
    Ready {
        token: u64,
        paths: Vec<PathBuf>,
        items: Vec<Item>,
        /// The submenu unfolded in place, by its index in `items`.
        open: Option<usize>,
    },
    Failed(String),
}

/// The files Explorer is asked about: the selection when the clicked item
/// belongs to it and it lies in one folder, otherwise the item alone.
pub(crate) fn menu_paths(clicked: &Path, selection: &[PathBuf]) -> Vec<PathBuf> {
    let together = selection.iter().any(|path| path == clicked)
        && selection.iter().all(|path| path.parent() == clicked.parent());
    if together { selection.to_vec() } else { vec![clicked.to_path_buf()] }
}

/// A menu to look at where there is no Explorer (Linux screenshots).
#[cfg(not(windows))]
fn sample() -> Vec<Item> {
    vec![
        Item::command("Изменить", 1),
        Item::submenu("Открыть с помощью", vec![Item::command("Блокнот", 2), Item::command("Microsoft Excel", 3), Item::separator(),
            Item::command("Выбрать другое приложение", 4)]),
        Item::submenu("7-Zip", vec![Item::command("Добавить к архиву…", 5), Item::command("Сжать и отправить по e-mail…", 6)]),
        Item::separator(),
        Item::submenu("Отправить", vec![Item::command("Рабочий стол (создать ярлык)", 7), Item::command("Сжатая ZIP-папка", 8)]),
        Item::separator(),
        Item::command("Создать ярлык", 9),
        Item { enabled: false, ..Item::command("Восстановить прежнюю версию", 10) },
        Item::command("Свойства", 11),
    ]
}

impl Explorer {
    /// What the menu of `clicked` starts with in its Explorer part.
    pub(crate) fn shell_actions_for(&self, clicked: &Path) -> ShellActions {
        if filemanager_core::archive::is_inside(clicked) { return ShellActions::None; }
        let paths = menu_paths(clicked, &self.selection());
        #[cfg(windows)]
        {
            if self.hwnd.is_none() { return ShellActions::None; }
            if filemanager_core::path_utils::is_network_path(clicked) { ShellActions::Offer(paths) } else { ShellActions::Loading(paths) }
        }
        #[cfg(not(windows))]
        {
            if std::env::var_os("PAPKA_SAMPLE_SHELL_MENU").is_some() {
                ShellActions::Ready { token: 0, paths, items: sample(), open: None }
            } else {
                ShellActions::None
            }
        }
    }

    /// Asks Explorer for the commands once the menu is on screen. The
    /// answer goes to the menu that asked, if it is still the open one.
    pub(crate) fn load_shell_actions(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(menu) = self.context_menu.as_mut().filter(|menu| menu.serial == serial) else { return };
        let paths = match std::mem::take(&mut menu.shell) {
            ShellActions::Offer(paths) | ShellActions::Loading(paths) => paths,
            other => { menu.shell = other; return; }
        };
        menu.shell = ShellActions::Loading(paths.clone());
        cx.notify();
        #[cfg(windows)]
        {
            let Some(hwnd) = self.hwnd else { return };
            cx.spawn(async move |this, cx| {
                // Papka's own commands are drawn first; Explorer's handlers
                // load in the window's thread and may take a moment.
                cx.background_executor().timer(std::time::Duration::from_millis(80)).await;
                let wanted = this.update(cx, |this, _| this.context_menu.as_ref().is_some_and(|menu| menu.serial == serial)).unwrap_or(false);
                if !wanted { return; }
                let result = filemanager_core::shell_menu::load(hwnd, &paths, false);
                let _ = this.update(cx, |this, cx| {
                    let Some(menu) = this.context_menu.as_mut().filter(|menu| menu.serial == serial) else {
                        filemanager_core::shell_menu::release();
                        return;
                    };
                    menu.shell = match result {
                        Ok((token, items)) => ShellActions::Ready { token, paths, items, open: None },
                        Err(error) => ShellActions::Failed(error.to_string()),
                    };
                    cx.notify();
                });
            }).detach();
        }
    }

    /// Runs a command of the Explorer part. Windows does the work, past
    /// Papka's journal and Ctrl+Z, as when chosen in Explorer itself.
    fn run_shell_action(&mut self, token: u64, command: u32, label: String, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.context_menu = None;
        cx.notify();
        #[cfg(windows)]
        {
            // A command may open a dialog with its own message loop: it
            // starts from the executor, outside this event handler.
            cx.spawn(async move |this, cx| {
                let result = filemanager_core::shell_menu::invoke(token, command);
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(()) => for path in &paths { this.refresh_parent_of(path, cx); },
                        Err(error) => this.status = format!("«{label}» не выполнено: {error}"),
                    }
                    cx.notify();
                });
            }).detach();
        }
        #[cfg(not(windows))]
        {
            let _ = (token, command, paths);
            self.status = format!("«{label}» есть только в Windows");
        }
    }

    fn shell_row(id: impl Into<ElementId>, label: &str, enabled: bool, indent: f32, checked: bool, submenu: Option<bool>) -> Stateful<Div> {
        let label: SharedString = label.to_string().into();
        let tip = label.clone();
        div().id(id).h(px(28.)).flex_none().pl(px(8. + indent)).pr_2().flex().items_center().gap_2().rounded(px(4.))
            .text_size(px(13.)).text_color(rgb(if enabled { TEXT } else { TEXT_DIM }))
            .when(enabled, |row| row.cursor_pointer().hover(|style| style.bg(rgb(HOVER))))
            .when(tip.chars().count() > 34, |row| row.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)))
            .child(div().w(px(16.)).flex_none().flex().justify_center()
                .when(checked, |slot| slot.child(small_icon("fm/check.svg", TEXT_MUTED))))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when_some(submenu, |row, open| row.child(small_icon(if open { "fm/chevron-down.svg" } else { "fm/chevron-right.svg" }, TEXT_DIM)))
    }

    /// The Explorer part of the menu, appended to Papka's own commands.
    pub(crate) fn shell_section(&self, mut list: Stateful<Div>, shell: &ShellActions, serial: u64, cx: &mut Context<Self>) -> Stateful<Div> {
        let caption = |text: &'static str| div().h(px(22.)).flex_none().px_2().flex().items_center()
            .text_size(px(11.)).text_color(rgb(TEXT_DIM)).child(text);
        let rule = || div().my_1().flex_none().h(px(1.)).bg(rgb(BORDER));
        match shell {
            ShellActions::None => {}
            ShellActions::Offer(_) => {
                list = list.child(rule()).child(Self::shell_row("shell-offer", "Действия Проводника…", true, 0., false, Some(false))
                    .tooltip(|window, cx| Tooltip::new("Сетевая папка: Проводник отвечает не сразу, поэтому его действия читаются по щелчку").build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.load_shell_actions(serial, cx); })));
            }
            ShellActions::Loading(_) => {
                list = list.child(rule()).child(caption("Проводник: действия загружаются…"));
            }
            ShellActions::Failed(error) => {
                let error = error.clone();
                list = list.child(rule()).child(div().id("shell-failed").h(px(22.)).flex_none().px_2().flex().items_center()
                    .text_size(px(11.)).text_color(rgb(TEXT_DIM)).child("Проводник: действия недоступны")
                    .tooltip(move |window, cx| Tooltip::new(error.clone()).build(window, cx)));
            }
            ShellActions::Ready { items, .. } if items.is_empty() => {}
            ShellActions::Ready { token, paths, items, open } => {
                list = list.child(rule()).child(caption("Проводник"));
                let token = *token;
                let command_row = |id: ElementId, item: &Item, indent: f32, cx: &mut Context<Self>| {
                    let (command, label, paths) = (item.command, item.label.clone(), paths.clone());
                    Self::shell_row(id, &item.label, item.enabled && command.is_some(), indent, item.checked, None)
                        .when_some(command.filter(|_| item.enabled), |row, command| row.on_click(cx.listener(move |this, _, _, cx| {
                            this.run_shell_action(token, command, label.clone(), paths.clone(), cx);
                        })))
                };
                for (index, item) in items.iter().enumerate() {
                    if item.separator { list = list.child(rule()); continue; }
                    if item.children.is_empty() {
                        list = list.child(command_row(("shell", index).into(), item, 0., cx));
                        continue;
                    }
                    let unfolded = *open == Some(index);
                    list = list.child(Self::shell_row(("shell", index), &item.label, item.enabled, 0., false, Some(unfolded))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(crate::ContextMenu { shell: ShellActions::Ready { open, .. }, .. }) = &mut this.context_menu {
                                *open = if *open == Some(index) { None } else { Some(index) };
                            }
                            cx.notify();
                        })));
                    if !unfolded { continue; }
                    // The submenu unfolds in place; its own submenus, one level more, lie open under their name.
                    for (child_index, child) in item.children.iter().enumerate() {
                        let id = index * 1000 + child_index;
                        if child.separator { list = list.child(rule()); continue; }
                        if child.children.is_empty() {
                            list = list.child(command_row(("shell-sub", id).into(), child, 18., cx));
                            continue;
                        }
                        list = list.child(Self::shell_row(("shell-sub", id), &child.label, false, 18., false, None));
                        for (deep_index, deep) in child.children.iter().enumerate().filter(|(_, deep)| !deep.separator && deep.children.is_empty()) {
                            list = list.child(command_row(("shell-deep", id * 1000 + deep_index).into(), deep, 36., cx));
                        }
                    }
                }
            }
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_is_asked_about_the_selection_only_when_the_click_is_in_it() {
        let (a, b, elsewhere) = (PathBuf::from("d/a.txt"), PathBuf::from("d/b.txt"), PathBuf::from("e/c.txt"));
        assert_eq!(menu_paths(&a, &[a.clone(), b.clone()]), vec![a.clone(), b.clone()]);
        assert_eq!(menu_paths(&a, &[b.clone()]), vec![a.clone()], "clicked outside the selection");
        assert_eq!(menu_paths(&a, &[]), vec![a.clone()]);
        // Search results from several folders: Explorer builds a menu for one folder.
        assert_eq!(menu_paths(&a, &[a.clone(), elsewhere]), vec![a]);
    }
}
