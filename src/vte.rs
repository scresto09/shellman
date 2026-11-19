#![allow(non_camel_case_types)]

use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::os::raw::{c_char, c_int, c_long};
use std::path::{Path, PathBuf};
use std::ptr;

use gtk::gdk::ffi::GdkRGBA;
use gtk::gio::{self, prelude::FileExt};
use gtk::glib::{self, ffi::gpointer, prelude::*, translate::*};
use gtk::pango;

use crate::log;
use crate::util::cstr;
use crate::util::guard;

pub enum VteTerminalC {}
pub enum VtePtyC {}
pub enum VteRegexC {}

type SpawnCallback =
    unsafe extern "C" fn(*mut VteTerminalC, glib::ffi::GPid, *mut glib::ffi::GError, gpointer);

const VTE_PTY_DEFAULT: c_int = 0;
const VTE_FORMAT_TEXT: c_int = 1;
const G_SPAWN_SEARCH_PATH: c_int = 1 << 2;
const PCRE2_CASELESS: u32 = 0x0000_0008;
const PCRE2_MULTILINE: u32 = 0x0000_0400;
const PCRE2_UCP: u32 = 0x0002_0000;
const PCRE2_UTF: u32 = 0x0008_0000;
const PCRE2_NO_UTF_CHECK: u32 = 0x4000_0000;

#[link(name = "libvte-2.91-gtk4.so.0", kind = "dylib", modifiers = "+verbatim")]
extern "C" {
    fn vte_terminal_new() -> *mut gtk::ffi::GtkWidget;
    #[allow(clippy::too_many_arguments)]
    fn vte_terminal_spawn_async(
        t: *mut VteTerminalC, pty_flags: c_int, working_directory: *const c_char,
        argv: *mut *mut c_char, envv: *mut *mut c_char, spawn_flags: c_int,
        child_setup: Option<unsafe extern "C" fn(gpointer)>, child_setup_data: gpointer,
        child_setup_data_destroy: Option<unsafe extern "C" fn(gpointer)>, timeout: c_int,
        cancellable: *mut gio::ffi::GCancellable, callback: Option<SpawnCallback>,
        user_data: gpointer);
    fn vte_terminal_feed(t: *mut VteTerminalC, data: *const c_char, length: isize);
    fn vte_terminal_feed_child(t: *mut VteTerminalC, text: *const c_char, length: isize);
    fn vte_terminal_copy_clipboard_format(t: *mut VteTerminalC, format: c_int);
    fn vte_terminal_paste_clipboard(t: *mut VteTerminalC);
    fn vte_terminal_set_scrollback_lines(t: *mut VteTerminalC, lines: c_long);
    fn vte_terminal_get_pty(t: *mut VteTerminalC) -> *mut VtePtyC;
    fn vte_pty_get_fd(p: *mut VtePtyC) -> c_int;
    fn vte_terminal_set_colors(t: *mut VteTerminalC, fg: *const GdkRGBA, bg: *const GdkRGBA,
                               palette: *const GdkRGBA, palette_size: usize);
    fn vte_terminal_set_color_cursor(t: *mut VteTerminalC, color: *const GdkRGBA);
    fn vte_terminal_set_font(t: *mut VteTerminalC,
                             desc: *const pango::ffi::PangoFontDescription);
    fn vte_terminal_get_current_directory_uri(t: *mut VteTerminalC) -> *const c_char;
    fn vte_get_user_shell() -> *mut c_char;
    fn vte_terminal_get_has_selection(t: *mut VteTerminalC) -> glib::ffi::gboolean;
    fn vte_terminal_select_all(t: *mut VteTerminalC);
    fn vte_terminal_get_char_height(t: *mut VteTerminalC) -> c_long;
    fn vte_terminal_set_font_scale(t: *mut VteTerminalC, scale: f64);
    fn vte_terminal_get_font_scale(t: *mut VteTerminalC) -> f64;
    fn vte_terminal_unselect_all(t: *mut VteTerminalC);
    fn vte_terminal_set_cursor_shape(t: *mut VteTerminalC, shape: c_int);
    fn vte_terminal_get_cursor_position(t: *mut VteTerminalC, column: *mut c_long, row: *mut c_long);
    fn vte_terminal_get_column_count(t: *mut VteTerminalC) -> c_long;
    fn vte_terminal_get_char_width(t: *mut VteTerminalC) -> c_long;
    fn vte_terminal_set_allow_hyperlink(t: *mut VteTerminalC, allow: glib::ffi::gboolean);
    fn vte_terminal_check_hyperlink_at(t: *mut VteTerminalC, x: f64, y: f64) -> *mut c_char;
    fn vte_regex_new_for_match(pattern: *const c_char, length: isize, flags: u32,
                               error: *mut *mut glib::ffi::GError) -> *mut VteRegexC;
    fn vte_terminal_match_add_regex(t: *mut VteTerminalC, regex: *mut VteRegexC, flags: u32) -> c_int;
    fn vte_terminal_match_set_cursor_name(t: *mut VteTerminalC, tag: c_int, cursor: *const c_char);
    fn vte_terminal_check_match_at(t: *mut VteTerminalC, x: f64, y: f64, tag: *mut c_int) -> *mut c_char;
    fn vte_regex_new_for_search(pattern: *const c_char, length: isize, flags: u32,
                                error: *mut *mut glib::ffi::GError) -> *mut VteRegexC;
    fn vte_regex_unref(r: *mut VteRegexC) -> *mut VteRegexC;
    fn vte_terminal_search_set_regex(t: *mut VteTerminalC, r: *mut VteRegexC, flags: u32);
    fn vte_terminal_search_set_wrap_around(t: *mut VteTerminalC, wrap: glib::ffi::gboolean);
    fn vte_terminal_search_find_next(t: *mut VteTerminalC) -> glib::ffi::gboolean;
    fn vte_terminal_search_find_previous(t: *mut VteTerminalC) -> glib::ffi::gboolean;
}


