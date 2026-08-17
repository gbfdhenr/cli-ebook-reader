use anyhow::Result;
use std::path::PathBuf;

mod cache;
mod common;
mod file_browser;
mod reader;
mod terminal;

fn main() -> Result<()> {
    eprintln!("DEBUG: main started");
    // Handle --version flag before initializing terminal
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Get EPUB path from command line argument
    let epub_path = std::env::args().nth(1).map(PathBuf::from);

    eprintln!("DEBUG: before terminal::init");
    // Initialize terminal once for the entire program
    terminal::init()?;
    eprintln!("DEBUG: after terminal::init");

    // Setup resize handler
    common::events::init_resize_handler();

    // Ensure terminal is restored on any exit path
    let result = run_app(epub_path);

    terminal::restore();

    // Print any error after terminal is restored
    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app(epub_path: Option<PathBuf>) -> Result<()> {
    eprintln!("DEBUG: run_app started, epub_path: {:?}", epub_path);
    // Step 1: Get EPUB path (from arg or file browser)
    let epub_path = match epub_path {
        Some(path) => path,
        None => match file_browser::run_file_browser(PathBuf::from("."))? {
            Some(path) => path,
            None => {
                println!("No file selected");
                return Ok(());
            }
        }
    };

    eprintln!("DEBUG: creating reader");
    // Step 2: Launch EPUB reader (terminal already initialized)
    let mut reader = reader::ReaderState::new(epub_path)?;
    eprintln!("DEBUG: reader created, calling run()");
    reader.run()
}