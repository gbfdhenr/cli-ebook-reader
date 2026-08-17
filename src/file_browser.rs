use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{Terminal, Frame};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::style::{Style, Color, Modifier};
use ratatui::text::{Line, Span};
use std::io;
use std::path::{Path, PathBuf};
use std::fs;

use crate::common::events::{read_key, check_resize};

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_epub: bool,
}

pub struct FileBrowser {
    current_dir: PathBuf,
    entries: Vec<FileEntry>,
    state: ListState,
    quit_requested: bool,
    selected_file: Option<PathBuf>,
    show_hidden: bool,
    last_error: Option<String>,
    /// 当前选中的条目索引（用于刷新后恢复）
    selected_index: usize,
}

impl FileBrowser {
    pub fn new(start_path: PathBuf) -> Self {
        // 规范化为绝对路径
        let abs_path = Self::resolve_absolute_path(&start_path);
        let mut browser = Self {
            current_dir: abs_path,
            entries: Vec::new(),
            state: ListState::default(),
            quit_requested: false,
            selected_file: None,
            show_hidden: false,
            last_error: None,
            selected_index: 0,
        };
        browser.refresh();
        browser
    }

    /// 将路径解析为绝对路径并规范化（解析符号链接）
    fn resolve_absolute_path(path: &Path) -> PathBuf {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
        };
        // 使用 canonicalize 解析符号链接，获取真实路径，避免循环
        abs_path.canonicalize().unwrap_or(abs_path)
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.last_error = None;

        // Add parent directory entry if not at root
        if let Some(parent) = self.current_dir.parent() {
            let parent_canonical = parent.canonicalize().unwrap_or(parent.to_path_buf());
            self.entries.push(FileEntry {
                path: parent_canonical,
                name: "..".to_string(),
                is_dir: true,
                is_epub: false,
            });
        }

        match fs::read_dir(&self.current_dir) {
            Ok(read_dir) => {
                let mut dirs = Vec::new();
                let mut epubs = Vec::new();

                for entry in read_dir.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();

                    // Skip hidden files unless show_hidden is true
                    if name.starts_with('.') && !self.show_hidden {
                        continue;
                    }

                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    let ext = path.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase());
                    let is_epub = !is_dir && ext.as_deref() == Some("epub");

                    let entry = FileEntry { path, name, is_dir, is_epub };

                    if is_dir {
                        dirs.push(entry);
                    } else if is_epub {
                        epubs.push(entry);
                    }
                    // 忽略非 EPUB 文件
                }

                // Sort each category alphabetically
                dirs.sort_by(|a, b| a.name.cmp(&b.name));
                epubs.sort_by(|a, b| a.name.cmp(&b.name));

