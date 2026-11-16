//! The commands of the Explorer context menu as plain data, so Papka can
//! list them in its own menu. Reading them from Windows is in `shell_menu`.

/// One line of the Explorer menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    /// What to pass to `shell_menu::invoke`; `None` for separators and
    /// for lines that only open a submenu.
    pub command: Option<u32>,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub children: Vec<Item>,
    /// A submenu not read yet ("Open with", "Send to"): its number for
    /// `shell_menu::expand`. Reading every submenu up front is what makes
    /// a context menu slow.
    pub submenu: Option<usize>,
}

impl Item {
    pub fn separator() -> Item { Item { separator: true, ..Item::default() } }
    pub fn command(label: &str, command: u32) -> Item {
        Item { label: label.into(), command: Some(command), enabled: true, ..Item::default() }
    }
    pub fn submenu(label: &str, children: Vec<Item>) -> Item {
        Item { label: label.into(), enabled: true, children, ..Item::default() }
    }
}

/// Explorer's own commands that Papka already has in its menu, where they
/// go through the journal, the Recycle Bin rules and Ctrl+Z.
const OWN_VERBS: [&str; 7] = ["open", "cut", "copy", "paste", "delete", "rename", "pastelink"];

/// Whether the command with this language-independent name stays out.
pub fn is_own_verb(verb: &str) -> bool {
    OWN_VERBS.iter().any(|own| verb.eq_ignore_ascii_case(own))
}

/// A menu text as shown: no "&" before the access key, no "\tCtrl+C" tail.
pub fn clean_label(raw: &str) -> String {
    let text = raw.split('\t').next().unwrap_or_default();
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            // "&&" is a literal ampersand.
            if chars.peek() == Some(&'&') { chars.next(); out.push('&'); }
        } else {
            out.push(c);
        }
    }
    // "Открыть (O)" style keys of some translations stay: they are text.
    out.trim().to_string()
}

/// Drops what cannot be shown or chosen: lines without text, submenus
/// that came out empty, and separators left at the edges or doubled.
pub fn tidy(items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    for mut item in items {
        if item.separator {
            if out.last().is_some_and(|last| !last.separator) { out.push(item); }
            continue;
        }
        item.children = tidy(item.children);
        if item.label.is_empty() || (item.command.is_none() && item.children.is_empty() && item.submenu.is_none()) { continue; }
        out.push(item);
    }
    if out.last().is_some_and(|last| last.separator) { out.pop(); }
    out
}

/// A label for a line its handler draws itself and gives no text: its
/// help text, shortened, if it has one.
pub fn fallback_label(help: &str) -> String {
    let help = clean_label(help);
    let help = help.trim_end_matches('.');
    if help.chars().count() <= 48 { return help.to_string(); }
    format!("{}…", help.chars().take(48).collect::<String>().trim_end())
}

/// How many commands the menu holds, submenus included.
pub fn count(items: &[Item]) -> usize {
    items.iter().map(|item| usize::from(item.command.is_some() && item.children.is_empty()) + count(&item.children)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_lose_access_keys_and_shortcut_tails() {
        assert_eq!(clean_label("&Открыть с помощью"), "Открыть с помощью");
        assert_eq!(clean_label("Сво&йства\tAlt+Enter"), "Свойства");
        assert_eq!(clean_label("Tom && Jerry"), "Tom & Jerry");
        assert_eq!(clean_label("  7-Zip  "), "7-Zip");
        assert_eq!(clean_label("&"), "");
    }

    #[test]
    fn a_line_without_text_takes_its_help_text() {
        assert_eq!(fallback_label("Открывает элемент в новом окне."), "Открывает элемент в новом окне");
        assert_eq!(fallback_label(""), "");
        assert!(fallback_label(&"я".repeat(100)).ends_with('…'));
    }

    #[test]
    fn commands_papka_has_itself_stay_out() {
        for verb in ["open", "Delete", "RENAME", "cut", "copy", "paste"] { assert!(is_own_verb(verb), "{verb}"); }
        for verb in ["properties", "openas", "sendto", "edit", "print", "link", ""] { assert!(!is_own_verb(verb), "{verb}"); }
    }

    #[test]
    fn empty_lines_and_stray_separators_are_dropped() {
        let menu = vec![
            Item::separator(),
            Item::command("Изменить", 3),
            Item::separator(),
            Item::separator(),
            Item::command("", 4),
            Item::submenu("Отправить", vec![Item::separator(), Item::command("Рабочий стол", 9), Item::separator()]),
            Item::submenu("Пустое", vec![Item::command("", 12)]),
            Item { submenu: Some(0), ..Item::submenu("Отправить позже", Vec::new()) },
            Item::separator(),
            Item::command("Свойства", 20),
            Item::separator(),
        ];
        let tidy = tidy(menu);
        let labels: Vec<&str> = tidy.iter().map(|item| if item.separator { "—" } else { item.label.as_str() }).collect();
        assert_eq!(labels, ["Изменить", "—", "Отправить", "Отправить позже", "—", "Свойства"], "an unread submenu stays");
        assert_eq!(tidy[2].children, vec![Item::command("Рабочий стол", 9)]);
        assert_eq!(count(&tidy), 3);
    }
}
