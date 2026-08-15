use anyhow::{Context, Result};
use memmap2::{MmapMut, MmapOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// 章节元数据（不包含正文，仅索引信息）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterMeta {
    pub index: usize,
    pub title: String,
    pub start_offset: u64,  // 在 mmap 文件中的字节偏移
    pub length: u32,        // 字节长度
    pub line_count: u32,    // 行数（按当前宽度换行后）
}

/// 书籍缓存元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookCacheMeta {
    pub book_id: String,           // EPUB 文件哈希
    pub file_path: String,         // 原始文件路径
    pub book_title: String,        // 书名
    pub chapter_count: usize,      // 总章节数
    pub created_at: u64,           // 创建时间戳
    pub last_read_chapter: usize,  // 上次阅读章节
    pub last_line_offset: usize,   // 上次阅读行偏移
    pub chapters: Vec<ChapterMeta>,// 章节索引
}

/// 缓存管理器
pub struct CacheManager {
    cache_dir: PathBuf,
    /// 内存映射文件（全书内容，swap-backed）
    mmap_file: Option<File>,
    mmap: Option<MmapMut>,
    /// 热章节缓存（±3 章），保存在内存 + 可选快速文件
    hot_cache: Arc<Mutex<HotChapterCache>>,
    meta: Option<BookCacheMeta>,
}

#[derive(Debug, Clone)]
struct HotChapterCache {
    chapters: HashMap<usize, CachedChapter>,
    center_chapter: Option<usize>,
}

#[derive(Debug, Clone)]
struct CachedChapter {
    title: String,
    lines: Vec<String>,  // 已按宽度换行的行
}

impl CacheManager {
    /// 获取缓存元数据（只读）
    pub fn get_meta(&self) -> Option<&BookCacheMeta> {
        self.meta.as_ref()
    }

