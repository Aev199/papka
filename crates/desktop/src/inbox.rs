//! Publishing Papka's projects for other programs (Notch's «В проект…»)
//! and reading what they send into a project's inbox. See core `inbox`
//! and `docs/INBOX.md`.

use std::collections::HashMap;
use std::path::PathBuf;

use filemanager_core::inbox::{self, RegistryProject};
use gpui::{AppContext, Context};

use crate::Explorer;

/// How the object's folder of a project was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootSource {
    /// Set by the user on the card.
    Chosen,
    /// Where the object's passport lies.
    Passport,
    /// The folder the project's tabs share.
    Tabs,
    /// Not known: the inbox lives in Papka's profile.
    None,
}

impl RootSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            RootSource::Chosen => "задана вручную",
            RootSource::Passport => "по паспорту объекта",
            RootSource::Tabs => "общая папка вкладок",
            RootSource::None => "не определена: входящие хранятся в профиле Papka",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Published {
    pub(crate) project: RegistryProject,
    pub(crate) source: RootSource,
}

/// What was last written to `projects.json`, by project name.
#[derive(Default)]
pub(crate) struct Registry {
    pub(crate) projects: HashMap<String, Published>,
    /// Something changed: write the file again on the next autosave tick.
    pub(crate) due: bool,
    pub(crate) busy: bool,
}

impl Explorer {
    /// Writes `projects.json` with every project's number, name, object
    /// folder and inbox. The disk (a passport on a network share) is read
    /// off the window's thread.
    pub(crate) fn publish_projects(&mut self, cx: &mut Context<Self>) {
        if self.registry.busy { return; }
        let Some(store) = self.workspaces.clone() else { return };
        self.registry.due = false;
        self.registry.busy = true;
        // Projects open in a pane: their tabs as they are now, not as saved.
        let open: HashMap<String, Vec<PathBuf>> = self.browser.groups.iter()
            .filter_map(|group| Some((group.project.as_ref()?.name.clone(), group.tabs.iter().map(|tab| tab.pane.path.clone()).collect())))
            .collect();
        let names = self.workspace_names.clone();
        let task = cx.background_spawn(async move {
            let mut published = Vec::new();
            for name in names {
                let Ok((id, chosen)) = store.project_identity(&name) else { continue };
                let folders = open.get(&name).cloned().unwrap_or_else(|| store.load_project(&name).ok().flatten()
                    .map(|workspace| workspace.groups.iter().flat_map(|group| group.tabs.iter().map(|tab| tab.pane.path.clone())).collect())
                    .unwrap_or_default());
                let (root, source) = match chosen {
                    Some(root) => (Some(root), RootSource::Chosen),
                    None => {
                        let passport = filemanager_core::passport::find(&folders);
                        match inbox::derive_root(passport.as_deref(), &folders) {
                            Some(root) if passport.as_deref().and_then(|file| file.parent()) == Some(root.as_path()) => (Some(root), RootSource::Passport),
                            Some(root) => (Some(root), RootSource::Tabs),
                            None => (None, RootSource::None),
                        }
                    }
                };
                let inbox = inbox::inbox_for(root.as_deref(), &id);
                published.push(Published { project: RegistryProject { id, name, root, inbox }, source });
            }
            let written = inbox::write_registry(&inbox::registry_path(), published.iter().map(|item| item.project.clone()).collect());
            (published, written)
        });
        cx.spawn(async move |this, cx| {
            let (published, written) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.registry.busy = false;
                this.registry.projects = published.into_iter().map(|item| (item.project.name.clone(), item)).collect();
                if let Err(error) = written { this.status = format!("Список проектов для других программ не записан: {error}"); }
                this.project_card_registry_changed(cx);
                cx.notify();
            });
        }).detach();
    }

    /// Sets or clears the object's folder of project `name`.
    pub(crate) fn set_project_root(&mut self, name: &str, root: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(store) = &self.workspaces else { return };
        match store.set_project_root(name, root.as_deref()) {
            Ok(()) => {
                self.status = match &root {
                    Some(root) => format!("Папка объекта: {}", root.display()),
                    None => "Папка объекта снова определяется сама".into(),
                };
                self.publish_projects(cx);
            }
            Err(error) => self.status = format!("Папка объекта не сохранена: {error}"),
        }
        cx.notify();
    }
}
