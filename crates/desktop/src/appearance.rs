//! «Оформление»: the theme by Windows' own setting, row density, folder
//! colour and tile size. Each is a small global the views read when drawn.

use gpui::{Context, Window, WindowAppearance};
use gpui_component::theme::{Theme, ThemeMode};

use crate::{theme, Explorer};

const SYSTEM_THEME_KEY: &str = "theme_system";
const DENSITY_KEY: &str = "row_density";
const TILE_SIZE_KEY: &str = "tile_size";
/// «Плотность строк», matching `layout::DENSITIES`.
pub(crate) const DENSITIES: [&str; 3] = ["Плотно", "Обычно", "Просторно"];

static DENSITY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
pub(crate) fn density() -> usize { DENSITY.load(std::sync::atomic::Ordering::Relaxed) }

#[derive(Clone, Copy)]
pub(crate) struct Look {
    pub system_theme: bool,
    pub density: usize,
    pub folder_color: usize,
    pub tile_size: usize,
}

fn is_light(appearance: WindowAppearance) -> bool {
    matches!(appearance, WindowAppearance::Light | WindowAppearance::VibrantLight)
}

impl Explorer {
    /// At start: the saved look, and the theme following Windows from now on.
    pub(crate) fn load_look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = |key| self.workspaces.as_ref().and_then(|s| s.value(key).ok().flatten());
        let index = |key, len: usize, default| value(key).and_then(|v| v.parse::<usize>().ok()).filter(|v| *v < len).unwrap_or(default);
        let look = Look {
            system_theme: value(SYSTEM_THEME_KEY).is_some_and(|flag| flag == "1"),
            density: index(DENSITY_KEY, DENSITIES.len(), 1),
            folder_color: index(theme::FOLDER_COLOR_KEY, theme::FOLDER_COLORS.len(), 0),
            tile_size: index(TILE_SIZE_KEY, crate::thumbs::SIZES.len(), 1),
        };
        self.light_preference = theme::is_light();
        self.system_light = is_light(window.appearance());
        self.set_look(look);
        if self.system_theme { self.apply_theme(cx); }
        self._subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            this.system_light = is_light(window.appearance());
            if this.system_theme { this.apply_theme(cx); }
        }));
    }

    pub(crate) fn set_look(&mut self, look: Look) {
        self.system_theme = look.system_theme;
        DENSITY.store(look.density, std::sync::atomic::Ordering::Relaxed);
        crate::layout::set_density(look.density);
        theme::set_folder_color(look.folder_color);
        crate::thumbs::set_size(look.tile_size);
        if let Some(store) = &self.workspaces {
            let _ = store.set_value(SYSTEM_THEME_KEY, if look.system_theme { "1" } else { "0" });
            let _ = store.set_value(DENSITY_KEY, &look.density.to_string());
            let _ = store.set_value(theme::FOLDER_COLOR_KEY, &look.folder_color.to_string());
            let _ = store.set_value(TILE_SIZE_KEY, &look.tile_size.to_string());
        }
    }

    /// Light or dark as chosen, or as Windows is now.
    pub(crate) fn apply_theme(&mut self, cx: &mut Context<Self>) {
        let light = if self.system_theme { self.system_light } else { self.light_preference };
        theme::set_light(light);
        Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
        crate::view::apply_component_theme(cx);
        cx.notify();
    }
}
