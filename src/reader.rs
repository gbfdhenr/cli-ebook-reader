use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use epub::doc::EpubDoc;
use html2text::from_read;
use ratatui::{Terminal, Frame};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::style::{Style, Color, Modifier};
use ratatui::text::{Line, Span, Text};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unicode_width::{UnicodeWidthStr, UnicodeWidthChar};

use crate::cache::{CacheManager, global_cache};
use crate::common::events::check_resize;

#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    /// 缓存的行内容（每行一个 String），已按视口宽度换行
    pub lines: Vec<String>,
}

/// 底部状态栏的可点击区域
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BottomBarAction {
    PrevChapter,    // [ < ]
    Exit,           // [退出]
    Menu,           // 中间菜单区
    NextChapter,    // [ > ]
    None,
}

/// 退出确认状态
#[derive(Debug, Clone, Copy, PartialEq)]
enum ExitConfirmState {
    None,           // 正常状态
    CtrlCPressed,   // 第一次按下 Ctrl+C，等待 3 秒内第二次
    ExitClicked,    // 点击了退出按钮，等待 y/其他键
}

/// 加载进度信息（从后台线程发送）
#[derive(Debug, Clone)]
struct LoadingProgress {
    progress: f32,           // 0.0 - 1.0
    current_chapter: usize,
    total_chapters: usize,
    stage: LoadingStage,
}

/// 读取器状态
enum LoadingState {
    Idle,
    Loading(LoadingProgress),
    Loaded,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LoadingStage {
    ParsingSpine,
    ExtractingChapters,
    BuildingCache,
    Done,
}

pub struct ReaderState {
    chapters: Vec<Chapter>,
    current_chapter: usize,
    /// 当前显示的起始行索引
    line_offset: usize,
    /// 可显示的行数
    viewport_lines: usize,
    /// 当前视口宽度
    viewport_width: u16,
    terminal_height: u16,
    terminal_width: u16,
    /// 书名
    book_title: String,
    /// 底部栏菜单是否展开
    menu_open: bool,
    /// 退出确认状态
    exit_confirm: ExitConfirmState,
    /// 第一次 Ctrl+C 的时间
    first_ctrl_c_time: Option<Instant>,
    /// 缓存管理器
    cache: Arc<Mutex<CacheManager>>,
    /// EPUB 文件路径
    epub_path: PathBuf,
    /// 加载状态
    loading_state: LoadingState,
    /// 加载线程句柄
    load_handle: Option<thread::JoinHandle<Result<Vec<Chapter>>>>,
    /// 进度接收器（从后台线程接收进度更新）
    progress_rx: Option<mpsc::Receiver<LoadingProgress>>,
    /// 是否首次加载（用于显示文件名还是书名）
    first_load: bool,
}

impl ReaderState {
    pub fn new(epub_path: PathBuf) -> Result<Self> {
        let cache = global_cache();
        let first_load = true;
        let book_title = "Unknown Book".to_string(); // 临时，加载后更新

        Ok(ReaderState {
            chapters: Vec::new(),
            current_chapter: 0,
            line_offset: 0,
            viewport_lines: 0,
            viewport_width: 80,
            terminal_height: 24,
            terminal_width: 80,
            book_title,
            menu_open: false,
            exit_confirm: ExitConfirmState::None,
            first_ctrl_c_time: None,
            cache,
            epub_path,
            loading_state: LoadingState::Idle,
            load_handle: None,
            progress_rx: None,
            first_load,
        })
    }

    /// 启动异步加载
    fn start_loading(&mut self) {
        let epub_path = self.epub_path.clone();
        let cache = self.cache.clone();
        let first_load = self.first_load;

        // 创建进度通道
        let (progress_tx, progress_rx) = mpsc::channel();
        self.progress_rx = Some(progress_rx);

        // 初始加载状态 - 显示 1% 避免空进度条
        self.loading_state = LoadingState::Loading(LoadingProgress {
            progress: 0.01,
            current_chapter: 0,
            total_chapters: 0,
            stage: LoadingStage::ParsingSpine,
        });

        let handle = thread::spawn(move || {
            // 发送进度更新的辅助函数
            let send_progress = |tx: &mpsc::Sender<LoadingProgress>, progress: f32, current: usize, total: usize, stage: LoadingStage| {
                let _ = tx.send(LoadingProgress { progress, current_chapter: current, total_chapters: total, stage });
            };

            // 启动时清理临时文件
            if let Ok(cache_guard) = cache.lock() {
                let _ = cache_guard.cleanup_temp_files();
            }

            // 尝试从缓存加载
            let mut cache_guard = cache.lock().unwrap();
            if let Ok(Some(meta)) = cache_guard.load_meta(&epub_path) {
                // 有有效缓存，从 mmap 读取所有章节（按当前视口宽度换行）
                let total = meta.chapters.len();
                let mut chapters = Vec::new();
                for (idx, cm) in meta.chapters.iter().enumerate() {
                    // 使用 80 作为默认宽度，后续会在 UI 中重新换行
                    if let Ok(Some(lines)) = cache_guard.read_chapter(idx, 80) {
                        chapters.push(Chapter { title: cm.title.clone(), lines });
                    }
                    // 发送缓存加载进度
                    send_progress(&progress_tx, (idx + 1) as f32 / total.max(1) as f32, idx + 1, total, LoadingStage::BuildingCache);
                }
                cache_guard.update_hot_cache(0, 80);
                send_progress(&progress_tx, 1.0, total, total, LoadingStage::Done);
                return Ok(chapters);
            }
            drop(cache_guard);

            // 无缓存，解析 EPUB 并构建缓存
            Self::parse_and_cache(epub_path, cache, first_load, progress_tx)
        });

        self.load_handle = Some(handle);
    }