                self.entries.extend(dirs);
                self.entries.extend(epubs);
            }
            Err(e) => {
                self.last_error = Some(format!("无法读取目录 '{}': {}", self.current_dir.display(), e));
            }
        }

        // 恢复选中索引，但不超出范围
        let new_index = self.selected_index.min(self.entries.len().saturating_sub(1));
        if !self.entries.is_empty() {
            self.state.select(Some(new_index));
            self.selected_index = new_index;
        } else {
            self.state.select(None);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<PathBuf> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.quit_requested = true;
                None
            }
            KeyCode::Char('.') => {
                self.show_hidden = !self.show_hidden;
                // 保持当前选中项（如果还在范围内）
                self.refresh();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let selected = self.state.selected().unwrap_or(0);
                if selected > 0 {
                    self.state.select(Some(selected - 1));
                    self.selected_index = selected - 1;
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let selected = self.state.selected().unwrap_or(0);
                if selected + 1 < self.entries.len() {
                    self.state.select(Some(selected + 1));
                    self.selected_index = selected + 1;
                }
                None
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(idx) = self.state.selected() {
                    if let Some(entry) = self.entries.get(idx) {
                        if entry.is_dir {
                            // 使用 canonicalize 规范化路径，避免符号链接问题
                            self.current_dir = entry.path.canonicalize().unwrap_or(entry.path.clone());
                            self.selected_index = 0; // 进入目录重置为第一项
                            self.refresh();
                        } else if entry.is_epub {
                            self.selected_file = Some(entry.path.clone());
                            return Some(entry.path.clone());
                        }
                    }
                }
                None
            }
            KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => {
                if let Some(parent) = self.current_dir.parent() {
                    // 使用 canonicalize 规范化父目录路径
                    self.current_dir = parent.canonicalize().unwrap_or(parent.to_path_buf());
                    self.selected_index = 0;
                    self.refresh();
                }
                None
            }
            KeyCode::PageUp => {
                let selected = self.state.selected().unwrap_or(0);
                let new_idx = selected.saturating_sub(10);
                self.state.select(Some(new_idx));
                self.selected_index = new_idx;
                None
            }
            KeyCode::PageDown => {
                let selected = self.state.selected().unwrap_or(0);
                let new_idx = (selected + 10).min(self.entries.len().saturating_sub(1));
                self.state.select(Some(new_idx));
                self.selected_index = new_idx;
                None
            }
            KeyCode::Home => {
                self.state.select(Some(0));
                self.selected_index = 0;
                None
            }
            KeyCode::End => {
                if !self.entries.is_empty() {
                    let last = self.entries.len() - 1;
                    self.state.select(Some(last));
                    self.selected_index = last;
                }
                None
            }
            _ => None,
        }
    }

    pub fn is_quit_requested(&self) -> bool {
        self.quit_requested
    }

    /// 获取当前目录的显示路径（截断过长路径）
    fn get_display_path(&self, max_width: usize) -> String {
        let path_str = self.current_dir.to_string_lossy();
        if path_str.len() <= max_width {
            path_str.to_string()
        } else {
            // 保留开头和结尾，中间用 ... 省略
            let keep_start = max_width / 3;
            let keep_end = max_width - keep_start - 3;
            format!("{}...{}", &path_str[..keep_start], &path_str[path_str.len() - keep_end..])
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(area);

        // Header - 显示绝对路径，过长时截断
        let header_width = area.width.saturating_sub(4) as usize; // 减去边框
        let display_path = self.get_display_path(header_width);
        let mut header_text = format!("📁 {}", display_path);
        if self.show_hidden {
            header_text.push_str("  [显示隐藏]");
        }

        let header = Paragraph::new(header_text)
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL).title(" 文件浏览器 "));
        frame.render_widget(header, chunks[0]);

        // File list - 仅显示目录和 EPUB
        let items: Vec<ListItem> = self.entries.iter().enumerate().map(|(i, entry)| {
            let (icon, base_style) = if entry.is_dir {
                ("📂", Style::default().fg(Color::Blue))
            } else { // EPUB
                ("📖", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            };

            let style = if self.state.selected() == Some(i) {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                base_style
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", icon), style),
                Span::styled(&entry.name, style),
            ]))
        }).collect();

        let title = if self.entries.iter().any(|e| e.is_epub) {
            " 文件 (目录/EPUB) "
        } else {
            " 文件 (仅目录) "
        };
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black))
            .highlight_symbol("► ");

        frame.render_stateful_widget(list, chunks[1], &mut self.state);

        // Footer with error or hints
        let footer_text = if let Some(err) = &self.last_error {
            format!("错误: {}", err)
        } else {
            "↑/↓/j/k: 移动  Enter/l: 打开  h/←/Backspace: 上级  .: 切换隐藏  q/Esc: 退出".to_string()
        };

        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(if self.last_error.is_some() { Color::Red } else { Color::DarkGray }))
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        frame.render_widget(footer, chunks[2]);
    }
}

pub fn run_file_browser(start_path: PathBuf) -> Result<Option<PathBuf>> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

    let mut browser = FileBrowser::new(start_path);

    let result = (|| {
        loop {
            // Check for terminal resize
            if check_resize() {
                terminal.autoresize()?;
            }

            terminal.draw(|frame| {
                let area = frame.area();
                browser.draw(frame, area);
            })?;

            if let Some(key) = read_key(100)? {
                if let Some(path) = browser.handle_key(key) {
                    return Ok(Some(path));
                }
                if browser.is_quit_requested() {
                    return Ok(None);
                }
            }
        }
        #[allow(unreachable_code)]
        Ok(None)
    })();

    crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
    terminal.clear()?;
    result
}