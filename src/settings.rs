use std::cell::RefCell;
use std::path::PathBuf;

use gtk::glib;

use crate::config;
use crate::log;

const GROUP: &str = "terminal";
const KEYS: &str = "shortcuts";

/// Folder synchronisation between the terminal and the Nautilus tab
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sync {
    Both,
    TerminalFollows,
    NautilusFollows,
    Off,
}

impl Sync {
    pub const ALL: [Sync; 4] = [Sync::Both, Sync::TerminalFollows, Sync::NautilusFollows, Sync::Off];

    pub fn id(self) -> &'static str {
        match self {
            Sync::Both => "both",
            Sync::TerminalFollows => "terminal",
            Sync::NautilusFollows => "nautilus",
            Sync::Off => "off",
        }
    }

    pub fn from_id(id: &str) -> Option<Sync> {
        Sync::ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn terminal_follows(self) -> bool {
        matches!(self, Sync::Both | Sync::TerminalFollows)
    }

    pub fn nautilus_follows(self) -> bool {
        matches!(self, Sync::Both | Sync::NautilusFollows)
    }

    pub fn from_parts(terminal: bool, nautilus: bool) -> Sync {
        match (terminal, nautilus) {
            (true, true) => Sync::Both,
            (true, false) => Sync::TerminalFollows,
            (false, true) => Sync::NautilusFollows,
            (false, false) => Sync::Off,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Settings {
    pub sync: Sync,
    pub per_tab: bool,
    pub show_in_new_tabs: bool,
    pub rows: i32,
    pub scrollback: i64,
    pub font: Option<String>,
    pub palette: String,
    pub style: String,
    pub bg_image: String,
    pub bg_opacity: i32,
    pub bg_blur: i32,
    pub bg_fit: String,
    pub bg_position: String,
    pub bg_shared: bool,
    pub see_through: bool,
    pub ls_links: bool,
    pub notify_done: bool,
    pub cursor_bar: bool,
    pub prompt_marks: bool,
    pub prompt_buttons: bool,
    pub shortcuts: Vec<(String, Vec<String>)>,
}

pub fn default_shortcuts() -> Vec<(String, Vec<String>)> {
    config::ACTIONS.iter().map(|(action, _)| {
        let accels = config::DEFAULT_SHORTCUTS.iter()
            .filter(|(_, a)| a == action).map(|(k, _)| k.to_string()).collect();
        (action.to_string(), accels)
    }).collect()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sync: Sync::Both,
            per_tab: true,
            show_in_new_tabs: false,
            rows: config::MIN_ROWS,
            scrollback: config::SCROLLBACK_LINES,
            font: None,
            palette: crate::colors::FOLLOW_TERMINAL.into(),
            style: "system".into(),
            bg_image: String::new(),
            bg_opacity: 85,
            bg_blur: 0,
            bg_fit: "none".into(),
            bg_position: "top-left".into(),
            bg_shared: true,
            see_through: false,
            ls_links: true,
            notify_done: true,
            cursor_bar: true,
            prompt_marks: true,
            prompt_buttons: true,
            shortcuts: default_shortcuts(),
        }
    }
}

fn path() -> PathBuf {
    glib::user_config_dir().join("shellman").join("settings.ini")
}

fn string(kf: &glib::KeyFile, key: &str, default: String) -> String {
    kf.string(GROUP, key).ok().map(|s| s.to_string()).filter(|s| !s.is_empty()).unwrap_or(default)
}

fn load() -> Settings {
    let d = Settings::default();
    let kf = glib::KeyFile::new();
    if kf.load_from_file(path(), glib::KeyFileFlags::NONE).is_err() {
        return d;
    }
    Settings {
        sync: kf.string(GROUP, "sync").ok().and_then(|v| Sync::from_id(&v)).unwrap_or(d.sync),
        per_tab: kf.boolean(GROUP, "per_tab").unwrap_or(d.per_tab),
        show_in_new_tabs: kf.boolean(GROUP, "show_in_new_tabs").unwrap_or(d.show_in_new_tabs),
        rows: kf.integer(GROUP, "rows").map(|v| v.clamp(1, 100)).unwrap_or(d.rows),
        scrollback: kf.int64(GROUP, "scrollback").map(|v| v.max(0)).unwrap_or(d.scrollback),
        font: kf.string(GROUP, "font").ok().map(|s| s.to_string()).filter(|s| !s.is_empty()),
        palette: string(&kf, "palette", d.palette),
        style: Some(string(&kf, "style", d.style.clone()))
            .filter(|v| ["system", "light", "dark"].contains(&v.as_str())).unwrap_or(d.style),
        bg_image: kf.string(GROUP, "bg_image").map(|s| s.to_string()).unwrap_or_default(),
        bg_opacity: kf.integer(GROUP, "bg_opacity").map(|v| v.clamp(0, 100)).unwrap_or(d.bg_opacity),
        bg_blur: kf.integer(GROUP, "bg_blur").map(|v| v.clamp(0, 100)).unwrap_or(d.bg_blur),
        bg_fit: Some(string(&kf, "bg_fit", d.bg_fit.clone()))
            .filter(|v| crate::backdrop::FITS.iter().any(|f| f.0 == v)).unwrap_or(d.bg_fit),
        bg_shared: kf.boolean(GROUP, "bg_shared").unwrap_or(d.bg_shared),
        see_through: kf.boolean(GROUP, "see_through").unwrap_or(d.see_through),
        ls_links: kf.boolean(GROUP, "ls_links").unwrap_or(d.ls_links),
        notify_done: kf.boolean(GROUP, "notify_done").unwrap_or(d.notify_done),
        cursor_bar: kf.boolean(GROUP, "cursor_bar").unwrap_or(d.cursor_bar),
        prompt_marks: kf.boolean(GROUP, "prompt_marks").unwrap_or(d.prompt_marks),
        prompt_buttons: kf.boolean(GROUP, "prompt_buttons").unwrap_or(d.prompt_buttons),
        bg_position: Some(string(&kf, "bg_position", d.bg_position.clone()))
            .filter(|v| crate::backdrop::POSITIONS.iter().any(|p| p.0 == v)).unwrap_or(d.bg_position),
        shortcuts: load_shortcuts(&kf, d.shortcuts),
    }
}

// actions missing from the file get their defaults, minus the keys already saved elsewhere
fn load_shortcuts(kf: &glib::KeyFile, defaults: Vec<(String, Vec<String>)>) -> Vec<(String, Vec<String>)> {
    let loaded: Vec<(String, Vec<String>, bool)> = defaults.into_iter().map(|(action, def)| {
        match kf.string(KEYS, &action) {
            Ok(v) => (action, v.split_whitespace().map(String::from).collect(), true),
            Err(_) => (action, def, false),
        }
    }).collect();
    let taken: Vec<_> = loaded.iter().filter(|(_, _, saved)| *saved)
        .flat_map(|(_, accels, _)| accels.iter().filter_map(gtk::accelerator_parse))
        .collect();
    loaded.into_iter().map(|(action, mut accels, saved)| {
        if !saved {
            accels.retain(|k| gtk::accelerator_parse(k).is_none_or(|p| !taken.contains(&p)));
        }
        (action, accels)
    }).collect()
}

fn save(s: &Settings) {
    let kf = glib::KeyFile::new();
    kf.set_string(GROUP, "sync", s.sync.id());
    kf.set_boolean(GROUP, "per_tab", s.per_tab);
    kf.set_boolean(GROUP, "show_in_new_tabs", s.show_in_new_tabs);
    kf.set_integer(GROUP, "rows", s.rows);
    kf.set_int64(GROUP, "scrollback", s.scrollback);
    kf.set_string(GROUP, "font", s.font.as_deref().unwrap_or(""));
    kf.set_string(GROUP, "palette", &s.palette);
    kf.set_string(GROUP, "style", &s.style);
    kf.set_string(GROUP, "bg_image", &s.bg_image);
    kf.set_integer(GROUP, "bg_opacity", s.bg_opacity);
    kf.set_integer(GROUP, "bg_blur", s.bg_blur);
    kf.set_string(GROUP, "bg_fit", &s.bg_fit);
    kf.set_string(GROUP, "bg_position", &s.bg_position);
    kf.set_boolean(GROUP, "bg_shared", s.bg_shared);
    kf.set_boolean(GROUP, "see_through", s.see_through);
    kf.set_boolean(GROUP, "ls_links", s.ls_links);
    kf.set_boolean(GROUP, "notify_done", s.notify_done);
    kf.set_boolean(GROUP, "cursor_bar", s.cursor_bar);
    kf.set_boolean(GROUP, "prompt_marks", s.prompt_marks);
    kf.set_boolean(GROUP, "prompt_buttons", s.prompt_buttons);
    for (action, accels) in &s.shortcuts {
        kf.set_string(KEYS, action, &accels.join(" "));
    }
    let p = path();
    let res = p.parent().map_or(Ok(()), std::fs::create_dir_all)
        .map_err(|e| e.to_string())
        .and_then(|_| kf.save_to_file(&p).map_err(|e| e.to_string()));
    if let Err(e) = res {
        log!("cannot save {}: {e}", p.display());
    }
}

thread_local! {
    static CURRENT: RefCell<Settings> = RefCell::new(load());
}

pub fn get() -> Settings {
    CURRENT.with(|c| c.borrow().clone())
}

pub fn with<R>(f: impl FnOnce(&Settings) -> R) -> R {
    CURRENT.with(|c| f(&c.borrow()))
}

pub fn update(f: impl FnOnce(&mut Settings)) {
    crate::util::guard("settings", (), || {
        let changed = CURRENT.with(|c| {
            let mut s = c.borrow_mut();
            let before = s.clone();
            f(&mut s);
            (*s != before).then(|| s.clone())
        });
        if let Some(s) = changed {
            save(&s);
            crate::panel::apply_settings_all();
        }
    })
}

pub fn accels(action: &str) -> Vec<String> {
    CURRENT.with(|c| c.borrow().shortcuts.iter()
        .find(|(a, _)| a == action).map(|(_, k)| k.clone()).unwrap_or_default())
}

pub fn action_for(ctrl: &gtk::EventControllerKey, key: gtk::gdk::Key, code: u32,
                  state: gtk::gdk::ModifierType) -> Option<String> {
    CURRENT.with(|c| c.borrow().shortcuts.iter()
        .find(|(_, accels)| accels.iter()
            .any(|a| crate::util::shortcut_matches(a, ctrl, key, code, state)))
        .map(|(action, _)| action.clone()))
}

pub fn owner_of(accel: &str, except: &str) -> Option<String> {
    let parsed = gtk::accelerator_parse(accel);
    CURRENT.with(|c| c.borrow().shortcuts.iter()
        .find(|(a, accels)| a != except
              && accels.iter().any(|k| gtk::accelerator_parse(k) == parsed))
        .map(|(a, _)| a.clone()))
}

pub fn set_accel(action: &str, accel: Option<String>) {
    let parsed = accel.as_deref().and_then(gtk::accelerator_parse);
    update(|s| {
        for (a, accels) in s.shortcuts.iter_mut() {
            if a == action {
                *accels = accel.iter().cloned().collect();
            } else if parsed.is_some() {
                accels.retain(|k| gtk::accelerator_parse(k) != parsed);
            }
        }
    });
}

pub fn reset_shortcuts() {
    update(|s| s.shortcuts = default_shortcuts());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_modes_round_trip_and_split() {
        for m in Sync::ALL {
            assert_eq!(Sync::from_id(m.id()), Some(m));
            assert_eq!(Sync::from_parts(m.terminal_follows(), m.nautilus_follows()), m);
        }
        assert_eq!(Sync::from_id("nope"), None);
    }
}
