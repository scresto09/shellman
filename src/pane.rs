use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::gdk;
use gtk::gio;
use gtk::glib;

use crate::backdrop::{self, Backdrop};
use crate::config;
use crate::panel::Panel;
use crate::shell::{self, Shell};
use crate::settings;
use crate::util::guard;
use crate::vte::{user_shell, Terminal};

mod actions;
mod activity;
mod folders;
mod links;
mod menu;
mod prompts;
mod search;

pub struct Pane {
    panel: Weak<Panel>,
    pub term: Terminal,
    pub widget: gtk::Widget,
    backdrop: Backdrop,
    layers: gtk::Overlay,
    actions: gio::SimpleActionGroup,
    search: search::Search,
    prompts: prompts::Prompts,
    shell: Shell,
    pid: Cell<Option<i32>>,
    spawning: Cell<bool>,
    integrated: Cell<bool>,
    shell_dir: RefCell<Option<PathBuf>>,
    /// folder to cd into once the shell is at its prompt
    pending: RefCell<Option<PathBuf>>,
    sync_override: Cell<Option<settings::Sync>>,
    link: RefCell<Option<PathBuf>>,
    web_link: RefCell<Option<String>>,
    activity_id: Cell<u64>,
    busy_since: Cell<Option<std::time::Instant>>,
    bar_cursor: Cell<bool>,
    /// zoom level shown for a moment after a change
    zoom_label: gtk::Label,
    zoom_timer: RefCell<Option<glib::SourceId>>,
    cd_file: PathBuf,
    state_file: PathBuf,
}

pub fn system_monospace_font() -> Option<String> {
    let schema = "org.gnome.desktop.interface";
    gio::SettingsSchemaSource::default()?.lookup(schema, true)?;
    let s = gio::Settings::new(schema).string("monospace-font-name");
    (!s.is_empty()).then(|| s.to_string())
}

// the color the "view" style class would paint
#[allow(deprecated)]
fn view_background(w: &gtk::Widget) -> Option<gdk::RGBA> {
    let ctx = w.style_context();
    ctx.lookup_color("view_bg_color").or_else(|| ctx.lookup_color("theme_base_color"))
}

