// Benchmark full reader parse_and_cache flow
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use epub::doc::EpubDoc;
use html2text::from_read;
use regex::Regex;

fn clean_html_for_text_extraction(html: &str) -> String {
    let mut result = html.to_string();
    let img_with_alt = Regex::new(r#"(?i)<img\b[^>]*\balt\s*=\s*(["'])([^"']*)\1[^>]*>"#).unwrap();
    result = img_with_alt.replace_all(&result, |caps: &regex::Captures| {
        let alt = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        format!("[图片: {}]", alt)
    }).to_string();
    let img_no_alt = Regex::new(r#"(?i)<img\b[^>]*>"#).unwrap();
    result = img_no_alt.replace_all(&result, "[图片]").to_string();
    let script_re = Regex::new(r"(?si)<script\b[^>]*>.*?</script>").unwrap();
    result = script_re.replace_all(&result, "").to_string();
    let style_re = Regex::new(r"(?si)<style\b[^>]*>.*?</style>").unwrap();
    result = style_re.replace_all(&result, "").to_string();
    let noscript_re = Regex::new(r"(?si)<noscript\b[^>]*>.*?</noscript>").unwrap();
    result = noscript_re.replace_all(&result, "").to_string();
    let comment_re = Regex::new(r"(?s)<!--.*?-->").unwrap();
    result = comment_re.replace_all(&result, "").to_string();
    let svg_re = Regex::new(r"(?si)<svg\b[^>]*>.*?</svg>").unwrap();
    result = svg_re.replace_all(&result, "[SVG图片]").to_string();
    let canvas_re = Regex::new(r"(?si)<canvas\b[^>]*>.*?</canvas>").unwrap();
    result = canvas_re.replace_all(&result, "[Canvas]").to_string();
    let allowed_tags = ["p", "br", "h1", "h2", "h3", "h4", "h5", "h6",
                        "strong", "b", "em", "i", "u", "span", "div",
                        "blockquote", "ul", "ol", "li", "a"];
    let allowed_pattern = allowed_tags.join("|");
    let tag_re = Regex::new(&format!(r"(?i)</?(?!/?(?:{})\b)[a-z][a-z0-9]*\b[^>]*>", allowed_pattern)).unwrap();
    result = tag_re.replace_all(&result, " ").to_string();
    let whitespace_re = Regex::new(r"\s+").unwrap();
    result = whitespace_re.replace_all(&result, " ").to_string();
    result.trim().to_string()
}

fn find_chapter_title_static(doc: &EpubDoc<impl std::io::Read + std::io::Seek>, resource_id: &str) -> Option<String> {
    for navpoint in &doc.toc {
        if let Some(chapter_idx) = doc.resource_uri_to_chapter(&navpoint.content) {
            if chapter_idx < doc.spine.len() && doc.spine[chapter_idx].idref == resource_id {
                return Some(navpoint.label.clone());
            }
        }
    }
    None
}

fn is_toc_chapter(doc: &mut EpubDoc<impl std::io::Read + std::io::Seek>, resource_id: &str, title: &str) -> bool {
    let title_lower = title.to_lowercase();
    if title_lower.contains("目录")
        || title_lower.contains("table of contents")
        || title_lower.contains("contents")
        || title_lower.contains("toc") {
        return true;
    }
    if let Some((content, _)) = doc.get_resource_str(resource_id) {
        let link_count = content.matches("<a href=").count();
        let text_len = content.len();
        if link_count > 5 && text_len < 5000 {
            return true;
        }
    }
    false
}

fn main() -> Result<()> {
    let epub_path = PathBuf::from("/home/liangxiangan/TND/我不是戏神.epub");
    
    // Phase 1: Parse spine and collect chapter info (like parse_and_cache)
    let start = Instant::now();
    let mut doc = EpubDoc::new(&epub_path)?;
    let spine_items: Vec<_> = doc.spine.iter().cloned().collect();
    let parse_spine = start.elapsed();
    println!("Parse spine: {:?}, items: {}", parse_spine, spine_items.len());
    
    let start = Instant::now();
    let mut chapter_infos = Vec::new();
    for spine_item in &spine_items {
        let resource_id = &spine_item.idref;
        let mime = doc.get_resource_mime(resource_id).unwrap_or_default();
        if mime == "application/xhtml+xml" || mime == "text/html" {
            let title = find_chapter_title_static(&doc, resource_id)
                .unwrap_or_else(|| format!("Chapter {}", chapter_infos.len() + 1));
            let (content, _) = doc.get_resource_str(resource_id).unwrap_or((String::new(), String::new()));
            let is_toc = is_toc_chapter(&mut doc, resource_id, &title);
            chapter_infos.push((resource_id.to_string(), title, is_toc, content));
        }
    }
    let collect_info = start.elapsed();
    println!("Collect chapter info: {:?}, chapters: {}", collect_info, chapter_infos.len());
    
    // Phase 2: Extract content and convert to text (like parse_and_cache does)
    let start = Instant::now();
    let mut chapters = Vec::new();
    for (idx, (resource_id, title, is_toc, _content)) in chapter_infos.iter().enumerate() {
        if let Some((content, _)) = doc.get_resource_str(&resource_id) {
            let mut plain_text = from_read(content.as_bytes(), 80);
            if *is_toc {
                // process_toc_chapter - skip for benchmark
            }
            chapters.push((title.clone(), plain_text));
        }
        if idx % 200 == 0 && idx > 0 {
            println!("  Processed {} chapters...", idx);
        }
    }
    let extract_text = start.elapsed();
    println!("Extract & convert text: {:?}, chapters: {}", extract_text, chapters.len());
    
    // Total
    let total = parse_spine + collect_info + extract_text;
    println!("\n=== TOTAL: {:?} ===", total);
    
    Ok(())
}