use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use crossterm::cursor::{Show, Hide};
use std::io::{self, stdout};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize terminal: raw mode + alternate screen + hidden cursor
/// Returns error if already initialized or initialization fails
pub fn init() -> io::Result<()> {
    eprintln!("DEBUG: terminal::init start");
    // Atomic check-and-set to prevent double initialization
    if INITIALIZED.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Terminal already initialized",
        ));
    }
    eprintln!("DEBUG: atomic check passed");

    // Setup panic hook to restore terminal on panic
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal before printing panic
        let _ = restore_terminal();
        original_hook(panic_info);
    }));
    eprintln!("DEBUG: panic hook set");

    terminal::enable_raw_mode()?;
    eprintln!("DEBUG: raw mode enabled");
    execute!(stdout(), EnterAlternateScreen, Hide)?;
    eprintln!("DEBUG: execute completed");
    Ok(())
}

/// Restore terminal: disable raw mode + leave alternate screen + show cursor
/// Safe to call multiple times
pub fn restore() {
    if INITIALIZED.swap(false, Ordering::AcqRel) {
        let _ = restore_terminal();
    }
}

/// Internal restore without modifying INITIALIZED flag
fn restore_terminal() -> io::Result<()> {
    terminal::disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen, Show)?;
    Ok(())
}