fn literal_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars().filter(|&c| c != '\0') {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn rgba(c: &gtk::gdk::RGBA) -> GdkRGBA {
    GdkRGBA { red: c.red(), green: c.green(), blue: c.blue(), alpha: c.alpha() }
}

pub fn user_shell() -> String {
    unsafe {
        let p = vte_get_user_shell();
        if !p.is_null() {
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            glib::ffi::g_free(p as gpointer);
            if !s.is_empty() {
                return s;
            }
        }
    }
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

type OnSpawn = Box<dyn FnOnce(Option<i32>)>;

unsafe extern "C" fn spawn_done(_t: *mut VteTerminalC, pid: glib::ffi::GPid,
                                err: *mut glib::ffi::GError, data: gpointer) {
    let cb = Box::from_raw(data as *mut OnSpawn);
    guard("shell spawn", (), move || {
        if !err.is_null() {
            let msg = CStr::from_ptr((*err).message).to_string_lossy();
            log!("cannot start the shell: {msg}");
        }
        cb((pid > 0).then_some(pid));
    })
}

fn match_regex(pattern: &str) -> Option<*mut VteRegexC> {
    let flags = PCRE2_UTF | PCRE2_NO_UTF_CHECK | PCRE2_UCP | PCRE2_MULTILINE | PCRE2_CASELESS;
    let mut err: *mut glib::ffi::GError = ptr::null_mut();
    unsafe {
        let r = vte_regex_new_for_match(pattern.as_ptr() as *const c_char, pattern.len() as isize, flags, &mut err);
        if r.is_null() {
            if !err.is_null() {
                log!("invalid link pattern: {}", CStr::from_ptr((*err).message).to_string_lossy());
                glib::ffi::g_error_free(err);
            }
            return None;
        }
        Some(r)
    }
}

/// whether VTE accepts a pattern for `add_match`
pub fn match_pattern_ok(pattern: &str) -> bool {
    match match_regex(pattern) {
        Some(r) => {
            unsafe { vte_regex_unref(r) };
            true
        }
        None => false,
    }
}

#[derive(Clone)]
pub struct Terminal(gtk::Widget);

impl Default for Terminal {
    fn default() -> Self {
        Terminal::new()
    }
}

impl Terminal {
    pub fn new() -> Terminal {
        let w: gtk::Widget = unsafe { from_glib_none(vte_terminal_new()) };
        let t = Terminal(w);
        unsafe {
            vte_terminal_search_set_wrap_around(t.ptr(), glib::ffi::GTRUE);
            vte_terminal_set_allow_hyperlink(t.ptr(), glib::ffi::GTRUE);
        }
        t
    }

    pub fn widget(&self) -> &gtk::Widget {
        &self.0
    }

    fn ptr(&self) -> *mut VteTerminalC {
        self.0.as_ptr() as *mut VteTerminalC
    }

    pub fn spawn(&self, dir: &Path, argv: &[String], env: &[String],
                 done: impl FnOnce(Option<i32>) + 'static) {
        let wd = CString::new(dir.as_os_str().as_bytes()).unwrap_or_default();
        let argv_c: Vec<CString> = argv.iter().map(|a| cstr(a)).collect();
        let env_c: Vec<CString> = env.iter().map(|e| cstr(e)).collect();
        let nul_list = |v: &[CString]| -> Vec<*mut c_char> {
            v.iter().map(|c| c.as_ptr() as *mut c_char).chain([ptr::null_mut()]).collect()
        };
        let mut argv = nul_list(&argv_c);
        let mut envv = nul_list(&env_c);
        let data: Box<OnSpawn> = Box::new(Box::new(done));
        unsafe {
            vte_terminal_spawn_async(
                self.ptr(), VTE_PTY_DEFAULT, wd.as_ptr(), argv.as_mut_ptr(), envv.as_mut_ptr(),
                G_SPAWN_SEARCH_PATH, None, ptr::null_mut(), None, -1, ptr::null_mut(),
                Some(spawn_done), Box::into_raw(data) as gpointer);
        }
    }

    pub fn feed(&self, text: &str) {
        unsafe { vte_terminal_feed_child(self.ptr(), text.as_ptr() as *const c_char,
                                         text.len() as isize) }
    }

    /// data processed as if the program had written it
    pub fn feed_display(&self, data: &str) {
        unsafe { vte_terminal_feed(self.ptr(), data.as_ptr() as *const c_char, data.len() as isize) }
    }

    pub fn shell_idle(&self, pid: i32) -> bool {
        unsafe {
            let pty = vte_terminal_get_pty(self.ptr());
            if pty.is_null() {
                return false;
            }
            let fd = vte_pty_get_fd(pty);
            fd >= 0 && libc::tcgetpgrp(fd) == pid
        }
    }

    pub fn cwd(&self) -> Option<PathBuf> {
        gio::File::for_uri(&self.cwd_uri()?).path()
    }

    pub fn cwd_uri(&self) -> Option<String> {
        let p = unsafe { vte_terminal_get_current_directory_uri(self.ptr()) };
        if p.is_null() {
            return None;
        }
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }

    pub fn copy(&self) {
        unsafe { vte_terminal_copy_clipboard_format(self.ptr(), VTE_FORMAT_TEXT) }
    }

    pub fn has_selection(&self) -> bool {
        unsafe { vte_terminal_get_has_selection(self.ptr()) != 0 }
    }

    pub fn font_scale(&self) -> f64 {
        unsafe { vte_terminal_get_font_scale(self.ptr()) }
    }

    pub fn set_font_scale(&self, scale: f64) {
        unsafe { vte_terminal_set_font_scale(self.ptr(), scale) }
    }

    pub fn char_height(&self) -> i32 {
        unsafe { vte_terminal_get_char_height(self.ptr()) as i32 }
    }

    pub fn select_all(&self) {
        unsafe { vte_terminal_select_all(self.ptr()) }
    }

    /// URI of the OSC 8 hyperlink at this point of the widget
    pub fn hyperlink_at(&self, x: f64, y: f64) -> Option<String> {
        unsafe {
            let p = vte_terminal_check_hyperlink_at(self.ptr(), x, y);
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            glib::ffi::g_free(p as gpointer);
            Some(s)
        }
    }

    /// Text matching `pattern` gets underlined under the pointer; see `match_at`.
    pub fn add_match(&self, pattern: &str) {
        let Some(r) = match_regex(pattern) else { return };
        unsafe {
            let tag = vte_terminal_match_add_regex(self.ptr(), r, 0);
            vte_regex_unref(r);
            vte_terminal_match_set_cursor_name(self.ptr(), tag, c"pointer".as_ptr());
        }
    }

    pub fn match_at(&self, x: f64, y: f64) -> Option<String> {
        unsafe {
            let mut tag: c_int = -1;
            let p = vte_terminal_check_match_at(self.ptr(), x, y, &mut tag);
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            glib::ffi::g_free(p as gpointer);
            Some(s)
        }
    }

    pub fn set_bar_cursor(&self, bar: bool) {
        unsafe { vte_terminal_set_cursor_shape(self.ptr(), if bar { 1 } else { 0 }) }
    }

    /// (column, row) of the cursor; the row counts from the top of the scrollback
    #[allow(clippy::useless_conversion)] // c_long is i32 on 32-bit systems
    pub fn cursor(&self) -> (i64, i64) {
        let (mut col, mut row): (c_long, c_long) = (0, 0);
        unsafe { vte_terminal_get_cursor_position(self.ptr(), &mut col, &mut row) };
        (col.into(), row.into())
    }

    #[allow(clippy::useless_conversion)] // c_long is i32 on 32-bit systems
    pub fn columns(&self) -> i64 {
        unsafe { vte_terminal_get_column_count(self.ptr()).into() }
    }

    pub fn char_width(&self) -> i32 {
        unsafe { vte_terminal_get_char_width(self.ptr()) as i32 }
    }

    pub fn unselect(&self) {
        unsafe { vte_terminal_unselect_all(self.ptr()) }
    }

    /// Literal search, case-insensitive unless the text contains an uppercase letter.
    pub fn set_search(&self, text: &str) {
        if text.is_empty() {
            unsafe { vte_terminal_search_set_regex(self.ptr(), ptr::null_mut(), 0) }
            return;
        }
        let mut flags = PCRE2_UTF | PCRE2_NO_UTF_CHECK | PCRE2_UCP | PCRE2_MULTILINE;
        if !text.chars().any(char::is_uppercase) {
            flags |= PCRE2_CASELESS;
        }
        let pattern = literal_pattern(text);
        let mut err: *mut glib::ffi::GError = ptr::null_mut();
        unsafe {
            let r = vte_regex_new_for_search(pattern.as_ptr() as *const c_char,
                                             pattern.len() as isize, flags, &mut err);
            if r.is_null() {
                if !err.is_null() {
                    log!("invalid search: {}", CStr::from_ptr((*err).message).to_string_lossy());
                    glib::ffi::g_error_free(err);
                }
                vte_terminal_search_set_regex(self.ptr(), ptr::null_mut(), 0);
                return;
            }
            vte_terminal_search_set_regex(self.ptr(), r, 0);
            vte_regex_unref(r);
        }
    }

    pub fn find(&self, backward: bool) -> bool {
        unsafe {
            if backward {
                vte_terminal_search_find_previous(self.ptr()) != 0
            } else {
                vte_terminal_search_find_next(self.ptr()) != 0
            }
        }
    }

    pub fn paste(&self) {
        unsafe { vte_terminal_paste_clipboard(self.ptr()) }
    }

    pub fn set_scrollback(&self, lines: i64) {
        unsafe { vte_terminal_set_scrollback_lines(self.ptr(), lines as c_long) }
    }

    pub fn set_colors(&self, fg: &gtk::gdk::RGBA, bg: &gtk::gdk::RGBA, palette: &[gtk::gdk::RGBA],
                      cursor: Option<&gtk::gdk::RGBA>) {
        let (fg, bg) = (rgba(fg), rgba(bg));
        let palette: Vec<GdkRGBA> = palette.iter().map(rgba).collect();
        let p = if palette.is_empty() { ptr::null() } else { palette.as_ptr() };
        let cursor = cursor.map(rgba);
        unsafe {
            vte_terminal_set_colors(self.ptr(), &fg, &bg, p, palette.len());
            vte_terminal_set_color_cursor(self.ptr(), cursor.as_ref().map_or(ptr::null(), |c| c as *const _));
        }
    }

    pub fn set_font(&self, desc: &str) {
        let d = pango::FontDescription::from_string(desc);
        unsafe { vte_terminal_set_font(self.ptr(), d.to_glib_none().0) }
    }

    pub fn connect_child_exited(&self, f: impl Fn() + 'static) {
        self.0.connect_local("child-exited", false, move |_| {
            guard("shell exit", (), &f);
            None
        });
    }

    pub fn connect_contents_changed(&self, f: impl Fn() + 'static) {
        self.0.connect_local("contents-changed", false, move |_| {
            guard("terminal contents", (), &f);
            None
        });
    }

    pub fn connect_cwd_changed(&self, f: impl Fn() + 'static) {
        if glib::subclass::SignalId::lookup("current-directory-uri-changed", self.0.type_()).is_none() {
            log!("signal current-directory-uri-changed missing");
            return;
        }
        self.0.connect_local("current-directory-uri-changed", false, move |_| {
            guard("shell directory", (), &f);
            None
        });
    }
}