impl Pane {
    pub fn new(panel: &Rc<Panel>) -> Rc<Pane> {
        let term = Terminal::new();
        let tw = term.widget().clone();
        tw.set_hexpand(true);
        tw.set_vexpand(true);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&tw)
            .build();
        let backdrop = Backdrop::default();
        backdrop.set_can_target(false);
        let layers = gtk::Overlay::new();
        // an overlay only takes the expand flags of its main child, here the image
        layers.set_hexpand(true);
        layers.set_vexpand(true);
        layers.set_child(Some(&backdrop));
        layers.add_overlay(&scrolled);
        layers.set_measure_overlay(&scrolled, true);
        layers.add_css_class("view");
        let search = search::Search::new();
        let prompts = prompts::Prompts::new(&layers);
        let zoom_label = gtk::Label::builder()
            .css_classes(["osd", "title-2"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .visible(false)
            .build();
        zoom_label.set_margin_top(12);
        layers.add_overlay(&zoom_label);
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.append(&search.bar);
        vbox.append(&layers);

        let pane = Rc::new(Pane {
            panel: Rc::downgrade(panel),
            term: term.clone(),
            widget: vbox.upcast(),
            backdrop,
            layers,
            actions: gio::SimpleActionGroup::new(),
            search,
            prompts,
            shell: Shell::of(&user_shell()),
            pid: Cell::new(None),
            spawning: Cell::new(false),
            integrated: Cell::new(false),
            shell_dir: RefCell::new(None),
            pending: RefCell::new(None),
            sync_override: Cell::new(None),
            link: RefCell::new(None),
            web_link: RefCell::new(None),
            activity_id: Cell::new(0),
            busy_since: Cell::new(None),
            bar_cursor: Cell::new(false),
            zoom_label,
            zoom_timer: RefCell::new(None),
            cd_file: shell::runtime_file("cd"),
            state_file: shell::runtime_file("state"),
        });
        pane.apply_settings();
        pane.connect_terminal();
        actions::install(&pane);
        search::Search::connect(&pane);
        prompts::Prompts::connect(&pane);
        menu::connect(&pane);
        links::connect(&pane);
        activity::register(&pane);
        pane
    }

    fn connect_terminal(self: &Rc<Self>) {
        let term = &self.term;
        let weak = Rc::downgrade(self);
        term.widget().connect_map(move |_| {
            if let Some(p) = weak.upgrade() {
                guard("terminal colors", (), || p.apply_colors(&crate::colors::current().0));
            }
        });
        let weak = Rc::downgrade(self);
        term.connect_child_exited(move || {
            if let Some(p) = weak.upgrade() {
                p.pid.set(None);
                if let Some(panel) = p.panel.upgrade() {
                    panel.close(&p);
                }
            }
        });
        // the prompt shown after a command triggers the pending cd
        let weak = Rc::downgrade(self);
        term.connect_contents_changed(move || {
            if let Some(p) = weak.upgrade() {
                if p.pending.borrow().is_some() {
                    p.flush();
                }
                p.update_cursor_shape();
                p.note_prompt();
            }
        });
        let weak = Rc::downgrade(self);
        term.connect_cwd_changed(move || {
            if let Some(p) = weak.upgrade() {
                p.cwd_changed();
            }
        });

        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            if let Some(p) = weak.upgrade() {
                if let Some(panel) = p.panel.upgrade() {
                    guard("focus", (), || panel.pane_focused(&p));
                }
            }
        });
        let weak = Rc::downgrade(self);
        focus.connect_leave(move |_| {
            if let Some(panel) = weak.upgrade().and_then(|p| p.panel.upgrade()) {
                guard("focus", (), || panel.mute_app_shortcuts(false));
            }
        });
        self.widget.add_controller(focus);
    }

    pub fn min_height(&self) -> i32 {
        let ch = self.term.char_height();
        let ch = if ch > 0 { ch } else { 18 };
        settings::with(|s| s.rows) * ch + 4
    }

    pub fn apply_settings(&self) {
        let s = settings::get();
        self.term.set_scrollback(s.scrollback);
        if let Some(f) = s.font.clone().or_else(system_monospace_font) {
            self.term.set_font(&f);
        }
        self.update_size_request();
        self.apply_colors(&crate::colors::current().0);
        self.update_cursor_shape();
        self.update_prompt_marks();
        self.prompts.update_tooltips();
    }

    pub fn update_size_request(&self) {
        self.widget.set_size_request(config::MIN_PANE_WIDTH, self.min_height());
    }

    pub fn redraw_background(&self) {
        self.backdrop.queue_draw();
    }

    pub fn has_focus(&self) -> bool {
        self.term.widget().has_focus()
    }

    pub fn focus(&self) {
        self.term.widget().grab_focus();
    }

    pub fn apply_colors(&self, v: &crate::colors::Variant) {
        let fg = v.fg.unwrap_or_else(|| {
            let c = self.term.widget().color();
            gtk::gdk::RGBA::new(c.red(), c.green(), c.blue(), 1.0)
        });
        self.prompts.set_fg(fg);
        let see_through = crate::seethrough::active();
        let mut look = settings::with(|s| backdrop::Look {
            image: None,
            color: v.bg,
            opacity: s.bg_opacity as f64 / 100.0,
            fit: s.bg_fit.clone(),
            position: s.bg_position.clone(),
            shared: s.bg_shared,
            see_through,
        });
        let (path, blur) = settings::with(|s| (s.bg_image.clone(), s.bg_blur));
        look.image = backdrop::image(&path, blur);
        if see_through && look.color.is_none() {
            look.color = view_background(self.term.widget());
        }
        let clear = look.image.is_some() || see_through;
        backdrop::clear_cells(self.term.widget(), clear);
        if see_through {
            self.layers.remove_css_class("view");
        } else {
            self.layers.add_css_class("view");
        }
        let bg = if clear { gtk::gdk::RGBA::TRANSPARENT } else { v.bg.unwrap_or(gtk::gdk::RGBA::TRANSPARENT) };
        self.backdrop.set(look);
        self.term.set_colors(&fg, &bg, &v.palette, v.cursor.as_ref());
    }

    pub fn spawn(self: &Rc<Self>, dir: &Path) {
        if self.pid.get().is_some() || self.spawning.get() {
            return;
        }
        let Some(panel) = self.panel.upgrade() else { return };
        self.spawning.set(true);
        self.shell_dir.replace(Some(dir.to_path_buf()));
        let launch = shell::launch(&user_shell());
        self.integrated.set(launch.integrated);
        let _ = std::fs::remove_file(&self.state_file);
        let env: Vec<String> = [
            "SHELLMAN=1".to_string(),
            format!("SHELLMAN_SEL={}", panel.sel_file().display()),
            format!("SHELLMAN_CD={}", self.cd_file.display()),
            format!("SHELLMAN_STATE={}", self.state_file.display()),
        ].into_iter()
            .chain(settings::with(|s| s.ls_links).then(|| "SHELLMAN_LS_LINKS=1".to_string()))
            .chain(launch.env).collect();
        let weak = Rc::downgrade(self);
        self.term.spawn(dir, &launch.argv, &env, move |pid| {
            let Some(p) = weak.upgrade() else { return };
            p.spawning.set(false);
            p.pid.set(pid);
            if let (Some(_), Some(panel)) = (pid, p.panel.upgrade()) {
                panel.follow();
            }
        });
    }

    /// test hook: types one line every 700 ms
    pub fn test_input(self: &Rc<Self>, input: &str) {
        self.prompts.hovered.set(true);
        for (i, line) in input.split(';').enumerate() {
            let weak = Rc::downgrade(self);
            let line = format!("{}\r", line.trim());
            glib::timeout_add_local_once(std::time::Duration::from_millis(1000 + 700 * i as u64), move || {
                if let Some(p) = weak.upgrade() {
                    guard("test input", (), || p.term.feed(&line));
                }
            });
        }
    }

    pub fn stop(&self) {
        self.pending.replace(None);
        let _ = std::fs::remove_file(&self.cd_file);
        let _ = std::fs::remove_file(&self.state_file);
        let mut c = self.term.widget().first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            if w.is::<gtk::Popover>() {
                w.unparent();
            }
        }
    }
}
