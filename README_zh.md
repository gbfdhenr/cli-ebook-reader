# cli-ebook-reader

**[English](README.md) | 中文**

基于 Rust 和 ratatui 的终端 EPUB 电子书阅读器，支持类 Vim 键位、鼠标操作、持久化缓存。

## 功能特性

- 📖 **EPUB 支持** - 按 spine 顺序正确解析章节，而非资源 ID 顺序
- ⌨️ **类 Vim 导航** - `j/k` 滚动，`h/l` 翻章，`Ctrl+C` 双击退出
- 🖱️ **鼠标支持** - 底栏按钮点击：`[退出]` `[ < ]` `[ > ]` `☰ Menu`
- ⚡ **持久化缓存** - 内存映射文件缓存（`~/.cache/cli-ebook-reader/`）+ ±3 章热缓存
- 📊 **进度追踪** - 阅读进度百分比、行号位置、章节信息
- 🎨 **简洁 UI** - 顶栏（书名+时间）、章节标题栏、正文区、底栏
- 🌍 **Unicode 支持** - 通过 `unicode-width` 正确处理中日韩文本换行

## 界面预览

```
┌─────────────────────────────────────────────────────────────┐
│           The Great Gatsby                    14:32        │  ← 顶栏
├─────────────────────────────────────────────────────────────┤
│ 📖 Chapter 1  (1/9)  Lines: 45/1234                       │  ← 章节标题栏
├─────────────────────────────────────────────────────────────┤
│                                                             │
│   In my younger and more vulnerable years my father        │
│   gave me some advice that I've been turning over in       │
│   my mind ever since.                                      │  ← 正文区
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ [退出] [ < ]    ☰ Menu    [ > ]                        12%  │  ← 底栏
└─────────────────────────────────────────────────────────────┘
```

## 安装方式

### 源码编译

```bash
git clone https://github.com/gbfdhenr/cli-ebook-reader
cd cli-ebook-reader
cargo install --path .
```

### Debian/Ubuntu (.deb 包)

```bash
# 构建 .deb 包
dpkg-buildpackage -us -uc
sudo dpkg -i ../cli-ebook-reader_1.0.0_amd64.deb
```

## 使用方法

```bash
# 在当前目录运行（显示文件浏览器）
cli-ebook-reader

# 或直接指定 EPUB 文件
cli-ebook-reader /path/to/book.epub
```

### 键位绑定

| 按键 | 功能 |
|------|------|
| `j` / `↓` | 向下滚动一行 |
| `k` / `↑` | 向上滚动一行 |
| `PgDn` / `PgUp` | 翻页向下/向上 |
| `Home` / `End` | 跳转到章首/章尾 |
| `l` / `n` / `→` | 下一章 |
| `h` / `p` / `←` | 上一章 |
| `m` | 切换菜单（预留） |
| `Ctrl+C` (3秒内两次) | 确认退出 |
| 点击 `[退出]` | 确认退出（按 `y` 确认） |
| 点击 `[ < ]` / `[ > ]` | 上一章/下一章 |

### 文件浏览器

- `j/k` / `↑/↓` - 上下选择
- `Enter` / `l` / `→` - 进入目录 / 选中 EPUB
- `h` / `←` / `Backspace` - 返回上级目录
- `.` - 切换隐藏文件显示
- `q` / `Esc` - 退出程序

## 缓存系统

```
~/.cache/cli-ebook-reader/
├── <book-id>.meta.json      # 元数据（书名、章节索引、阅读进度）
└── <book-id>.content.dat    # 全文内容（内存映射，swap-backed）
```

- **首次打开**：解析 EPUB，构建缓存（典型书籍 1-3 秒）
- **再次打开**：从 mmap 瞬间加载，无需重新解析
- **热缓存**：当前章 ±3 章常驻内存，翻章零延迟
- **进度自动保存**：退出时自动记录当前章节和行偏移

## 核心依赖

- `ratatui` 0.30 - TUI 框架
- `epub` 2.1 - EPUB 解析
- `html2text` - HTML 转纯文本
- `unicode-width` - CJK 字符宽度计算
- `memmap2` - 内存映射文件缓存
- `blake3` - 高性能哈希（缓存键）
- `signal-hook` - SIGWINCH 终端 resize 处理

## 许可证

GPL-3.0 - 详见 [LICENSE](LICENSE)

## 作者

**gbfdhenr** (Liang Xiangan) - [GitHub](https://github.com/gbfdhenr)