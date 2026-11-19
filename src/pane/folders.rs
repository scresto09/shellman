use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;

use super::links::local_path;
use super::Pane;
use crate::log;
use crate::settings;
use crate::shell;
use crate::util::guard;

impl Pane {
    pub fn sync_mode(&self) -> settings::Sync {
        self.sync_override.get().unwrap_or_else(|| settings::with(|s| s.sync))
    }

    pub fn follows(&self) -> bool {
        self.sync_mode().terminal_follows()
    }

    pub(super) fn set_sync(self: &Rc<Self>, mode: settings::Sync) {
        self.sync_override.set((mode != settings::with(|s| s.sync)).then_some(mode));
        if let Some(a) = self.actions.lookup_action("sync") {
            a.change_state(&mode.id().to_variant());
        }
        if let Some(panel) = self.panel.upgrade() {
            guard("follow folder", (), || panel.follow());
        }
        // with both ways, Nautilus is the reference when the mode is chosen
        if mode == settings::Sync::NautilusFollows {
            self.lead_nautilus();
        }
    }

    pub fn shell_dir(&self) -> Option<PathBuf> {
        self.shell_dir.borrow().clone()
    }

    pub(super) fn cwd_changed(self: &Rc<Self>) {
        if let Some(d) = self.term.cwd() {
            self.shell_dir.replace(Some(d));
        }
        guard("nautilus follows the shell", (), || self.lead_nautilus());
    }

    pub fn go_to(self: &Rc<Self>, dir: Option<PathBuf>) {
        if self.pid.get().is_none() || !self.integrated.get() {
            return;
        }
        let Some(dir) = dir else {
            self.pending.replace(None);
            return;
        };
        if self.shell_dir.borrow().as_deref() == Some(dir.as_path()) {
            self.pending.replace(None);
            return;
        }
        self.pending.replace(Some(dir));
        self.flush();
    }

    // runs the pending cd once the shell waits at its prompt
    pub(super) fn flush(&self) {
        let Some(pid) = self.pid.get() else {
            self.pending.replace(None);
            return;
        };
        if !self.panel.upgrade().is_some_and(|p| p.shown()) {
            self.pending.replace(None);
            return;
        }
        let at_prompt = std::fs::read(&self.state_file).is_ok_and(|s| s == b"prompt");
        if !at_prompt || !self.term.shell_idle(pid) {
            return;
        }
        if let Some(dir) = self.pending.take() {
            // the path goes through a file: only a fixed key is sent to the terminal
            let mut content = dir.as_os_str().as_bytes().to_vec();
            content.push(0);
            if let Err(e) = shell::write_atomic(&self.cd_file, &content) {
                log!("cannot write {}: {e}", self.cd_file.display());
                return;
            }
            self.term.feed(shell::CD_KEY);
            self.shell_dir.replace(Some(dir));
        }
    }

    pub(super) fn goto_tab_dir(self: &Rc<Self>) {
        let dir = self.panel.upgrade().and_then(|p| p.current_dir());
        if !self.integrated.get() || dir.is_none() {
            self.term.widget().error_bell();
            return;
        }
        self.shell_dir.replace(None);
        self.go_to(dir);
    }

    // the reverse of following: a cd in the shell moves the Nautilus tab
    fn lead_nautilus(self: &Rc<Self>) {
        if !self.sync_mode().nautilus_follows() {
            return;
        }
        let Some(dir) = self.term.cwd_uri().and_then(|u| local_path(&u)).filter(|d| d.is_dir()) else { return };
        if let Some(panel) = self.panel.upgrade() {
            panel.show_folder(self, &dir);
        }
    }

    pub(super) fn show_in_nautilus(self: &Rc<Self>) {
        let dir = self.term.cwd_uri().and_then(|u| local_path(&u)).or_else(|| self.shell_dir())
            .filter(|d| d.is_dir());
        match (dir, self.panel.upgrade()) {
            (Some(dir), Some(panel)) => panel.show_folder(self, &dir),
            _ => self.term.widget().error_bell(),
        }
    }
}
