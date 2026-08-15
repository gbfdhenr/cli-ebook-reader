use anyhow::Result;
use crossterm::event::{Event, KeyEvent, KeyEventKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Shared flag for terminal resize detection
static RESIZE_FLAG: AtomicBool = AtomicBool::new(false);

/// Initialize SIGWINCH handler (call once at startup)
pub fn init_resize_handler() {
    use signal_hook::consts::SIGWINCH;
    use signal_hook::iterator::Signals;

    let mut signals = Signals::new([SIGWINCH]).expect("Failed to register SIGWINCH handler");
    std::thread::spawn(move || {
        for _ in signals.forever() {
            RESIZE_FLAG.store(true, Ordering::Relaxed);
        }
    });
}

/// Check if terminal was resized since last check
pub fn check_resize() -> bool {
    RESIZE_FLAG.swap(false, Ordering::Relaxed)
}

/// Read a key event with timeout (milliseconds)
/// Returns None on timeout, Some(KeyEvent) on key press
pub fn read_key(timeout_ms: u64) -> Result<Option<KeyEvent>> {
    use crossterm::event::{poll, read};

    if poll(Duration::from_millis(timeout_ms))? {
        match read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => Ok(Some(key)),
            _ => Ok(None),
        }
    } else {
        Ok(None)
    }
}