    /// 解析 EPUB 并构建缓存（在后台线程运行）
    fn parse_and_cache(
        epub_path: PathBuf,
        cache: Arc<Mutex<CacheManager>>,
        _first_load: bool,
        progress_tx: mpsc::Sender<LoadingProgress>,
    ) -> Result<Vec<Chapter>> {
        let send_progress = |tx: &mpsc::Sender<LoadingProgress>, progress: f32, current: usize, total: usize, stage: LoadingStage| {
            let _ = tx.send(LoadingProgress { progress, current_chapter: current, total_chapters: total, stage });
        };

        // 立即发送初始进度，避免大文件打开时长时间无反馈
        send_progress(&progress_tx, 0.01, 0, 0, LoadingStage::ParsingSpine);

        let mut doc = EpubDoc::new(&epub_path).context("open epub")?;
        let book_title = doc.get_title().unwrap_or_else(|| "Unknown Book".to_string());

        // EPUB 文档已打开，发送进度
        send_progress(&progress_tx, 0.05, 0, 0, LoadingStage::ParsingSpine);
        let spine_items: Vec<_> = doc.spine.iter().cloned().collect();
        let mut chapter_infos = Vec::new();

        for spine_item in &spine_items {
            let resource_id = &spine_item.idref;
            if let Some((_, mime)) = doc.get_resource_str(resource_id) {
                if mime == "application/xhtml+xml" || mime == "text/html" {
                    let title = Self::find_chapter_title_static(&doc, resource_id)
                        .unwrap_or_else(|| format!("Chapter {}", chapter_infos.len() + 1));
                    // 检查是否为目录章节（通过标题或内容特征判断）
                    let is_toc = Self::is_toc_chapter(&mut doc, resource_id, &title);
                    chapter_infos.push((resource_id.to_string(), title, is_toc));
                }
            }
        }

        let total_chapters = chapter_infos.len();
        send_progress(&progress_tx, 0.15, 0, total_chapters, LoadingStage::ExtractingChapters);

        // 开始构建缓存
        let mut builder = {
            let mut cache_guard = cache.lock().unwrap();
            cache_guard.start_build(&epub_path)?
        };
        builder.set_book_title(book_title.clone());

        // 阶段 2：提取章节内容并写入缓存
        let mut chapters = Vec::new();
        for (idx, (resource_id, title, is_toc)) in chapter_infos.iter().enumerate() {
            let progress = 0.15 + 0.7 * (idx as f32 / total_chapters.max(1) as f32);
            send_progress(&progress_tx, progress, idx + 1, total_chapters, LoadingStage::ExtractingChapters);

            if let Some((content, _)) = doc.get_resource_str(&resource_id) {
                // 预处理 HTML：移除图片标签和其他可能导致 html2text 卡顿的元素
                let cleaned_html = Self::clean_html_for_text_extraction(&content);
                let mut plain_text = Self::html_to_text_with_timeout(&cleaned_html, 80);
                
                // 如果是目录章节，尝试解析其中的 markdown 链接
                if *is_toc {
                    plain_text = Self::process_toc_chapter(&plain_text, &doc, &chapter_infos);
                }
                
                builder.add_chapter(idx, title.clone(), &plain_text)?;
                chapters.push(Chapter {
                    title: title.clone(),
                    lines: plain_text.lines().map(|s| s.to_string()).collect()
                });
            }
        }

        // 阶段 3：完成缓存构建
        send_progress(&progress_tx, 0.9, total_chapters, total_chapters, LoadingStage::BuildingCache);
        {
            let mut cache_guard = cache.lock().unwrap();
            cache_guard.finish_build(builder)?;
        }

        // 初始化热缓存
        send_progress(&progress_tx, 0.95, total_chapters, total_chapters, LoadingStage::BuildingCache);
        {
            let cache_guard = cache.lock().unwrap();
            cache_guard.update_hot_cache(0, 80);
        }

        send_progress(&progress_tx, 1.0, total_chapters, total_chapters, LoadingStage::Done);
        Ok(chapters)
    }

