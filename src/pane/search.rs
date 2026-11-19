use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use super::Pane;
use crate::i18n::tr;
use crate::settings;
use crate::util::guard;

pub(super) struct Search {
    pub bar: gtk::SearchBar,
    pub entry: gtk::SearchEntry,
    prev: gtk::Button,
    next: gtk::Button,
}

impl Search {
    pub fn new() -> Search {
        let entry = gtk::SearchEntry::new();
        entry.set_hexpand(true);
        let prev = gtk::Button::from_icon_name("go-up-symbolic");
        prev.set_tooltip_text(Some(&tr("Find Previous")));
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some(&tr("Find Next")));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&entry);
        row.append(&prev);
        row.append(&next);
        let bar = gtk::SearchBar::builder().child(&row).show_close_button(true).build();
        bar.connect_entry(&entry);
        Search { bar, entry, prev, next }
    }

    pub fn connect(pane: &Rc<Pane>) {
        let s = &pane.search;
        let weak = Rc::downgrade(pane);
        s.entry.connect_search_changed(move |e| {
            if let Some(p) = weak.upgrade() {
                guard("search", (), || p.search_changed(&e.text()));
            }
        });
        let find = |backward: bool| {
            let weak = Rc::downgrade(pane);
            move || {
                if let Some(p) = weak.upgrade() {
                    guard("search", (), || p.find(backward));
                }
            }
        };
        let f = find(true);
        s.entry.connect_activate(move |_| f());
        let f = find(true);
        s.prev.connect_clicked(move |_| f());
        let f = find(false);
        s.next.connect_clicked(move |_| f());
        let weak = Rc::downgrade(pane);
        s.bar.connect_search_mode_enabled_notify(move |b| {
            if !b.is_search_mode() {
                if let Some(p) = weak.upgrade() {
                    guard("search closed", (), || p.focus());
                }
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(pane);
        keys.connect_key_pressed(move |ctrl, key, code, state| guard("search key", glib::Propagation::Proceed, || {
            let Some(p) = weak.upgrade() else { return glib::Propagation::Proceed };
            let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter);
            if enter && state.contains(gdk::ModifierType::SHIFT_MASK) {
                p.find(false);
                return glib::Propagation::Stop;
            }
            match settings::action_for(ctrl, key, code, state).as_deref() {
                Some(a @ ("find" | "find-next" | "find-previous")) => {
                    p.actions.activate_action(a, None);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        }));
        s.entry.add_controller(keys);
    }
}

impl Pane {
    pub(super) fn open_search(&self) {
        self.search.bar.set_search_mode(true);
        self.search.entry.grab_focus();
        self.search.entry.select_region(0, -1);
    }

    fn search_changed(&self, text: &str) {
        self.term.set_search(text);
        self.term.unselect();
        let found = text.is_empty() || self.term.find(true);
        if found {
            self.search.entry.remove_css_class("error");
        } else {
            self.search.entry.add_css_class("error");
        }
    }

    pub(super) fn find(&self, backward: bool) {
        if self.search.entry.text().is_empty() {
            self.open_search();
            return;
        }
        if !self.term.find(backward) {
            self.term.widget().error_bell();
        }
    }
}
