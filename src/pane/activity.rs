// Running commands: polled by the panel for the indicator on its button, and a notification
// when a long command ends while its terminal cannot be seen.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gio, glib};

use super::Pane;
use crate::i18n::tr;
use crate::settings;
use crate::util::guard;

const NOTIFY_AFTER: Duration = Duration::from_secs(10);
// below this, a command is not shown as running: the shell starting up, a quick ls
const SHOW_AFTER: Duration = Duration::from_secs(1);
const SHOW_ACTION: &str = "shellman-show";

thread_local! {
    static PANES: RefCell<HashMap<u64, Weak<Pane>>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

pub(super) fn register(pane: &Rc<Pane>) {
    let id = NEXT_ID.replace(NEXT_ID.get() + 1);
    pane.activity_id.set(id);
    PANES.with(|p| {
        let mut p = p.borrow_mut();
        p.retain(|_, w| w.strong_count() > 0);
        p.insert(id, Rc::downgrade(pane));
    });
    ensure_action();
}

// clicking the notification brings back the window and the terminal
fn ensure_action() {
    let Some(app) = gio::Application::default() else { return };
    if app.lookup_action(SHOW_ACTION).is_some() {
        return;
    }
    let action = gio::SimpleAction::new(SHOW_ACTION, Some(glib::VariantTy::UINT64));
    action.connect_activate(|_, v| guard("show terminal", (), || {
        let Some(id) = v.and_then(|v| v.get::<u64>()) else { return };
        let pane = PANES.with(|p| p.borrow().get(&id).and_then(Weak::upgrade));
        if let Some((pane, panel)) = pane.and_then(|p| p.panel.upgrade().map(|panel| (p, panel))) {
            panel.present_pane(&pane);
        }
    }));
    app.add_action(&action);
}

fn duration_text(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min {:02} s", s / 60, s % 60),
        _ => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
    }
}

impl Pane {
    /// Called every second; true while a command has been running for a moment.
    pub fn poll_activity(&self) -> bool {
        let busy = self.pid.get().is_some() && !self.at_prompt();
        match (busy, self.busy_since.get()) {
            (true, None) => self.busy_since.set(Some(Instant::now())),
            (false, Some(start)) => {
                self.busy_since.set(None);
                self.command_done(start.elapsed());
            }
            _ => {}
        }
        self.busy_since.get().is_some_and(|s| s.elapsed() >= SHOW_AFTER)
    }

    fn seen(&self) -> bool {
        self.widget.is_mapped()
            && self.widget.root().and_downcast::<gtk::Window>().is_some_and(|w| w.is_active())
    }

    fn command_done(&self, took: Duration) {
        if took < NOTIFY_AFTER || self.seen() || !settings::with(|s| s.notify_done) {
            return;
        }
        let Some(app) = gio::Application::default() else { return };
        let folder = self.shell_dir()
            .map(|d| d.file_name().map_or_else(|| d.display().to_string(), |n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let body = if folder.is_empty() {
            tr("After {time}").replace("{time}", &duration_text(took))
        } else {
            tr("In {folder}, after {time}").replace("{folder}", &folder).replace("{time}", &duration_text(took))
        };
        let n = gio::Notification::new(&tr("Command Finished"));
        n.set_body(Some(&body));
        n.set_default_action_and_target_value(&format!("app.{SHOW_ACTION}"), Some(&self.activity_id.get().to_variant()));
        app.send_notification(Some("shellman-command-done"), &n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration_text(Duration::from_secs(45)), "45 s");
        assert_eq!(duration_text(Duration::from_secs(125)), "2 min 05 s");
        assert_eq!(duration_text(Duration::from_secs(3725)), "1 h 02 min");
    }
}