    fn find_chapter_title_static(doc: &EpubDoc<impl io::Read + io::Seek>, resource_id: &str) -> Option<String> {
        for navpoint in &doc.toc {
            if let Some(chapter_idx) = doc.resource_uri_to_chapter(&navpoint.content) {
                if chapter_idx < doc.spine.len() && doc.spine[chapter_idx].idref == resource_id {
                    return Some(navpoint.label.clone());
                }
            }
        }
        None
    }


    /// 清理 HTML，移除图片、脚本、样式等可能导致 html2text 卡顿的元素
    fn clean_html_for_text_extraction(html: &str) -> String {
        use regex::Regex;
        let mut result = html.to_string();

        // 1. 移除 <img> 标签（处理各种属性顺序、单双引号、自闭合等）
        // 先处理有 alt 的
        let img_with_alt = Regex::new(r#"(?i)<img\b[^>]*\balt\s*=\s*(["'])([^"']*)\1[^>]*>"#).unwrap();
        result = img_with_alt.replace_all(&result, |caps: &regex::Captures| {
            let alt = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            format!("[图片: {}]", alt)
        }).to_string();

        // 再处理没有 alt 的（包括各种属性组合）
        let img_no_alt = Regex::new(r#"(?i)<img\b[^>]*>"#).unwrap();
        result = img_no_alt.replace_all(&result, "[图片]").to_string();

        // 2. 移除 <script> 标签及其内容（包括跨行）
        let script_re = Regex::new(r"(?si)<script\b[^>]*>.*?</script>").unwrap();
        result = script_re.replace_all(&result, "").to_string();

        // 3. 移除 <style> 标签及其内容
        let style_re = Regex::new(r"(?si)<style\b[^>]*>.*?</style>").unwrap();
        result = style_re.replace_all(&result, "").to_string();

        // 4. 移除 <noscript> 标签及其内容
        let noscript_re = Regex::new(r"(?si)<noscript\b[^>]*>.*?</noscript>").unwrap();
        result = noscript_re.replace_all(&result, "").to_string();

        // 5. 移除 HTML 注释
        let comment_re = Regex::new(r"(?s)<!--.*?-->").unwrap();
        result = comment_re.replace_all(&result, "").to_string();

        // 6. 移除 SVG 标签及其内容（常包含大量路径数据）
        let svg_re = Regex::new(r"(?si)<svg\b[^>]*>.*?</svg>").unwrap();
        result = svg_re.replace_all(&result, "[SVG图片]").to_string();

        // 7. 移除 canvas 标签
        let canvas_re = Regex::new(r"(?si)<canvas\b[^>]*>.*?</canvas>").unwrap();
        result = canvas_re.replace_all(&result, "[Canvas]").to_string();

        // 8. 简化剩余标签：保留常用排版标签，其余替换为空格
        // 保留: p, br, h1-h6, strong, b, em, i, u, span, div, blockquote, ul, ol, li, a
        let allowed_tags = ["p", "br", "h1", "h2", "h3", "h4", "h5", "h6",
                            "strong", "b", "em", "i", "u", "span", "div",
                            "blockquote", "ul", "ol", "li", "a"];
        let allowed_pattern = allowed_tags.join("|");

        // 移除不在允许列表中的标签（保留内容）
        let tag_re = Regex::new(&format!(r"(?i)</?(?!/?(?:{})\b)[a-z][a-z0-9]*\b[^>]*>", allowed_pattern)).unwrap();
        result = tag_re.replace_all(&result, " ").to_string();

        // 9. 清理多余空白
        let whitespace_re = Regex::new(r"\s+").unwrap();
        result = whitespace_re.replace_all(&result, " ").to_string();

        result.trim().to_string()
    }

    /// 带超时的 HTML 转文本，超时后回退到简单正则提取
    fn html_to_text_with_timeout(html: &str, width: usize) -> String {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let html_owned = html.to_string();
        let html_for_fallback = html_owned.clone();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let result = from_read(html_owned.as_bytes(), width);
            let _ = tx.send(result);
        });

        // 等待 5 秒，超时则回退
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(text) => text,
            Err(_) => {
                // 超时：使用简单的正则提取文本内容
                Self::simple_html_to_text(&html_for_fallback, width)
            }
        }
    }

