use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::Pane;
use crate::i18n::tr;
use crate::log;
use crate::shell;
use crate::util::guard;

const MENU_NAME_MAX: usize = 40;

// Web addresses in the output. A link ends on a character that is not punctuation, so that
// "see https://gnome.org." leaves the dot out; parentheses and brackets are kept when balanced.
const URL_PATTERN: &str = concat!(
    r#"(?:\b(?:https?|ftps?)://|\bwww\.)"#,
    r#"(?:[^\s<>"'`()\[\]{}]|\([^\s<>"'`()]*\)|\[[^\s<>"'`\[\]]*\])*"#,
    r#"(?:[^\s<>"'`()\[\]{}.,;:!?]|\([^\s<>"'`()]*\)|\[[^\s<>"'`\[\]]*\])"#,
);

/// the address to open for a matched or OSC 8 link, if it is a web one
fn web_uri(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("www.") {
        return Some(format!("https://{t}"));
    }
    ["http://", "https://", "ftp://", "ftps://"].iter().any(|s| lower.starts_with(s)).then(|| t.to_string())
}

pub(super) fn connect(pane: &Rc<Pane>) {
    let tw = pane.term.widget();
    pane.term.add_match(URL_PATTERN);

    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(pane);
    click.connect_pressed(move |g, n, x, y| {
        let mods = g.current_event_state() & gtk::accelerator_get_default_mod_mask();
        if n != 1 || mods != gdk::ModifierType::CONTROL_MASK {
            return;
        }
        if let Some(p) = weak.upgrade() {
            if guard("link click", false, || p.open_link_at(x, y)) {
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        }
    });
    tw.add_controller(click);

    tw.set_has_tooltip(true);
    let weak = Rc::downgrade(pane);
    tw.connect_query_tooltip(move |_, x, y, keyboard, tip| {
        let Some(p) = weak.upgrade() else { return false };
        guard("link tooltip", false, || {
            if keyboard || !p.has_link_at(x as f64, y as f64) {
                return false;
            }
            tip.set_text(Some(&tr("Ctrl+click to open")));
            true
        })
    });
}

// links from a remote host (ls through ssh) name files of that host: ignored
pub(super) fn local_path(uri: &str) -> Option<PathBuf> {
    let (path, host) = glib::filename_from_uri(uri).ok()?;
    let local = host.as_deref().is_none_or(|h| h.is_empty() || h == "localhost" || h == glib::host_name());
    (local && path.exists()).then_some(path)
}

// "a-very-long-file-name.tar.gz" -> "a-very-lo…me.tar.gz": both ends stay readable
fn ellipsize_middle(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max || max < 3 {
        return s.to_string();
    }
    let tail = (max - 1) / 2;
    let head = max - 1 - tail;
    chars[..head].iter().chain(std::iter::once(&'…')).chain(&chars[chars.len() - tail..]).collect()
}

impl Pane {
    /// Menu entries for the hyperlink at (x, y), remembered for the link-* actions.
    pub(super) fn link_section(&self, x: f64, y: f64, item: &dyn Fn(&str, &str) -> gio::MenuItem) -> gio::Menu {
        let section = gio::Menu::new();
        let target = self.term.hyperlink_at(x, y).and_then(|uri| local_path(&uri));
        let web = if target.is_none() { self.web_link_at(x, y) } else { None };
        if web.is_some() {
            section.append_item(&item(&tr("Open the Link"), "link-web-open"));
            section.append_item(&item(&tr("Copy the Link Address"), "link-web-copy"));
        }
        self.web_link.replace(web);
        if let Some(path) = &target {
            let name = ellipsize_middle(&path.file_name().map_or_else(|| path.display().to_string(),
                                                                      |n| n.to_string_lossy().into_owned()),
                                        MENU_NAME_MAX);
            if path.is_dir() {
                self.set_action_enabled("link-cd", self.integrated.get());
                section.append_item(&item(&tr("Go to “{name}”").replace("{name}", &name), "link-cd"));
            } else {
                section.append_item(&item(&tr("Open “{name}”").replace("{name}", &name), "link-open"));
            }
            section.append_item(&item(&tr("Copy the Path"), "link-copy"));
            section.append_item(&item(&tr("Insert the Path"), "link-insert"));
        }
        self.link.replace(target);
        section
    }

    fn web_link_at(&self, x: f64, y: f64) -> Option<String> {
        match self.term.hyperlink_at(x, y) {
            Some(uri) => web_uri(&uri),
            None => self.term.match_at(x, y).and_then(|t| web_uri(&t)),
        }
    }

    fn has_link_at(&self, x: f64, y: f64) -> bool {
        self.term.hyperlink_at(x, y).is_some_and(|u| local_path(&u).is_some() || web_uri(&u).is_some())
            || self.web_link_at(x, y).is_some()
    }

    // Ctrl+click: a file of `ls` opens like from the menu, a web address in the browser
    fn open_link_at(&self, x: f64, y: f64) -> bool {
        if let Some(path) = self.term.hyperlink_at(x, y).and_then(|u| local_path(&u)) {
            if path.is_dir() {
                return false;
            }
            self.link.replace(Some(path));
            self.link_open();
            return true;
        }
        let Some(uri) = self.web_link_at(x, y) else { return false };
        self.open_web(&uri);
        true
    }

    fn open_web(&self, uri: &str) {
        let window = self.term.widget().root().and_downcast::<gtk::Window>();
        let (uri2, tw) = (uri.to_string(), self.term.widget().downgrade());
        gtk::UriLauncher::new(uri).launch(window.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Err(e) = res {
                log!("cannot open {uri2}: {e}");
                if let Some(w) = tw.upgrade() {
                    w.error_bell();
                }
            }
        });
    }

    pub(super) fn link_web_open(&self) {
        if let Some(uri) = self.web_link.take() {
            self.open_web(&uri);
        }
    }

    pub(super) fn link_web_copy(&self) {
        if let Some(uri) = self.web_link.take() {
            self.term.widget().clipboard().set_text(&uri);
        }
    }

    pub(super) fn link_cd(self: &Rc<Self>) {
        let Some(dir) = self.link.take().filter(|d| d.is_dir()) else { return };
        if !self.integrated.get() {
            self.term.widget().error_bell();
            return;
        }
        self.shell_dir.replace(None);
        self.go_to(Some(dir));
    }

    pub(super) fn link_copy(&self) {
        let Some(path) = self.link.take() else { return };
        self.term.widget().clipboard().set_text(&path.to_string_lossy());
    }

    pub(super) fn link_insert(&self) {
        let Some(path) = self.link.take() else { return };
        match shell::quote(self.shell, &path, shell::utf8_locale()) {
            Some(word) => self.term.feed(&format!("{word} ")),
            None => {
                log!("insertion refused: {} cannot be written safely", path.display());
                self.term.widget().error_bell();
            }
        }
    }

    pub(super) fn link_open(&self) {
        let Some(path) = self.link.take() else { return };
        let uri = gio::File::for_path(&path).uri();
        let ctx = self.term.widget().display().app_launch_context();
        if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, Some(&ctx)) {
            log!("cannot open {}: {e}", path.display());
            self.term.widget().error_bell();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_pattern_compiles() {
        assert!(crate::vte::match_pattern_ok(URL_PATTERN));
    }

    #[test]
    fn web_addresses() {
        assert_eq!(web_uri("https://gnome.org").as_deref(), Some("https://gnome.org"));
        assert_eq!(web_uri("www.debian.org/").as_deref(), Some("https://www.debian.org/"));
        assert_eq!(web_uri("FTP://h/f").as_deref(), Some("FTP://h/f"));
        assert_eq!(web_uri("javascript:alert(1)"), None);
        assert_eq!(web_uri("file:///etc/passwd"), None);
        assert_eq!(web_uri("https://a b"), None);
    }

    #[test]
    fn long_names_are_cut_in_the_middle() {
        assert_eq!(ellipsize_middle("court.txt", 40), "court.txt");
        assert_eq!(ellipsize_middle("abcdefghij", 7), "abc…hij");
        assert_eq!(ellipsize_middle("abcdefghij", 8), "abcd…hij");
        assert_eq!(ellipsize_middle("ééééééééé.pdf", 7).chars().count(), 7);
        assert!(ellipsize_middle("Rapport annuel de la société 2026 version finale.pdf", 40).ends_with("finale.pdf"));
    }

    #[test]
    fn links_must_be_local_existing_files() {
        let host = glib::host_name();
        assert_eq!(local_path("file:///tmp"), Some(PathBuf::from("/tmp")));
        assert_eq!(local_path(&format!("file://{host}/tmp")), Some(PathBuf::from("/tmp")));
        assert_eq!(local_path("file://other-host.example/tmp"), None);
        assert_eq!(local_path("file:///no/such/shellman/path"), None);
        assert_eq!(local_path("https://example.com/"), None);
    }
}
