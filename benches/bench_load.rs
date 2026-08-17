// Benchmark loading performance
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let epub_path = PathBuf::from("/home/liangxiangan/TND/我不是戏神.epub");
    
    // Test 1: Parse EPUB spine
    let start = Instant::now();
    let mut doc = epub::doc::EpubDoc::new(&epub_path)?;
    let spine_items: Vec<_> = doc.spine.iter().cloned().collect();
    let spine_time = start.elapsed();
    println!("Spine items: {}, time: {:?}", spine_items.len(), spine_time);
    
    // Test 2: Filter HTML chapters
    let start = Instant::now();
    let mut html_chapters = 0;
    for item in &spine_items {
        let resource_id = &item.idref;
        let mime = doc.get_resource_mime(resource_id).unwrap_or_default();
        if mime == "application/xhtml+xml" || mime == "text/html" {
            html_chapters += 1;
        }
    }
    let filter_time = start.elapsed();
    println!("HTML chapters: {}, filter time: {:?}", html_chapters, filter_time);
    
    // Test 3: Extract first 10 chapters content
    let start = Instant::now();
    for (i, item) in spine_items.iter().enumerate().take(10) {
        let resource_id = &item.idref;
        let mime = doc.get_resource_mime(resource_id).unwrap_or_default();
        if mime == "application/xhtml+xml" || mime == "text/html" {
            let _ = doc.get_resource_str(resource_id);
        }
    }
    let extract_time = start.elapsed();
    println!("Extract 10 chapters time: {:?}", extract_time);
    
    // Test 4: Full extraction (first 50 chapters)
    let start = Instant::now();
    let mut count = 0;
    for item in &spine_items {
        let resource_id = &item.idref;
        let mime = doc.get_resource_mime(resource_id).unwrap_or_default();
        if mime == "application/xhtml+xml" || mime == "text/html" {
            let _ = doc.get_resource_str(resource_id);
            count += 1;
            if count >= 50 { break; }
        }
    }
    let extract50_time = start.elapsed();
    println!("Extract 50 chapters time: {:?}", extract50_time);
    
    // Estimate full book
    let estimated = extract50_time * (html_chapters as u32 / 50).max(1);
    println!("Estimated full extraction: {:?}", estimated);
    
    Ok(())
}