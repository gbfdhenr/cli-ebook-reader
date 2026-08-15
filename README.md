# cli-ebook-reader

**English | [中文](README_zh.md)**

A terminal-based EPUB ebook reader with vim-like keybindings, built with Rust and ratatui.

## Features

- 📖 **EPUB Support** - Parse and read EPUB files with proper chapter ordering (spine-based)
- ⌨️ **Vim-like Navigation** - `j/k` scroll, `h/l` prev/next chapter, `Ctrl+C` twice to quit
- 🖱️ **Mouse Support** - Click bottom bar buttons: `[退出]` `[ < ]` `[ > ]` `☰ Menu`
- ⚡ **Persistent Cache** - Memory-mapped file cache (`~/.cache/cli-ebook-reader/`) with ±3 chapter hot cache
- 📊 **Progress Tracking** - Reading progress percentage, line position, chapter info
- 🎨 **Clean UI** - Top bar (book title + time), chapter header, content area, bottom bar
- 🌍 **Unicode Support** - Proper Chinese/Japanese/Korean text wrapping via `unicode-width`

## Screenshots

```
┌─────────────────────────────────────────────────────────────┐
│           The Great Gatsby                    14:32        │  ← Top bar
├─────────────────────────────────────────────────────────────┤
│ 📖 Chapter 1  (1/9)  Lines: 45/1234                       │  ← Chapter header
├─────────────────────────────────────────────────────────────┤
│                                                             │
│   In my younger and more vulnerable years my father        │
│   gave me some advice that I've been turning over in       │
│   my mind ever since.                                      │  ← Content area
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ [退出] [ < ]    ☰ Menu    [ > ]                        12%  │  ← Bottom bar
└─────────────────────────────────────────────────────────────┘
```

## Installation

### From Source

```bash
git clone https://github.com/gbfdhenr/cli-ebook-reader
cd cli-ebook-reader
cargo install --path .
```

### Debian/Ubuntu

```bash
# Build .deb package
dpkg-buildpackage -us -uc
sudo dpkg -i ../cli-ebook-reader_1.0.0_amd64.deb
```

## Usage

```bash
# Run in current directory (shows file browser)
cli-ebook-reader

# Or specify EPUB directly
cli-ebook-reader /path/to/book.epub
```

### Keybindings

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down one line |
| `k` / `↑` | Scroll up one line |
| `PgDn` / `PgUp` | Page down/up |
| `Home` / `End` | Jump to top/bottom |
| `l` / `n` / `→` | Next chapter |
| `h` / `p` / `←` | Previous chapter |
| `m` | Toggle menu (placeholder) |
| `Ctrl+C` (×2 in 3s) | Quit with confirmation |
| Click `[退出]` | Quit with confirmation (press `y`) |
| Click `[ < ]` / `[ > ]` | Prev/Next chapter |

### File Browser

- `j/k` / `↑/↓` - Navigate
- `Enter` / `l` / `→` - Open directory / Select EPUB
- `h` / `←` / `Backspace` - Parent directory
- `.` - Toggle hidden files
- `q` / `Esc` - Quit

## Cache System

```
~/.cache/cli-ebook-reader/
├── <book-id>.meta.json      # Metadata (title, chapter index, progress)
└── <book-id>.content.dat    # Full text content (memory-mapped, swap-backed)
```

- **First open**: Parses EPUB, builds cache (~1-3s for typical books)
- **Subsequent opens**: Instant load from mmap cache
- **Hot cache**: ±3 chapters around current kept in memory for instant navigation
- **Progress**: Auto-saved on exit

## Dependencies

- `ratatui` 0.30 - TUI framework
- `epub` 2.1 - EPUB parsing
- `html2text` - HTML to text conversion
- `unicode-width` - CJK text width calculation
- `memmap2` - Memory-mapped file cache
- `blake3` - Fast hashing for cache keys
- `signal-hook` - SIGWINCH for terminal resize

## License

GPL-3.0 - See [LICENSE](LICENSE) for details.

## Author

**gbfdhenr** (Liang Xiangan) - [GitHub](https://github.com/gbfdhenr)