    /// 创建缓存管理器
    pub fn new() -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".").join(".cache"))
            .join("cli-ebook-reader");
        fs::create_dir_all(&cache_dir)?;
        Ok(Self {
            cache_dir,
            mmap_file: None,
            mmap: None,
            hot_cache: Arc::new(Mutex::new(HotChapterCache {
                chapters: HashMap::new(),
                center_chapter: None,
            })),
            meta: None,
        })
    }

    /// 计算书籍 ID（基于文件路径 + 大小 + 修改时间）
    fn compute_book_id(path: &Path) -> Result<String> {
        let meta = fs::metadata(path)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(&meta.len().to_le_bytes());
        hasher.update(&meta.modified()?.elapsed()?.as_nanos().to_le_bytes());
        Ok(hasher.finalize().to_hex().to_string()[..16].to_string())
    }

    /// 获取缓存文件路径
    fn cache_paths(&self, book_id: &str) -> (PathBuf, PathBuf) {
        let base = self.cache_dir.join(book_id);
        (base.with_extension("meta.json"), base.with_extension("content.dat"))
    }

    /// 检查是否有有效缓存
    pub fn has_valid_cache(&self, epub_path: &Path) -> bool {
        if let Ok(book_id) = Self::compute_book_id(epub_path) {
            let (meta_path, content_path) = self.cache_paths(&book_id);
            meta_path.exists() && content_path.exists()
        } else {
            false
        }
    }

    /// 加载缓存元数据
    pub fn load_meta(&mut self, epub_path: &Path) -> Result<Option<BookCacheMeta>> {
        let book_id = Self::compute_book_id(epub_path)?;
        let (meta_path, content_path) = self.cache_paths(&book_id);

        if !meta_path.exists() || !content_path.exists() {
            return Ok(None);
        }

        // 读取元数据
        let meta_content = fs::read_to_string(&meta_path)?;
        let meta: BookCacheMeta = serde_json::from_str(&meta_content)?;

        // 打开并映射内容文件
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&content_path)
            .context("open content file")?;

        // 确保文件大小匹配
        let file_len = file.metadata()?.len();
        let expected_len = meta.chapters.last().map(|c| c.start_offset + c.length as u64).unwrap_or(0);
        if file_len < expected_len {
            return Ok(None); // 文件不完整
        }

        // 创建内存映射
        let mmap = unsafe { MmapOptions::new().map_mut(&file)? };

        self.mmap_file = Some(file);
        self.mmap = Some(mmap);
        self.meta = Some(meta.clone());
        Ok(Some(meta))
    }

    /// 开始构建新缓存（返回 writer 和临时路径）
    pub fn start_build(&mut self, epub_path: &Path) -> Result<CacheBuilder> {
        let book_id = Self::compute_book_id(epub_path)?;
        let (meta_path, content_path) = self.cache_paths(&book_id);

        // 创建临时文件
        let tmp_meta = meta_path.with_extension("meta.json.tmp");
        let tmp_content = content_path.with_extension("content.dat.tmp");

        let meta_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_meta)?;

        let content_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_content)?;

        Ok(CacheBuilder {
            book_id,
            book_title: String::new(),
            epub_path: epub_path.to_path_buf(),
            meta_path,
            content_path,
            tmp_meta,
            tmp_content,
            meta_file,
            content_file,
            chapters: Vec::new(),
            current_offset: 0,
        })
    }

    /// 完成缓存构建
    pub fn finish_build(&mut self, builder: CacheBuilder) -> Result<BookCacheMeta> {
        let CacheBuilder {
            book_id,
            book_title,
            epub_path,
            meta_path,
            content_path,
            tmp_meta,
            tmp_content,
            mut meta_file,
            mut content_file,
            chapters,
            ..
        } = builder;

        // 刷新内容文件
        content_file.flush()?;
        content_file.sync_all()?;

        // 写入元数据
        let meta = BookCacheMeta {
            book_id: book_id.clone(),
            file_path: epub_path.to_string_lossy().to_string(),
            book_title,
            chapter_count: chapters.len(),
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            last_read_chapter: 0,
            last_line_offset: 0,
            chapters,
        };

        let meta_json = serde_json::to_string_pretty(&meta)?;
        meta_file.write_all(meta_json.as_bytes())?;
        meta_file.flush()?;
        meta_file.sync_all()?;

        // 原子重命名
        fs::rename(&tmp_meta, &meta_path)?;
        fs::rename(&tmp_content, &content_path)?;

        // 重新打开并映射
        let file = OpenOptions::new().read(true).write(true).open(&content_path)?;
        let mmap = unsafe { MmapOptions::new().map_mut(&file)? };
        self.mmap_file = Some(file);
        self.mmap = Some(mmap);
        self.meta = Some(meta.clone());

        Ok(meta)
    }

    /// 读取章节内容（从 mmap）
    pub fn read_chapter(&self, chapter_index: usize) -> Result<Option<Vec<String>>> {
        let meta = self.meta.as_ref().ok_or_else(|| anyhow::anyhow!("cache meta not loaded"))?;
        let mmap = self.mmap.as_ref().ok_or_else(|| anyhow::anyhow!("mmap not initialized"))?;

        if chapter_index >= meta.chapters.len() {
            return Ok(None);
        }

        let cm = &meta.chapters[chapter_index];
        let start = cm.start_offset as usize;
        let end = start + cm.length as usize;

        if end > mmap.len() {
            return Ok(None);
        }

        let data = &mmap[start..end];
        let text = String::from_utf8_lossy(data);
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        Ok(Some(lines))
    }

    /// 更新热缓存中心章节（保持 ±3 章在内存）
    pub fn update_hot_cache(&self, center: usize, viewport_width: u16) {
        let mut hot = self.hot_cache.lock().unwrap();
        if hot.center_chapter == Some(center) {
            return;
        }
        hot.center_chapter = Some(center);

        // 移除超出范围的章节
        let min_idx = center.saturating_sub(3);
        let max_idx = center + 3;
        hot.chapters.retain(|&idx, _| idx >= min_idx && idx <= max_idx);

        // 预加载缺失的章节（异步或同步）
        // 这里同步预加载，实际可放后台线程
        if let Some(meta) = &self.meta {
            for idx in min_idx..=max_idx {
                if idx < meta.chapters.len() && !hot.chapters.contains_key(&idx) {
                    if let Ok(Some(lines)) = self.read_chapter(idx) {
                        let title = meta.chapters[idx].title.clone();
                        hot.chapters.insert(idx, CachedChapter { title, lines });
                    }
                }
            }
        }
    }

    /// 从热缓存获取章节（若命中）
    pub fn get_hot_chapter(&self, index: usize) -> Option<CachedChapter> {
        let hot = self.hot_cache.lock().unwrap();
        hot.chapters.get(&index).cloned()
    }

    /// 更新阅读进度
    pub fn update_progress(&mut self, chapter: usize, line_offset: usize) -> Result<()> {
        if let Some(meta) = &mut self.meta {
            meta.last_read_chapter = chapter;
            meta.last_line_offset = line_offset;

            let book_id = meta.book_id.clone();
            drop(meta); // 释放可变借用

            let (meta_path, _) = self.cache_paths(&book_id);
            let tmp_path = meta_path.with_extension("meta.json.tmp");

            let mut file = OpenOptions::new().create(true).write(true).truncate(true).open(&tmp_path)?;
            let json = serde_json::to_string_pretty(&self.meta.as_ref().unwrap())?;
            file.write_all(json.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
            fs::rename(&tmp_path, &meta_path)?;
        }
        Ok(())
    }

    /// 获取缓存目录大小（用于清理）
    pub fn cache_size(&self) -> Result<u64> {
        let mut total = 0;
        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                total += entry.metadata()?.len();
            }
        }
        Ok(total)
    }

    /// 清理旧缓存（保留最近 N 本）
    pub fn cleanup_old(&self, keep: usize) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(&self.cache_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();

        entries.sort_by_key(|e| e.metadata().map(|m| m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)).unwrap_or(std::time::SystemTime::UNIX_EPOCH));

        for entry in entries.iter().take(entries.len().saturating_sub(keep)) {
            fs::remove_file(entry.path()).ok();
        }
        Ok(())
    }
}

