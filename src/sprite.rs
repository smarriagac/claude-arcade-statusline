//! Sprite loading: the `.ansi` sprite file format, the text pixel-grid format
//! and the embedded default character.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::convert::{Grid, Rgb};

/// U+2800 BRAILLE PATTERN BLANK. Claude Code trims leading ASCII whitespace on
/// every status line row, so transparent cells must use this instead of ' '.
pub const BLANK: char = '\u{2800}';

/// Source grid of the embedded default character (an original generic soldier).
pub const DEFAULT_GRID: &str = include_str!("../assets/default-sprite.txt");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprite {
    /// Visible width in terminal cells. Every row is padded to this width.
    pub width: usize,
    pub rows: Vec<String>,
}

impl Sprite {
    /// Parses a sprite file: ANSI rows with an optional `#width N` line.
    /// ASCII spaces become braille blanks and short rows are padded, so the
    /// HUD next to the sprite always lines up. Returns `None` without rows.
    pub fn parse(text: &str) -> Option<Sprite> {
        let mut declared = 0;
        let mut rows = Vec::new();
        for line in text.lines() {
            match line.strip_prefix("#width ") {
                Some(n) => declared = n.trim().parse().unwrap_or(declared),
                None => rows.push(line.replace(' ', "\u{2800}")),
            }
        }
        (!rows.is_empty()).then(|| Sprite::padded(rows, declared))
    }

    /// Half-block rendering of a pixel grid.
    pub fn from_grid(grid: &[Vec<Option<Rgb>>]) -> Sprite {
        let width = grid.iter().map(Vec::len).max().unwrap_or(0);
        Sprite::padded(crate::convert::half_blocks(grid), width)
    }

    /// The embedded default character, rendered as half blocks.
    pub fn default_soldier() -> Sprite {
        // The asset is a compile-time constant covered by tests; an empty
        // sprite (compact layout) is the never-panic fallback.
        parse_grid(DEFAULT_GRID)
            .map(|grid| Sprite::from_grid(&grid))
            .unwrap_or(Sprite {
                width: 0,
                rows: Vec::new(),
            })
    }

    /// Serializes to the sprite file format (`#width N` header plus rows).
    pub fn to_file_string(&self) -> String {
        let mut out = format!("#width {}\n", self.width);
        for row in &self.rows {
            out.push_str(row);
            out.push('\n');
        }
        out
    }

    fn padded(mut rows: Vec<String>, min_width: usize) -> Sprite {
        let width = rows
            .iter()
            .map(|r| visible_width(r))
            .max()
            .unwrap_or(0)
            .max(min_width);
        for row in &mut rows {
            let missing = width - visible_width(row);
            row.extend(std::iter::repeat_n(BLANK, missing));
        }
        Sprite { width, rows }
    }
}

/// Calls `f` for every visible character, skipping ANSI CSI sequences
/// (`ESC [ ... final-byte`) and stray escape characters.
fn for_each_visible(text: &str, mut f: impl FnMut(char)) {
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            f(c);
            continue;
        }
        if chars.clone().next() == Some('[') {
            chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
}

/// Number of terminal cells a row occupies, ignoring ANSI escape sequences.
/// Every character is assumed to be one cell wide (true for half blocks,
/// braille and ASCII; wide CJK or emoji characters are not measured).
pub fn visible_width(row: &str) -> usize {
    let mut n = 0;
    for_each_visible(row, |_| n += 1);
    n
}

/// Removes ANSI CSI escape sequences.
#[cfg(test)]
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for_each_visible(text, |c| out.push(c));
    out
}

