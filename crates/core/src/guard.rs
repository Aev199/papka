//! Keeping a bug in one piece of work from closing the window.
//!
//! On Windows GPUI runs background tasks in the system thread pool through
//! an `extern "system"` callback: a panic there cannot unwind and ends the
//! whole process without a word. Work that reads files of unknown shape
//! (pictures, drawings, archives) runs through [`catch`], which turns a
//! panic into an ordinary error. Every panic, caught or not, is written to
//! `crash.log` in the profile; one that was not caught also leaves
//! `crash.pending`, which the next start reports once.
//! Only the message, the place in the code and the thread are written —
//! never file contents.

use std::cell::Cell;
use std::io::{self, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

thread_local! {
    /// Nesting of [`catch`] on this thread: its panics are not fatal.
    static CATCHING: Cell<usize> = const { Cell::new(0) };
}

/// Runs `work`; a panic inside comes back as an error naming `what`.
pub fn catch<T>(what: &str, work: impl FnOnce() -> T) -> io::Result<T> {
    CATCHING.with(|depth| depth.set(depth.get() + 1));
    let result = panic::catch_unwind(AssertUnwindSafe(work));
    CATCHING.with(|depth| depth.set(depth.get() - 1));
    result.map_err(|payload| io::Error::other(format!("внутренняя ошибка ({what}): {}", message(payload.as_ref()))))
}

/// Locks `mutex` even after a thread panicked holding it: the data is
/// plain state (lists, flags) that stays usable, and refusing it would
/// turn one failed task into a closed window.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "без описания".into())
}

/// Writes panics to `folder/crash.log` from now on (the profile folder in
/// the program, a temporary one in tests).
pub fn install(folder: PathBuf) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let caught = CATCHING.with(Cell::get) > 0;
        let text = info.payload().downcast_ref::<&str>().map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "без описания".into());
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("без имени").to_string();
        let _ = record(&folder, caught, &text, &place, &thread);
        if !caught { previous(info); }
    }));
}

fn record(folder: &Path, caught: bool, text: &str, place: &str, thread: &str) -> io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let time = chrono::Local::now().format("%d.%m.%Y %H:%M:%S");
    let version = env!("CARGO_PKG_VERSION");
    let line = format!("{time} · {} · {text} · {place} · поток {thread} · ядро {version}\n",
        if caught { "перехвачено" } else { "СБОЙ" });
    let log = folder.join("crash.log");
    // Keeps the log small: the latest entries are what matters.
    if std::fs::metadata(&log).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::rename(&log, folder.join("crash.old.log"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(&log)?.write_all(line.as_bytes())?;
    if !caught {
        std::fs::write(folder.join("crash.pending"), line.trim_end())?;
    }
    Ok(())
}

/// The uncaught failure of the previous run, once: the file is removed.
pub fn take_pending(folder: &Path) -> Option<String> {
    let path = folder.join("crash.pending");
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caught_panic_is_an_error_and_logged_but_not_pending() {
        let dir = tempfile::tempdir().unwrap();
        let _ = record(dir.path(), true, "boom", "a.rs:1", "t");
        assert!(dir.path().join("crash.log").exists());
        assert_eq!(take_pending(dir.path()), None);
        let _ = record(dir.path(), false, "real", "b.rs:2", "t");
        let pending = take_pending(dir.path()).unwrap();
        assert!(pending.contains("СБОЙ") && pending.contains("real"), "{pending}");
        assert_eq!(take_pending(dir.path()), None);
        let log = std::fs::read_to_string(dir.path().join("crash.log")).unwrap();
        assert_eq!(log.lines().count(), 2);
    }

    #[test]
    fn catch_turns_a_panic_into_an_error() {
        let result: io::Result<u32> = catch("проверка", || panic!("сломалось"));
        let error = result.unwrap_err().to_string();
        assert!(error.contains("проверка") && error.contains("сломалось"), "{error}");
        assert_eq!(catch("x", || 5).unwrap(), 5);
    }

    #[test]
    fn a_poisoned_lock_still_opens() {
        let mutex = std::sync::Arc::new(Mutex::new(1));
        let held = mutex.clone();
        let _ = std::thread::spawn(move || { let _guard = held.lock().unwrap(); panic!("x"); }).join();
        assert!(mutex.is_poisoned());
        *lock(&mutex) += 1;
        assert_eq!(*lock(&mutex), 2);
    }
}
