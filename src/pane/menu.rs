use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::Pane;
use crate::i18n::{ntr, tr};
use crate::settings;
use crate::util::guard;

// actions disabled while the menu is open, when they do not apply
const MAY_DISABLE: &[&str] = &["copy", "insert", "insert-var", "sync", "goto-tab-dir", "link-cd"];

#[derive(Debug, PartialEq)]
enum Selection {
    Files,
    Folders,
    Mixed,
}

fn selection_kind(selected: &[PathBuf]) -> Selection {
    let dirs = selected.iter().filter(|p| p.is_dir()).count();
    match dirs {
        0 => Selection::Files,
        d if d == selected.len() => Selection::Folders,
        _ => Selection::Mixed,
    }
}

fn insert_label(selected: &[PathBuf]) -> String {
    let n = selected.len();
    let label = match selection_kind(selected) {
        Selection::Folders => ntr("Insert the Selected Folder", "Insert the {n} Selected Folders", n as u32),
        Selection::Mixed => ntr("Insert the Selected Item", "Insert the {n} Selected Items", n as u32),
        Selection::Files => ntr("Insert the Selected File", "Insert the {n} Selected Files", n.max(1) as u32),
    };
    label.replace("{n}", &n.to_string())
}

fn item(label: &str, action: &str) -> gio::MenuItem {
    let i = gio::MenuItem::new(Some(label), Some(&format!("shellman.{action}")));
    if let Some(a) = settings::accels(action).into_iter().next() {
        i.set_attribute_value("accel", Some(&a.to_variant()));
    }
    i
}

pub(super) fn connect(pane: &Rc<Pane>) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(pane);
    click.connect_pressed(move |g, _, x, y| {
        if let Some(p) = weak.upgrade() {
            g.set_state(gtk::EventSequenceState::Claimed);
            p.term.widget().grab_focus();
            guard("menu", (), || p.popup_menu(x, y));
        }
    });
    pane.term.widget().add_controller(click);
}

impl Pane {
    pub(super) fn set_action_enabled(&self, name: &str, on: bool) {
        if let Some(a) = self.actions.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            a.set_enabled(on);
        }
    }

    fn popup_menu(&self, x: f64, y: f64) {
        let selected = self.panel.upgrade().map(|p| p.selected().paths).unwrap_or_default();
        let sel_var = self.shell.sel_variable();
        self.set_action_enabled("copy", self.term.has_selection());
        self.set_action_enabled("insert", !selected.is_empty());
        self.set_action_enabled("insert-var", sel_var.is_some());
        self.set_action_enabled("sync", self.integrated.get());
        self.set_action_enabled("goto-tab-dir", self.integrated.get());
        if let Some(a) = self.actions.lookup_action("sync") {
            a.change_state(&self.sync_mode().id().to_variant());
        }

        let link = self.link_section(x, y, &item);
        let edit = gio::Menu::new();
        edit.append_item(&item(&tr("Copy"), "copy"));
        edit.append_item(&item(&tr("Paste"), "paste"));
        edit.append_item(&item(&tr("Select All"), "select-all"));
        edit.append_item(&item(&tr("Find…"), "find"));
        let files = gio::Menu::new();
        files.append_item(&item(&insert_label(&selected), "insert"));
        files.append_item(&item(&tr("Insert the {var} Variable").replace("{var}", sel_var.unwrap_or("SEL")), "insert-var"));
        let folder = gio::Menu::new();
        folder.append_item(&item(&tr("Terminal: Go to the Nautilus Folder"), "goto-tab-dir"));
        folder.append_item(&item(&tr("Nautilus: Go to the Terminal Folder"), "show-in-nautilus"));
        let sync = gio::Menu::new();
        for (mode, label) in [(settings::Sync::Both, tr("Both Ways")),
                              (settings::Sync::TerminalFollows, tr("The Terminal Follows Nautilus")),
                              (settings::Sync::NautilusFollows, tr("Nautilus Follows the Terminal")),
                              (settings::Sync::Off, tr("Off"))] {
            sync.append(Some(&label), Some(&format!("shellman.sync::{}", mode.id())));
        }
        folder.append_submenu(Some(&tr("Folder Sync")), &sync);
        let layout = gio::Menu::new();
        layout.append_item(&item(&tr("Split Right"), "split-right"));
        layout.append_item(&item(&tr("Split Down"), "split-down"));
        layout.append_item(&item(&tr("Close Terminal"), "close"));
        let clear = gio::Menu::new();
        clear.append_item(&item(&tr("Clear All"), "clear"));
        let app = gio::Menu::new();
        app.append(Some(&tr("Preferences…")), Some("shellman-app.preferences"));
        let menu = gio::Menu::new();
        if link.n_items() > 0 {
            menu.append_section(None, &link);
        }
        for section in [&edit, &files, &folder, &layout, &clear, &app] {
            menu.append_section(None, section);
        }

        let pop = gtk::PopoverMenu::from_model_full(&menu, gtk::PopoverMenuFlags::NESTED);
        pop.set_parent(self.term.widget());
        pop.set_has_arrow(false);
        pop.set_halign(gtk::Align::Start);
        pop.set_position(gtk::PositionType::Top);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        let actions = self.actions.clone();
        pop.connect_closed(move |p| guard("menu closed", (), || {
            for name in MAY_DISABLE {
                if let Some(a) = actions.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                    a.set_enabled(true);
                }
            }
            let p = p.clone();
            glib::idle_add_local_once(move || guard("menu unparent", (), || p.unparent()));
        }));
        pop.popup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_kind_tells_files_from_folders() {
        let dir = std::env::temp_dir();
        let file = dir.join(format!("shellman-label-{}", std::process::id()));
        std::fs::write(&file, b"").unwrap();
        assert_eq!(selection_kind(&[]), Selection::Files);
        assert_eq!(selection_kind(std::slice::from_ref(&file)), Selection::Files);
        assert_eq!(selection_kind(std::slice::from_ref(&dir)), Selection::Folders);
        assert_eq!(selection_kind(&[dir.clone(), dir.clone()]), Selection::Folders);
        assert_eq!(selection_kind(&[dir.clone(), file.clone()]), Selection::Mixed);
        std::fs::remove_file(&file).unwrap();
    }
}
