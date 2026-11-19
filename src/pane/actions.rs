use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::Pane;
use crate::i18n::tr;
use crate::log;
use crate::settings;
use crate::shell;
use crate::util::guard;

pub(super) fn install(pane: &Rc<Pane>) {
    let add = |name: &str, f: fn(&Rc<Pane>)| {
        let a = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(pane);
        let name = name.to_string();
        a.connect_activate(move |_, _| {
            if let Some(p) = weak.upgrade() {
                guard(&name, (), || f(&p));
            }
        });
        pane.actions.add_action(&a);
    };
    add("copy", |p| p.term.copy());
    add("paste", |p| p.term.paste());
    add("select-all", |p| p.term.select_all());
    add("insert", |p| p.insert_selection());
    // drops the scrollback; at the prompt, Ctrl+L also clears the screen and keeps the line being typed
    add("clear", |p| {
        p.term.feed_display("\x1b[3J");
        if p.at_prompt() {
            p.term.feed("\x0c");
        }
    });
    add("insert-var", |p| p.insert_variable());
    add("goto-tab-dir", |p| p.goto_tab_dir());
    add("show-in-nautilus", |p| p.show_in_nautilus());
    add("link-cd", |p| p.link_cd());
    add("link-open", |p| p.link_open());
    add("link-copy", |p| p.link_copy());
    add("link-insert", |p| p.link_insert());
    add("link-web-open", |p| p.link_web_open());
    add("link-web-copy", |p| p.link_web_copy());
    add("zoom-in", |p| p.zoom(Some(true)));
    add("zoom-out", |p| p.zoom(Some(false)));
    add("zoom-reset", |p| p.zoom(None));
    add("more-lines", |p| p.resize(gtk::Orientation::Vertical, 2));
    add("fewer-lines", |p| p.resize(gtk::Orientation::Vertical, -2));
    add("wider", |p| p.resize(gtk::Orientation::Horizontal, 4));
    add("narrower", |p| p.resize(gtk::Orientation::Horizontal, -4));
    add("prev-prompt", |p| p.jump_to_prompt(false));
    add("next-prompt", |p| p.jump_to_prompt(true));
    add("find", |p| p.open_search());
    add("find-next", |p| p.find(false));
    add("find-previous", |p| p.find(true));
    add("split-right", |p| p.split(gtk::Orientation::Horizontal));
    add("split-down", |p| p.split(gtk::Orientation::Vertical));
    add("split-auto", |p| {
        let (w, h) = (p.widget.width(), p.widget.height());
        p.split(if w >= h { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical });
    });
    add("close", |p| p.request_close());
    // shortcut: switches "the terminal follows Nautilus" on or off, keeping the other way
    add("follow", |p| {
        let m = p.sync_mode();
        p.set_sync(settings::Sync::from_parts(!m.terminal_follows(), m.nautilus_follows()));
    });

    for (name, dir) in [("focus-left", gtk::DirectionType::Left),
                        ("focus-right", gtk::DirectionType::Right),
                        ("focus-up", gtk::DirectionType::Up),
                        ("focus-down", gtk::DirectionType::Down)] {
        let a = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(pane);
        a.connect_activate(move |_, _| {
            if let Some(p) = weak.upgrade() {
                if let Some(panel) = p.panel.upgrade() {
                    guard("navigation", (), || panel.focus_neighbor(&p, dir));
                }
            }
        });
        pane.actions.add_action(&a);
    }

    let sync = gio::SimpleAction::new_stateful("sync", Some(glib::VariantTy::STRING),
                                               &settings::Sync::Both.id().to_variant());
    let weak = Rc::downgrade(pane);
    sync.connect_activate(move |_, v| {
        let Some(mode) = v.and_then(|v| v.str()).and_then(settings::Sync::from_id) else { return };
        if let Some(p) = weak.upgrade() {
            guard("folder sync", (), || p.set_sync(mode));
        }
    });
    pane.actions.add_action(&sync);

    let tw = pane.term.widget();
    tw.insert_action_group("shellman", Some(&pane.actions));
    // On the whole terminal area, scrollbar and prompt buttons included. Move must be accepted:
    // on Wayland the drag keeps the action chosen over the last target, and over its own files
    // view Nautilus chooses move, so the terminal would refuse it. Copy is always asked for.
    let drop = gtk::DropTarget::new(gdk::FileList::static_type(),
                                    gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK);
    drop.connect_motion(|_, _, _| gdk::DragAction::COPY);
    let weak = Rc::downgrade(pane);
    drop.connect_drop(move |_, value, _, _| {
        let Some(p) = weak.upgrade() else { return false };
        guard("drop", false, || match value.get::<gdk::FileList>() {
            Ok(files) => {
                p.insert_dropped(&files);
                true
            }
            Err(_) => false,
        })
    });
    pane.layers.add_controller(drop);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let acts = pane.actions.clone();
    keys.connect_key_pressed(move |ctrl, key, code, state| guard("terminal key", glib::Propagation::Proceed, || {
        match settings::action_for(ctrl, key, code, state) {
            Some(action) if action != "toggle" && acts.has_action(&action) => {
                acts.activate_action(&action, None);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }));
    tw.add_controller(keys);
}

impl Pane {
    // steps of GNOME Terminal
    fn zoom(self: &Rc<Self>, bigger: Option<bool>) {
        const SCALES: [f64; 11] = [0.5, 0.57, 0.69, 0.83, 1.0, 1.2, 1.44, 1.73, 2.07, 2.49, 3.0];
        let now = self.term.font_scale();
        let next = match bigger {
            None => Some(1.0),
            Some(true) => SCALES.iter().copied().find(|&s| s > now + 0.01),
            Some(false) => SCALES.iter().rev().copied().find(|&s| s < now - 0.01),
        };
        let Some(scale) = next else {
            self.term.widget().error_bell();
            return;
        };
        self.term.set_font_scale(scale);
        self.update_size_request();
        self.show_zoom(scale);
    }

    fn show_zoom(self: &Rc<Self>, scale: f64) {
        self.zoom_label.set_label(&format!("{} %", (scale * 100.0).round()));
        self.zoom_label.set_visible(true);
        if let Some(id) = self.zoom_timer.take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(1000), move || {
            if let Some(p) = weak.upgrade() {
                p.zoom_timer.take();
                guard("zoom label", (), || p.zoom_label.set_visible(false));
            }
        });
        *self.zoom_timer.borrow_mut() = Some(id);
    }

    /// lines: the height of the terminal area, columns: the width of this terminal in a side by side split
    fn resize(self: &Rc<Self>, orientation: gtk::Orientation, cells: i32) {
        let done = self.panel.upgrade().is_some_and(|panel| match orientation {
            gtk::Orientation::Vertical => panel.resize_lines(self, cells),
            _ => panel.resize_columns(self, cells),
        });
        if !done {
            self.term.widget().error_bell();
        }
    }

    fn split(self: &Rc<Self>, orientation: gtk::Orientation) {
        if let Some(panel) = self.panel.upgrade() {
            panel.split(self, orientation);
        }
    }

    fn insert_variable(&self) {
        match self.shell.sel_variable() {
            Some(v) => self.term.feed(&format!("{v} ")),
            None => self.insert_selection(),
        }
    }

    fn insert_selection(&self) {
        let Some(panel) = self.panel.upgrade() else { return };
        let sel = panel.selected();
        if sel.skipped > 0 {
            log!("insertion refused: {} selected item(s) have no local path", sel.skipped);
        }
        self.insert_paths(&sel.paths, sel.skipped);
    }

    /// files dropped from Nautilus or another application
    fn insert_dropped(&self, files: &gdk::FileList) {
        let all = files.files();
        let paths: Vec<std::path::PathBuf> = all.iter().filter_map(|f| f.path()).collect();
        let skipped = all.len() - paths.len();
        if skipped > 0 {
            log!("drop refused: {skipped} item(s) have no local path");
        }
        self.insert_paths(&paths, skipped);
        self.term.widget().grab_focus();
    }

    fn insert_paths(&self, paths: &[std::path::PathBuf], skipped: usize) {
        if paths.is_empty() || skipped > 0 {
            self.term.widget().error_bell();
            return;
        }
        match shell::words(self.shell, paths, shell::utf8_locale()) {
            Some(text) => self.term.feed(&text),
            None => {
                log!("insertion refused: a file name cannot be written safely for this shell");
                self.term.widget().error_bell();
            }
        }
    }

    fn request_close(self: &Rc<Self>) {
        let close = {
            let weak = Rc::downgrade(self);
            move || {
                if let Some(p) = weak.upgrade() {
                    if let Some(panel) = p.panel.upgrade() {
                        panel.close(&p);
                    }
                }
            }
        };
        if self.pid.get().is_none() || self.at_prompt() {
            close();
            return;
        }
        let dialog = adw::AlertDialog::new(Some(&tr("Close This Terminal?")),
                                           Some(&tr("A command is still running in it and will be stopped.")));
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("close", &tr("Close"))]);
        dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        dialog.connect_response(None, move |_, r| {
            if r == "close" {
                guard("close terminal", (), &close);
            }
        });
        dialog.present(Some(self.term.widget()));
    }
}
