use anyhow::Result;
use std::path::PathBuf;

mod cache;
mod common;
mod file_browser;
mod reader;
mod terminal;

fn main() -> Result<()> {
    // Initialize terminal once for the entire program
    terminal::init()?;
    
    // Setup resize handler
    common::events::init_resize_handler();

    // Ensure terminal is restored on any exit path
    let result = run_app();
    
    terminal::restore();
    
    // Print any error after terminal is restored
    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app() -> Result<()> {
    // Step 1: File browser to select EPUB
    let epub_path = match file_browser::run_file_browser(PathBuf::from("."))? {
        Some(path) => path,
        None => {
            println!("No file selected");
            return Ok(());
        }
    };

    // Step 2: Launch EPUB reader (terminal already initialized)
    let mut reader = reader::ReaderState::new(epub_path)?;
    reader.run()
}