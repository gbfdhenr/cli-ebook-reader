// Benchmark loading performance with text extraction
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use html2text::from_read;

fn main() -> Result<()> {
    let epub_path = PathBuf::from("/home/liangxiangan/TND/我不是戏神.epub");
    
    let mut doc = epub::doc::EpubDoc::new(&epub_path)?;
    let spine_items: Vec<_> = doc.spine.iter().cloned().collect();
    
    // Test full extraction with html2text (like the reader does)
    let start = Instant::now();
    let mut count = 0;
    let mut total_chars = 0;
    for item in &spine_items {
        let resource_id = &item.idref;
        let mime = doc.get_resource_mime(resource_id).unwrap_or_default();
        if mime == "application/xhtml+xml" || mime == "text/html" {
            if let Some((content, _)) = doc.get_resource_str(resource_id) {
                let text = from_read(content.as_bytes(), 80);
                total_chars += text.len();
                count += 1;
            }
        }
    }
    let full_time = start.elapsed();
    println!("Full extraction with html2text: {:?}, chapters: {}, total chars: {}", full_time, count, total_chars);
    
    Ok(())
}