    /// 简单的 HTML 转文本（不依赖 html2text，用作超时回退）
    fn simple_html_to_text(html: &str, width: usize) -> String {
        use regex::Regex;
        let mut result = html.to_string();

        // 替换块级标签为换行
        let block_tags = ["p", "div", "br", "h1", "h2", "h3", "h4", "h5", "h6", 
                          "blockquote", "ul", "ol", "li", "tr", "td", "th", "table"];
        for tag in block_tags {
            let re = Regex::new(&format!(r"(?i)</?{}\b[^>]*>", tag)).unwrap();
            result = re.replace_all(&result, "\n").to_string();
        }

        // 替换内联标签为空格
        let inline_tags = ["span", "strong", "b", "em", "i", "u", "a", "font", "small", "big"];
        for tag in inline_tags {
            let re = Regex::new(&format!(r"(?i)</?{}\b[^>]*>", tag)).unwrap();
            result = re.replace_all(&result, " ").to_string();
        }

        // 移除剩余所有标签
        let tag_re = Regex::new(r"(?i)<[^>]*>").unwrap();
        result = tag_re.replace_all(&result, " ").to_string();

                // 处理 HTML 实体
        result = result.replace("&nbsp;", " ")
            .replace("<", "<")
            .replace(">", ">")
            .replace("&", "&")
            .replace(""", "\"")
            .replace("&apos;", "'")
            .replace("&mdash;", "—")
            .replace("&ndash;", "–")
            .replace("&hellip;", "…");

        // 清理多余空白
        let ws_re = Regex::new(r"\s+").unwrap();
        result = ws_re.replace_all(&result, " ").to_string();

        // 按宽度换行
        Self::wrap_text(&result, width).join("\n")
    }

    /// 判断是否为目录章节（TOC）
    fn is_toc_chapter(doc: &mut EpubDoc<impl io::Read + io::Seek>, resource_id: &str, title: &str) -> bool {
        // 1. 通过标题判断：包含 "目录"、"Table of Contents"、"Contents" 等关键词
        let title_lower = title.to_lowercase();
        if title_lower.contains("目录") 
            || title_lower.contains("table of contents") 
            || title_lower.contains("contents")
            || title_lower.contains("toc") {
            return true;
        }

        // 2. 通过内容判断：如果章节主要包含指向其他章节的链接
        if let Some((content, _)) = doc.get_resource_str(resource_id) {
            // 简单检查：内容中是否包含大量指向 spine 内部的链接
            let link_count = content.matches("<a href=").count();
            let text_len = content.len();
            // 如果链接密度很高，可能是目录
            if link_count > 5 && text_len < 5000 {
                return true;
            }
        }
        false
    }

    /// 处理目录章节：解析 markdown 格式的链接，转换为可读格式
    fn process_toc_chapter(text: &str, _doc: &EpubDoc<impl io::Read + io::Seek>, chapter_infos: &[(String, String, bool)]) -> String {
        use regex::Regex;
        let mut result = text.to_string();
        
        // 正则匹配 markdown 格式的链接：[链接文本](链接地址)
        let md_link_re = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
        result = md_link_re.replace_all(&result, |caps: &regex::Captures| {
            let link_text = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let link_url = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            
            // 尝试匹配到对应的章节
            for (_, chapter_title, _) in chapter_infos {
                if chapter_title.contains(link_text) || link_text.contains(chapter_title) {
                    return format!("► {} (第 {} 章)", link_text, chapter_title);
                }
            }
            
            // 如果没匹配到章节，保留原文本但标记为链接
            format!("► {} → {}", link_text, link_url)
        }).to_string();
        
        // 也处理 HTML 格式的链接
        let html_link_re = Regex::new(r#"<a[^>]*href\s*=\s*["']([^"']*)["'][^>]*>([^<]*)</a>"#).unwrap();
        result = html_link_re.replace_all(&result, |caps: &regex::Captures| {
            let link_url = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let link_text = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            
            for (_, chapter_title, _) in chapter_infos {
                if chapter_title.contains(link_text) || link_text.contains(chapter_title) {
                    return format!("► {} (第 {} 章)", link_text, chapter_title);
                }
            }
            format!("► {} → {}", link_text, link_url)
        }).to_string();
        
        result
    }

    /// 检查加载进度（包括从后台线程接收进度更新）
    fn check_loading(&mut self) {
        // 先处理进度通道的更新
        if let Some(rx) = &self.progress_rx {
            while let Ok(progress) = rx.try_recv() {
                self.loading_state = LoadingState::Loading(progress);
            }
        }

        // 再检查线程是否完成
        if let Some(handle) = self.load_handle.take() {
            if handle.is_finished() {
                match handle.join() {
                    Ok(Ok(chapters)) => {
                        if !chapters.is_empty() {
                            // 从缓存获取真实书名和阅读进度
                            if let Ok(guard) = self.cache.lock() {
                                if let Some(meta) = guard.get_meta() {
                                    self.book_title = meta.book_title.clone();
                                    self.current_chapter = meta.last_read_chapter.min(chapters.len().saturating_sub(1));
                                    self.line_offset = meta.last_line_offset;
                                }
                            }
                            // 如果缓存没有 meta（不应发生），回退到第一个章节标题
                            if self.book_title == "Unknown Book" {
                                self.book_title = chapters[0].title.clone();
                            }
                        }
                        self.chapters = chapters;
                        self.loading_state = LoadingState::Loaded;
                        self.first_load = false;
                        // 加载完成后钳制偏移
                        self.clamp_offset();
                    }
                    Ok(Err(e)) => {
                        self.loading_state = LoadingState::Failed(e.to_string());
                    }
                    Err(_) => {
                        self.loading_state = LoadingState::Failed("加载线程异常终止".to_string());
                    }
                }
            } else {
                // 还在加载中，放回 handle
                self.load_handle = Some(handle);
            }
        }
    }

    /// 逐字符换行（支持中文无空格、混合文本）
    fn wrap_text(text: &str, width: usize) -> Vec<String> {
        if width == 0 { return vec![text.to_string()]; }
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            if paragraph.is_empty() { lines.push(String::new()); continue; }
            let mut current_line = String::new();
            let mut current_width = 0;
            for ch in paragraph.chars() {
                let ch_width = ch.width().unwrap_or(1);
                if current_width + ch_width <= width {
                    current_line.push(ch);
                    current_width += ch_width;
                } else {
                    lines.push(current_line);
                    current_line = ch.to_string();
                    current_width = ch_width;
                }
            }
            if !current_line.is_empty() { lines.push(current_line); }
        }
        lines
    }

    fn current_time_str() -> String {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let hours = (now / 3600) % 24;
        let minutes = (now / 60) % 60;
        format!("{:02}:{:02}", hours, minutes)
    }

    fn reading_progress(&self) -> u8 {
        if self.chapters.is_empty() { return 0; }
        let chapter = &self.chapters[self.current_chapter];
        let total_lines = chapter.lines.len();
        if total_lines == 0 {
            return ((self.current_chapter as f32 / self.chapters.len() as f32) * 100.0) as u8;
        }
        let line_progress = self.line_offset as f32 / total_lines as f32;
        let total_progress = (self.current_chapter as f32 + line_progress) / self.chapters.len() as f32 * 100.0;
        total_progress.min(100.0) as u8
    }

    pub fn run(&mut self) -> Result<()> {
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        terminal.clear()?;
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

        // 启动加载
        self.start_loading();

        let result = (|| {
            loop {
                // 检查 Ctrl+C 超时（3秒）
                if self.exit_confirm == ExitConfirmState::CtrlCPressed {
                    if let Some(t) = self.first_ctrl_c_time {
                        if t.elapsed() > Duration::from_secs(3) {
                            self.exit_confirm = ExitConfirmState::None;
                            self.first_ctrl_c_time = None;
                        }
                    }
                }

                // 检查加载进度
                self.check_loading();

                if check_resize() { terminal.autoresize()?; }

                terminal.draw(|frame| {
                    let area = frame.area();
                    self.terminal_height = area.height;
                    self.terminal_width = area.width;
                    let new_viewport_lines = area.height.saturating_sub(1 + 2 + 2 + 1) as usize;
                    let new_viewport_width = area.width.saturating_sub(4);
                    if new_viewport_width != self.viewport_width {
                        self.viewport_width = new_viewport_width;
                        // 重新换行当前章
                        if !self.chapters.is_empty() && self.current_chapter < self.chapters.len() {
                            let chapter = &mut self.chapters[self.current_chapter];
                            let text = chapter.lines.join("\n");
                            chapter.lines = Self::wrap_text(&text, self.viewport_width as usize);
                        }
                        self.clamp_offset();
                    }
                    self.viewport_lines = new_viewport_lines;
                    self.draw(frame, area);
                })?;

                if let Some(event) = self.read_event(100)? {
                    match event {
                        Event::Key(key) => {
                            if self.handle_key_with_exit(key)? {
                                return Ok(());
                            }
                        }
                        Event::Mouse(mouse) => {
                            if self.handle_mouse_with_exit(mouse)? {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
            }
            #[allow(unreachable_code)]
            Ok(())
        })();

        // 退出前保存进度
        if let LoadingState::Loaded = &self.loading_state {
            if let Ok(mut cache) = self.cache.lock() {
                cache.update_progress(self.current_chapter, self.line_offset).ok();
            }
        }

        crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
        result
    }

    fn read_event(&self, timeout_ms: u64) -> Result<Option<Event>> {
        use crossterm::event::{poll, read};
        use std::time::Duration;
        if poll(Duration::from_millis(timeout_ms))? {
            match read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => Ok(Some(Event::Key(key))),
                Event::Mouse(mouse) => Ok(Some(Event::Mouse(mouse))),
                _ => Ok(None),
            }
        } else { Ok(None) }
    }

    /// 处理键盘事件，返回 true 表示应该退出程序
    fn handle_key_with_exit(&mut self, key: KeyEvent) -> Result<bool> {
        // 加载中忽略大部分按键，只允许退出
        if matches!(self.loading_state, LoadingState::Loading(_)) {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(true); // 强制退出
                }
                KeyCode::Char('q') | KeyCode::Esc => {
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }

        if matches!(self.loading_state, LoadingState::Failed(_)) {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL)) {
                return Ok(true);
            }
            return Ok(false);
        }

        // 处理退出确认状态下的按键
        match self.exit_confirm {
            ExitConfirmState::CtrlCPressed => {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(true);
                }
                self.exit_confirm = ExitConfirmState::None;
                self.first_ctrl_c_time = None;
                return Ok(false);
            }
            ExitConfirmState::ExitClicked => {
                if key.code == KeyCode::Char('y') || key.code == KeyCode::Char('Y') {
                    return Ok(true);
                }
                self.exit_confirm = ExitConfirmState::None;
                return Ok(false);
            }
            ExitConfirmState::None => {}
        }

        // 正常按键处理
        if self.chapters.is_empty() { return Ok(false); }

        let chapter = &self.chapters[self.current_chapter];
        let total_lines = chapter.lines.len();
        let max_offset = total_lines.saturating_sub(self.viewport_lines);

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.exit_confirm = ExitConfirmState::CtrlCPressed;
                self.first_ctrl_c_time = Some(Instant::now());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.line_offset = self.line_offset.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.line_offset < max_offset { self.line_offset += 1; }
            }
            KeyCode::PageUp => {
                self.line_offset = self.line_offset.saturating_sub(self.viewport_lines);
            }
            KeyCode::PageDown => {
                self.line_offset = (self.line_offset + self.viewport_lines).min(max_offset);
            }
            KeyCode::Home => { self.line_offset = 0; }
            KeyCode::End => { self.line_offset = max_offset; }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('n') => {
                if self.current_chapter + 1 < self.chapters.len() {
                    self.current_chapter += 1;
                    self.line_offset = 0;
                    // 更新热缓存，并重新按当前宽度换行当前章
                    if let Ok(cache) = self.cache.lock() {
                        cache.update_hot_cache(self.current_chapter, self.viewport_width);
                        // 从热缓存读取当前章（已按 viewport_width 换行）
                        if let Some(cached) = cache.get_hot_chapter(self.current_chapter, self.viewport_width) {
                            self.chapters[self.current_chapter].lines = cached.lines;
                        }
                    }
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('p') => {
                if self.current_chapter > 0 {
                    self.current_chapter -= 1;
                    self.line_offset = 0;
                    if let Ok(cache) = self.cache.lock() {
                        cache.update_hot_cache(self.current_chapter, self.viewport_width);
                        if let Some(cached) = cache.get_hot_chapter(self.current_chapter, self.viewport_width) {
                            self.chapters[self.current_chapter].lines = cached.lines;
                        }
                    }
                }
            }
            KeyCode::Char('m') => {
                self.menu_open = !self.menu_open;
            }
            _ => {}
        }

        self.clamp_offset();
        Ok(false)
    }

    /// 处理鼠标事件，返回 true 表示应该退出程序
    fn handle_mouse_with_exit(&mut self, mouse: MouseEvent) -> Result<bool> {
        if matches!(self.loading_state, LoadingState::Loading(_) | LoadingState::Failed(_)) {
            return Ok(false);
        }

        if mouse.kind != MouseEventKind::Down(crossterm::event::MouseButton::Left) {
            return Ok(false);
        }

        let action = self.hit_test_bottom_bar(mouse.column, mouse.row);
        match action {
            BottomBarAction::PrevChapter => {
                if self.current_chapter > 0 {
                    self.current_chapter -= 1; self.line_offset = 0;
                    if let Ok(cache) = self.cache.lock() {
                        cache.update_hot_cache(self.current_chapter, self.viewport_width);
                        if let Some(cached) = cache.get_hot_chapter(self.current_chapter, self.viewport_width) {
                            self.chapters[self.current_chapter].lines = cached.lines;
                        }
                    }
                }
            }
            BottomBarAction::NextChapter => {
                if self.current_chapter + 1 < self.chapters.len() {
                    self.current_chapter += 1; self.line_offset = 0;
                    if let Ok(cache) = self.cache.lock() {
                        cache.update_hot_cache(self.current_chapter, self.viewport_width);
                        if let Some(cached) = cache.get_hot_chapter(self.current_chapter, self.viewport_width) {
                            self.chapters[self.current_chapter].lines = cached.lines;
                        }
                    }
                }
            }
            BottomBarAction::Menu => {
                self.menu_open = !self.menu_open;
            }
            BottomBarAction::Exit => {
                self.exit_confirm = ExitConfirmState::ExitClicked;
            }
            BottomBarAction::None => {}
        }
        Ok(false)
    }

    fn hit_test_bottom_bar(&self, x: u16, y: u16) -> BottomBarAction {
        if y != self.terminal_height.saturating_sub(1) {
            return BottomBarAction::None;
        }

        let width = self.terminal_width as usize;
        let exit_btn = "[退出]";
        let left_btn = "[ < ]";
        let right_btn = "[ > ]";
        let progress_pct = self.reading_progress();
        let progress_str = format!(" {}% ", progress_pct);

        let exit_width = exit_btn.width();
        let left_width = left_btn.width();
        let right_width = right_btn.width();
        let progress_width = progress_str.width();

        // 布局顺序：[退出] [ < ] [菜单区] [ > ] [进度%]
        let exit_end = exit_width;
        let left_end = exit_end + left_width;
        // 右按钮和进度在最右侧
        let progress_start = width.saturating_sub(progress_width);
        let right_start = progress_start.saturating_sub(right_width);
        let menu_start = left_end;
        let menu_end = right_start;

        let x_pos = x as usize;

        if x_pos < exit_end {
            BottomBarAction::Exit
        } else if x_pos < left_end {
            BottomBarAction::PrevChapter
        } else if x_pos >= right_start && x_pos < right_start + right_width {
            BottomBarAction::NextChapter
        } else if x_pos >= progress_start && x_pos < width {
            BottomBarAction::None // 进度条区域不响应点击
        } else if x_pos >= menu_start && x_pos < menu_end {
            BottomBarAction::Menu
        } else {
            BottomBarAction::None
        }
    }

    fn clamp_offset(&mut self) {
        if self.chapters.is_empty() { self.line_offset = 0; return; }
        let chapter = &self.chapters[self.current_chapter];
        let total_lines = chapter.lines.len();
        let max_offset = total_lines.saturating_sub(self.viewport_lines);
        if self.line_offset > max_offset { self.line_offset = max_offset; }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) {
        // 加载中状态
        if matches!(self.loading_state, LoadingState::Loading(_)) {
            self.draw_loading(frame, area);
            return;
        }

        // 失败状态
        if let LoadingState::Failed(err) = &self.loading_state {
            self.draw_error(frame, area, err);
            return;
        }

        // 空章节
        if self.chapters.is_empty() {
            self.draw_empty(frame, area);
            return;
        }

        let chapter = &self.chapters[self.current_chapter];

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),    // 顶栏
                Constraint::Length(2),    // 标题栏
                Constraint::Min(1),       // 内容区
                Constraint::Length(1),    // 底栏
            ])
            .split(area);

        self.draw_top_bar(frame, chunks[0]);
        self.draw_chapter_header(frame, chunks[1], chapter);
        self.draw_content(frame, chunks[2], chapter);
        self.draw_bottom_bar(frame, chunks[3]);

        // 绘制退出确认覆盖层
        if self.exit_confirm != ExitConfirmState::None {
            self.draw_exit_confirm_overlay(frame, area);
        }
    }

    /// 绘制加载进度界面
    fn draw_loading(&self, frame: &mut Frame, area: Rect) {
        let progress_info = match &self.loading_state {
            LoadingState::Loading(p) => p,
            _ => return,
        };

        let LoadingProgress { progress, current_chapter, total_chapters, stage } = progress_info;

        // 显示名称：首次加载显示文件名，后续显示书名
        let display_name = if self.first_load {
            self.epub_path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
        } else {
            &self.book_title
        };

        let stage_text = match stage {
            LoadingStage::ParsingSpine => "解析目录结构",
            LoadingStage::ExtractingChapters => "提取章节内容",
            LoadingStage::BuildingCache => "构建缓存索引",
            LoadingStage::Done => "完成",
        };

        let percent = (progress * 100.0) as u32;
        
        // popup 宽度固定为 80，确保能容纳完整进度信息
        let popup_width = 80u16.min(area.width);
        let inner_width = popup_width.saturating_sub(4) as usize; // 减去边框和内边距
        
        // 进度条宽度基于 popup 内部宽度
        let bar_width = inner_width.saturating_sub(30).min(40).max(10);
        let filled = ((bar_width as f32 * progress) as usize).min(bar_width);
        let empty = bar_width - filled;

        // total_chapters 是 &usize（从引用解构），需要解引用
        let total_ch = *total_chapters;

        let loading_text = format!(
            " 正在加载: {}  [{}{}] {}%  ({}/{})  {} ",
            display_name,
            "█".repeat(filled),
            "░".repeat(empty),
            percent,
            current_chapter + 1,
            total_ch.max(1),
            stage_text
        );

        let loading_paragraph = Paragraph::new(loading_text)
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .alignment(ratatui::layout::Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title(" 加载中 "));

        // 居中显示，高度改为 3 行更紧凑
        let popup_height = 3;
        let x = (area.width.saturating_sub(popup_width)) / 2;
        let y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        frame.render_widget(loading_paragraph, popup_area);
    }

    fn draw_error(&self, frame: &mut Frame, area: Rect, err: &str) {
        let error_text = format!(" 加载失败: {}\n\n 按 q/Esc/Ctrl+C 退出 ", err);
        let error_paragraph = Paragraph::new(error_text)
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            .alignment(ratatui::layout::Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title(" 错误 "));

        let popup_width = area.width.min(80);
        let popup_height = 7;
        let x = (area.width.saturating_sub(popup_width)) / 2;
        let y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        frame.render_widget(error_paragraph, popup_area);
    }

    fn draw_top_bar(&self, frame: &mut Frame, area: Rect) {
        let time_str = Self::current_time_str();
        let title = &self.book_title;
        let title_width = title.width();
        let time_width = time_str.width();
        let available = area.width as usize;

        let mut left_padding = (available.saturating_sub(title_width + time_width + 2)) / 2;
        if left_padding < 1 { left_padding = 1; }

        let mut line = String::new();
        line.push_str(&" ".repeat(left_padding));
        line.push_str(title);
        let used = left_padding + title_width;
        let remaining = available.saturating_sub(used);
        if remaining > time_width + 1 {
            line.push_str(&" ".repeat(remaining - time_width - 1));
        }
        line.push_str(&time_str);

        let top_bar = Paragraph::new(line)
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
        frame.render_widget(top_bar, area);
    }

    fn draw_chapter_header(&self, frame: &mut Frame, area: Rect, chapter: &Chapter) {
        let progress = if chapter.lines.is_empty() {
            "0/0".to_string()
        } else {
            let end_line = (self.line_offset + self.viewport_lines).min(chapter.lines.len());
            format!("{}/{}", end_line, chapter.lines.len())
        };
        let header_text = format!("📖 {}  ({}/{})  Lines: {}",
            chapter.title, self.current_chapter + 1, self.chapters.len(), progress);
        let header = Paragraph::new(header_text)
            .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
        frame.render_widget(header, area);
    }

    fn draw_content(&self, frame: &mut Frame, area: Rect, chapter: &Chapter) {
        let visible_lines: Vec<Line> = chapter.lines
            .iter()
            .skip(self.line_offset)
            .take(self.viewport_lines)
            .map(|line| Line::from(Span::raw(line.as_str())))
            .collect();
        let content = Paragraph::new(Text::from(visible_lines))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)))
            .wrap(Wrap { trim: false });
        frame.render_widget(content, area);
    }

    fn draw_bottom_bar(&self, frame: &mut Frame, area: Rect) {
        let progress_pct = self.reading_progress();
        let progress_str = format!(" {}% ", progress_pct);
        let progress_width = progress_str.width();

        let exit_btn = "[退出]";
        let left_btn = "[ < ]";
        let right_btn = "[ > ]";
        let exit_width = exit_btn.width();
        let left_width = left_btn.width();
        let right_width = right_btn.width();

        let available = area.width as usize;
        // 菜单区宽度 = 总宽 - 退出 - 左按钮 - 右按钮 - 进度 - 间距
        let menu_width = available
            .saturating_sub(exit_width + left_width + right_width + progress_width + 2)
            .max(8);

        let mut parts = Vec::new();

        // [退出] - 最左侧
        let exit_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        parts.push(Span::styled(exit_btn, exit_style));

        // [ < ] - 左按钮
        let left_style = if self.current_chapter > 0 {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else { Style::default().fg(Color::DarkGray) };
        parts.push(Span::styled(left_btn, left_style));

        // 菜单区 - 中间填充
        let menu_text = if self.menu_open { " ☰ MENU " } else { " ☰ Menu " };
        let menu_style = if self.menu_open {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else { Style::default().fg(Color::DarkGray) };
        let padded_menu = format!("{:^width$}", menu_text, width = menu_width.max(menu_text.width()));
        parts.push(Span::styled(padded_menu, menu_style));

        // [ > ] - 右按钮
        let right_style = if self.current_chapter + 1 < self.chapters.len() {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else { Style::default().fg(Color::DarkGray) };
        parts.push(Span::styled(right_btn, right_style));

        // 进度% - 最右侧
        parts.push(Span::styled(progress_str, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));

        let bottom_bar = Paragraph::new(Line::from(parts))
            .style(Style::default().bg(Color::Black));
        frame.render_widget(bottom_bar, area);
    }

    fn draw_exit_confirm_overlay(&self, frame: &mut Frame, area: Rect) {
        let (msg, style) = match self.exit_confirm {
            ExitConfirmState::CtrlCPressed => (
                " 再按一次 Ctrl+C 退出，或按任意键取消 ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            ),
            ExitConfirmState::ExitClicked => (
                " 确定退出？按 y 确认，按其他键取消 ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            ),
            _ => return,
        };

        let popup_width = (msg.width() + 4).min(area.width.saturating_sub(2) as usize) as u16;
        let popup_height = 3;
        let x = (area.width.saturating_sub(popup_width)) / 2;
        let y = (area.height.saturating_sub(popup_height)) / 2;

        let popup_area = Rect::new(x, y, popup_width, popup_height);

        // 半透明遮罩效果：仅在弹窗区域绘制深色背景
        let overlay = Block::default().style(Style::default().bg(Color::Rgb(0, 0, 0)));
        frame.render_widget(overlay, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(" 退出确认 ")
            .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));

        let text = Paragraph::new(msg)
            .style(style)
            .block(block)
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true });

        frame.render_widget(text, popup_area);
    }

    fn draw_empty(&self, frame: &mut Frame, area: Rect) {
        let text = Paragraph::new("No readable chapters found in this EPUB")
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"))
            .wrap(Wrap { trim: true });
        frame.render_widget(text, area);
    }
}