/// 缓存构建器（写入时使用）
pub struct CacheBuilder {
    book_id: String,
    book_title: String,
    epub_path: PathBuf,
    meta_path: PathBuf,
    content_path: PathBuf,
    tmp_meta: PathBuf,
    tmp_content: PathBuf,
    meta_file: File,
    content_file: File,
    chapters: Vec<ChapterMeta>,
    current_offset: u64,
}

impl CacheBuilder {
    pub fn set_book_title(&mut self, title: String) {
        self.book_title = title;
    }

    /// 添加一章内容
    pub fn add_chapter(&mut self, index: usize, title: String, content: &str) -> Result<()> {
        let bytes = content.as_bytes();
        self.content_file.write_all(bytes)?;
        self.content_file.flush()?;

        let chapter_meta = ChapterMeta {
            index,
            title,
            start_offset: self.current_offset,
            length: bytes.len() as u32,
            line_count: 0, // 后续按宽度换行时计算
        };
        self.chapters.push(chapter_meta);
        self.current_offset += bytes.len() as u64;
        Ok(())
    }

    /// 获取临时路径（用于外部写入）
    pub fn tmp_content_path(&self) -> &Path {
        &self.tmp_content
    }
}

/// 全局缓存管理器单例
static GLOBAL_CACHE: std::sync::OnceLock<Arc<Mutex<CacheManager>>> = std::sync::OnceLock::new();

pub fn global_cache() -> Arc<Mutex<CacheManager>> {
    GLOBAL_CACHE.get_or_init(|| Arc::new(Mutex::new(CacheManager::new().expect("init cache")))).clone()
}