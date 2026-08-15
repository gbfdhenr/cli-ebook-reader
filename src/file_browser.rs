use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{Terminal, Frame};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::style::{Style, Color, Modifier};
use ratatui::text::{Line, Span};
use std::io;
use std::path::PathBuf;
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
}

impl FileBrowser {
    pub fn new(start_path: PathBuf) -> Self {
        let mut browser = Self {
            current_dir: start_path,
            entries: Vec::new(),
            state: ListState::default(),
            quit_requested: false,
            selected_file: None,
            show_hidden: false,
            last_error: None,
        };
        browser.refresh();
        browser
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.last_error = None;

        // Add parent directory entry if not at root
        if self.current_dir.parent().is_some() {
            self.entries.push(FileEntry {
                path: self.current_dir.parent().unwrap().to_path_buf(),
                name: "..".to_string(),
                is_dir: true,
                is_epub: false,
            });
        }

        match fs::read_dir(&self.current_dir) {
            Ok(read_dir) => {
                let mut dirs = Vec::new();
                let mut epubs = Vec::new();
                let mut others = Vec::new();

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
                    } else {
                        others.push(entry);
                    }
                }

                // Sort each category alphabetically
                dirs.sort_by(|a, b| a.name.cmp(&b.name));
                epubs.sort_by(|a, b| a.name.cmp(&b.name));
                others.sort_by(|a, b| a.name.cmp(&b.name));

                self.entries.extend(dirs);
                self.entries.extend(epubs);
                self.entries.extend(others);
            }
            Err(e) => {
                self.last_error = Some(format!("Permission denied: {}", e));
            }
        }

        if !self.entries.is_empty() {
            self.state.select(Some(0));
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
                self.refresh();
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let selected = self.state.selected().unwrap_or(0);
                if selected > 0 {
                    self.state.select(Some(selected - 1));
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let selected = self.state.selected().unwrap_or(0);
                if selected + 1 < self.entries.len() {
                    self.state.select(Some(selected + 1));
                }
                None
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(idx) = self.state.selected() {
                    if let Some(entry) = self.entries.get(idx) {
                        if entry.is_dir {
                            self.current_dir = entry.path.clone();
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
                    self.current_dir = parent.to_path_buf();
                    self.refresh();
                }
                None
            }
            KeyCode::PageUp => {
                let selected = self.state.selected().unwrap_or(0);
                let new_idx = selected.saturating_sub(10);
                self.state.select(Some(new_idx));
                None
            }
            KeyCode::PageDown => {
                let selected = self.state.selected().unwrap_or(0);
                let new_idx = (selected + 10).min(self.entries.len().saturating_sub(1));
                self.state.select(Some(new_idx));
                None
            }
            KeyCode::Home => {
                self.state.select(Some(0));
                None
            }
            KeyCode::End => {
                if !self.entries.is_empty() {
                    self.state.select(Some(self.entries.len() - 1));
                }
                None
            }
            _ => None,
        }
    }

    pub fn is_quit_requested(&self) -> bool {
        self.quit_requested
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

        // Header
        let mut header_text = format!("📁 {}", self.current_dir.display());
        if self.show_hidden {
            header_text.push_str("  [showing hidden]");
        }
        
        let header = Paragraph::new(header_text)
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL).title("File Browser"));
        frame.render_widget(header, chunks[0]);

        // File list
        let items: Vec<ListItem> = self.entries.iter().enumerate().map(|(i, entry)| {
            let (icon, base_style) = if entry.is_dir {
                ("📂", Style::default().fg(Color::Blue))
            } else if entry.is_epub {
                ("📖", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                ("📄", Style::default().fg(Color::DarkGray))
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

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Files (./.epub)"))
            .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black))
            .highlight_symbol("► ");
        
        frame.render_stateful_widget(list, chunks[1], &mut self.state);

        // Footer with error or hints
        let footer_text = if let Some(err) = &self.last_error {
            format!("Error: {}", err)
        } else {
            "↑/↓/j/k: Navigate  Enter/l: Open  h/←/Backspace: Up  .: Toggle hidden  q/Esc: Quit".to_string()
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

    let mut browser = FileBrowser::new(start_path);

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
                terminal.clear()?;
                return Ok(Some(path));
            }
            if browser.is_quit_requested() {
                terminal.clear()?;
                return Ok(None);
            }
        }
    }
}