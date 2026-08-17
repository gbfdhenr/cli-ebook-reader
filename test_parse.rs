// Benchmark parse_and_cache directly
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use cli_ebook_reader_rust::cache::global_cache;
use cli_ebook_reader_rust::reader::ReaderState;

fn main() -> Result<()> {
    let epub_path = PathBuf::from("/home/liangxiangan/TND/我不是戏神.epub");
    
    println!("Starting parse_and_cache benchmark...");
    let start = Instant::now();
    
    let mut reader = ReaderState::new(epub_path)?;
    reader.run()?;
    
    println!("Total time: {:?}", start.elapsed());
    
    Ok(())
}