/// Parses the text pixel-grid format used by `assets/default-sprite.txt`:
/// `#` comments, `<key> <r> <g> <b>` palette lines, a `---` line, then one
/// character per pixel where `.` or a space is transparent.
pub fn parse_grid(text: &str) -> Result<Grid, String> {
    let mut palette: Vec<(char, Rgb)> = Vec::new();
    let mut grid: Grid = Vec::new();
    let mut in_grid = false;
    for (index, raw) in text.lines().enumerate() {
        let n = index + 1;
        let line = raw.trim_end();
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !in_grid {
            if line.trim() == "---" {
                in_grid = true;
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            let mut key = parts[0].chars();
            let (Some(k), None, 4) = (key.next(), key.next(), parts.len()) else {
                return Err(format!("line {n}: expected '<key> <r> <g> <b>'"));
            };
            if k == '.' || k == '#' {
                return Err(format!("line {n}: '{k}' cannot be a palette key"));
            }
            let mut rgb = [0u8; 3];
            for (slot, part) in rgb.iter_mut().zip(&parts[1..]) {
                *slot = part
                    .parse()
                    .map_err(|_| format!("line {n}: '{part}' is not a value in 0..=255"))?;
            }
            palette.push((k, rgb));
            continue;
        }
        let row = line
            .chars()
            .map(|c| match c {
                '.' | ' ' => Ok(None),
                _ => palette
                    .iter()
                    .find(|(k, _)| *k == c)
                    .map(|(_, rgb)| Some(*rgb))
                    .ok_or_else(|| format!("line {n}: unknown palette key '{c}'")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        grid.push(row);
    }
    if grid.is_empty() {
        return Err("no pixel grid found (expected a '---' line followed by pixel rows)".into());
    }
    Ok(grid)
}

/// `<home>/.config/arcade-statusline/sprite.ansi`
pub fn default_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("arcade-statusline")
        .join("sprite.ansi")
}

/// Resolution order: `--sprite`, `ARCADE_STATUSLINE_SPRITE`, the config file,
/// then the embedded default. Unreadable or empty candidates are skipped.
pub fn resolve(flag: Option<&Path>, env: Option<&OsStr>, home: Option<&Path>) -> Sprite {
    let config = home.map(default_path);
    let candidates = flag
        .into_iter()
        .chain(env.map(Path::new))
        .chain(config.as_deref());
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path)
            && let Some(sprite) = Sprite::parse(&String::from_utf8_lossy(&bytes))
        {
            return sprite;
        }
    }
    Sprite::default_soldier()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "arcade-sprite-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_uses_width_header_and_pads_rows() {
        let sprite = Sprite::parse("#width 5\nab\n\x1b[31mcde\x1b[0m\n").unwrap();
        assert_eq!(sprite.width, 5);
        assert_eq!(sprite.rows.len(), 2);
        assert_eq!(sprite.rows[0], "ab\u{2800}\u{2800}\u{2800}");
        assert_eq!(visible_width(&sprite.rows[1]), 5);
        assert!(sprite.rows[1].starts_with("\x1b[31mcde\x1b[0m"));
    }

    #[test]
    fn parse_without_header_measures_widest_row() {
        let sprite = Sprite::parse("abc\r\na\r\n").unwrap();
        assert_eq!(sprite.width, 3);
        assert_eq!(sprite.rows, vec!["abc", "a\u{2800}\u{2800}"]);
    }

    #[test]
    fn parse_header_narrower_than_content_keeps_content_width() {
        let sprite = Sprite::parse("#width 2\nabcd\n").unwrap();
        assert_eq!(sprite.width, 4);
    }

    #[test]
    fn parse_turns_ascii_spaces_into_braille_blanks() {
        let sprite = Sprite::parse("  x \n\n").unwrap();
        assert_eq!(sprite.rows[0], "\u{2800}\u{2800}x\u{2800}");
        assert_eq!(sprite.rows[1], "\u{2800}\u{2800}\u{2800}\u{2800}");
    }

    #[test]
    fn parse_empty_file_is_none() {
        assert!(Sprite::parse("").is_none());
        assert!(Sprite::parse("#width 7\n").is_none());
    }

    #[test]
    fn file_string_round_trips() {
        let sprite = Sprite::parse("#width 3\nab\nabc\n").unwrap();
        let text = sprite.to_file_string();
        assert!(text.starts_with("#width 3\n"));
        assert_eq!(Sprite::parse(&text).unwrap(), sprite);
    }

    #[test]
    fn strip_and_measure_ansi() {
        let row = "\x1b[38;2;1;2;3;48;2;4;5;6m▀\x1b[0m\u{2800}x";
        assert_eq!(strip_ansi(row), "▀\u{2800}x");
        assert_eq!(visible_width(row), 3);
    }

    #[test]
    fn default_soldier_is_sixteen_by_nine_half_blocks() {
        let sprite = Sprite::default_soldier();
        assert_eq!(sprite.width, 16);
        assert_eq!(sprite.rows.len(), 9);
        for row in &sprite.rows {
            assert_eq!(visible_width(row), 16);
            assert!(!row.contains(' '));
        }
        // Top-left pixels of the grid are transparent.
        assert!(sprite.rows[0].starts_with(BLANK));
    }

    #[test]
    fn grid_parser_reads_palette_and_transparency() {
        let grid = parse_grid("# c\nA 1 2 3\n---\nA.\n.A\n").unwrap();
        assert_eq!(
            grid,
            vec![vec![Some([1, 2, 3]), None], vec![None, Some([1, 2, 3])]]
        );
        assert!(parse_grid("A 1 2 3\n---\nAZ\n").is_err());
        assert!(parse_grid("A 1 2\n---\nA\n").is_err());
    }

    #[test]
    fn resolve_follows_priority_order() {
        let dir = temp_dir("resolve");
        let flag = dir.join("flag.ansi");
        let env = dir.join("env.ansi");
        fs::write(&flag, "F\n").unwrap();
        fs::write(&env, "E\n").unwrap();
        let config = default_path(&dir);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "C\n").unwrap();

        let rows = |s: Sprite| s.rows;
        assert_eq!(
            rows(resolve(Some(&flag), Some(env.as_os_str()), Some(&dir))),
            ["F"]
        );
        assert_eq!(
            rows(resolve(None, Some(env.as_os_str()), Some(&dir))),
            ["E"]
        );
        assert_eq!(rows(resolve(None, None, Some(&dir))), ["C"]);
        // An unreadable candidate falls through to the next one.
        let missing = dir.join("missing.ansi");
        assert_eq!(rows(resolve(Some(&missing), None, Some(&dir))), ["C"]);
        fs::remove_file(&config).unwrap();
        assert_eq!(resolve(None, None, Some(&dir)), Sprite::default_soldier());
        assert_eq!(resolve(None, None, None), Sprite::default_soldier());
        fs::remove_dir_all(&dir).unwrap();
    }
}
