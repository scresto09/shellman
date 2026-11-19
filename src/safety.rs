// Safe mode: when Nautilus crashes while ShellMan is set up in its windows, a marker
// file is written from the signal handler; on the next start ShellMan stays out of the
// windows until the user turns it back on from the folder context menu.

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use gtk::glib;

use crate::log;

const SIGNALS: [libc::c_int; 5] = [libc::SIGSEGV, libc::SIGBUS, libc::SIGABRT, libc::SIGILL, libc::SIGFPE];

static MARKER: AtomicPtr<libc::c_char> = AtomicPtr::new(std::ptr::null_mut());
static ARMED: AtomicBool = AtomicBool::new(false);
static SAFE_MODE: AtomicBool = AtomicBool::new(false);
static mut PREVIOUS: [std::mem::MaybeUninit<libc::sigaction>; 5] = [std::mem::MaybeUninit::uninit(); 5];

fn marker() -> PathBuf {
    glib::user_cache_dir().join("shellman").join("crashed")
}

pub fn crashed() -> bool {
    marker().exists()
}

pub fn safe_mode() -> bool {
    SAFE_MODE.load(Ordering::Relaxed)
}

pub fn set_safe_mode(on: bool) {
    SAFE_MODE.store(on, Ordering::Relaxed);
}

// Nautilus is often restarted in the background (desktop icons, search): the user is told once
pub fn first_notice() -> bool {
    let path = marker();
    if std::fs::read(&path).is_ok_and(|c| c == NOTIFIED) {
        return false;
    }
    let _ = std::fs::write(&path, NOTIFIED);
    true
}

const NOTIFIED: &[u8] = b"notified\n";

pub fn clear() {
    let _ = std::fs::remove_file(marker());
}

// only async-signal-safe calls: open, write, close, sigaction
extern "C" fn on_fatal(sig: libc::c_int) {
    let path = MARKER.load(Ordering::Relaxed);
    if !path.is_null() {
        unsafe {
            let fd = libc::open(path, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC, 0o600);
            if fd >= 0 {
                let _ = libc::write(fd, b"crash\n".as_ptr() as *const libc::c_void, 6);
                libc::close(fd);
            }
        }
    }
    // the previous handler (usually the default one) gets the signal again once this one returns
    if let Some(i) = SIGNALS.iter().position(|&s| s == sig) {
        unsafe {
            let prev = (*std::ptr::addr_of!(PREVIOUS))[i].as_ptr();
            libc::sigaction(sig, prev, std::ptr::null_mut());
            libc::raise(sig);
        }
    }
}

pub fn arm() {
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let path = marker();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(c) = CString::new(path.into_os_string().into_encoded_bytes()) else { return };
    MARKER.store(c.into_raw(), Ordering::Relaxed);
    for (i, &sig) in SIGNALS.iter().enumerate() {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_fatal as usize;
            action.sa_flags = libc::SA_RESETHAND;
            libc::sigemptyset(&mut action.sa_mask);
            let prev = (*std::ptr::addr_of_mut!(PREVIOUS))[i].as_mut_ptr();
            if libc::sigaction(sig, &action, prev) != 0 {
                log!("safe mode: cannot watch signal {sig}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn crash_child() {
        if std::env::var_os("SHELLMAN_CRASH_CHILD").is_none() {
            return;
        }
        arm();
        unsafe { std::ptr::read_volatile(std::ptr::null::<u8>()) };
    }

    #[test]
    fn a_crash_leaves_the_marker() {
        let cache = std::env::temp_dir().join(format!("shellman-crash-{}", std::process::id()));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "safety::tests::crash_child", "--test-threads=1"])
            .env("SHELLMAN_CRASH_CHILD", "1")
            .env("XDG_CACHE_HOME", &cache)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.signal(), Some(libc::SIGSEGV));
        assert!(cache.join("shellman/crashed").exists());
        std::fs::remove_dir_all(&cache).unwrap();
    }
}
