//! Image to sprite conversion, ported from the reference Python algorithms.
//!
//! - **Pixel mode**: downscales the image to a pixel grid (dominant,
//!   fill-preferring color per block) and draws two vertical pixels per text
//!   cell with half blocks (`▀` foreground = top, background = bottom).
//! - **ASCII mode**: one colored character per cell, picked by coverage and
//!   luminance ("fastfetch logo" style), after palette quantization.
//!
//! Transparent cells are always U+2800 (braille blank), never ASCII spaces.
//!
//! Differences from the Python reference, all deliberate:
//! - Alpha is composited over white, so transparent PNG areas are background
//!   (PIL's `convert("RGB")` drops alpha and turns them black).
//! - Every block covers at least one source pixel, so images smaller than the
//!   target grid upscale instead of dividing by zero.
//! - PIL's `quantize(method=MAXCOVERAGE)` is re-implemented by [`quantize`]
//!   (farthest-point palette plus nearest-color mapping). It matches Pillow
//!   except when two colors tie, where it picks the lowest RGB value.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::Hash;
use std::path::Path;

use crate::sprite::{self, BLANK, Sprite};

pub type Rgb = [u8; 3];
/// Rows of pixels; `None` is transparent.
pub type Grid = Vec<Vec<Option<Rgb>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ascii,
    Pixel,
}

impl Mode {
    /// Default text rows: 12 for ASCII mode, 10 (20 pixels) for pixel mode.
    pub fn default_rows(self) -> usize {
        match self {
            Mode::Ascii => 12,
            Mode::Pixel => 10,
        }
    }
}

/// An opaque RGB image, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub data: Vec<Rgb>,
}

impl Pixels {
    fn at(&self, x: usize, y: usize) -> Rgb {
        self.data[y * self.width + x]
    }

    /// Pixels of the block `xs` x `ys`, row by row.
    fn block(&self, xs: (usize, usize), ys: (usize, usize)) -> impl Iterator<Item = Rgb> + '_ {
        (ys.0..ys.1).flat_map(move |y| (xs.0..xs.1).map(move |x| self.at(x, y)))
    }
}

pub fn is_white(c: Rgb) -> bool {
    c[0] > 225 && c[1] > 225 && c[2] > 225
}

/// Relative luminance in 0..=1 (ITU-R BT.601 weights).
pub fn lum(c: Rgb) -> f64 {
    (0.299 * f64::from(c[0]) + 0.587 * f64::from(c[1]) + 0.114 * f64::from(c[2])) / 255.0
}

/// Composites RGBA over white, so transparent areas count as background.
pub fn flatten_alpha(img: &image::RgbaImage) -> Pixels {
    let data = img
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let a = u32::from(a);
            let mix = |c: u8| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
            [mix(r), mix(g), mix(b)]
        })
        .collect();
    Pixels {
        width: img.width() as usize,
        height: img.height() as usize,
        data,
    }
}

/// Crops to the bounding box of non-white pixels; `None` if all white.
pub fn crop_to_content(p: &Pixels) -> Option<Pixels> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..p.height {
        for x in 0..p.width {
            if !is_white(p.at(x, y)) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 == 0 {
        return None;
    }
    Some(Pixels {
        width: x1 - x0,
        height: y1 - y0,
        data: p.block((x0, x1), (y0, y1)).collect(),
    })
}

/// Python `colorsys.rgb_to_hls`.
fn rgb_to_hls(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let sumc = maxc + minc;
    let rangec = maxc - minc;
    let l = sumc / 2.0;
    if minc == maxc {
        return (0.0, l, 0.0);
    }
    let s = if l <= 0.5 {
        rangec / sumc
    } else {
        rangec / (2.0 - maxc - minc)
    };
    let rc = (maxc - r) / rangec;
    let gc = (maxc - g) / rangec;
    let bc = (maxc - b) / rangec;
    let h = if r == maxc {
        bc - gc
    } else if g == maxc {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    ((h / 6.0).rem_euclid(1.0), l, s)
}

/// Python `colorsys.hls_to_rgb`.
fn hls_to_rgb(h: f64, l: f64, s: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l, l, l);
    }
    let m2 = if l <= 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let m1 = 2.0 * l - m2;
    let v = |hue: f64| {
        let hue = hue.rem_euclid(1.0);
        if hue < 1.0 / 6.0 {
            m1 + (m2 - m1) * hue * 6.0
        } else if hue < 0.5 {
            m2
        } else if hue < 2.0 / 3.0 {
            m1 + (m2 - m1) * (2.0 / 3.0 - hue) * 6.0
        } else {
            m1
        }
    };
    (v(h + 1.0 / 3.0), v(h), v(h - 1.0 / 3.0))
}

/// Boosts saturation and keeps lightness in a readable band.
pub fn vivid(c: Rgb) -> Rgb {
    let unit = |v: u8| f64::from(v) / 255.0;
    let (h, l, s) = rgb_to_hls(unit(c[0]), unit(c[1]), unit(c[2]));
    let s = (s * 1.35).min(1.0);
    let l = (l * 1.1).clamp(0.32, 0.78);
    let (r, g, b) = hls_to_rgb(h, l, s);
    // `as` truncates like Python's int() and saturates out-of-range values.
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

fn distance2(a: Rgb, b: Rgb) -> u32 {
    (0..3)
        .map(|ch| {
            let d = i32::from(a[ch]) - i32::from(b[ch]);
            (d * d) as u32
        })
        .sum()
}

/// Deterministic "maximum coverage" quantization to at most `colors` colors,
/// matching the observable behavior of Pillow's `Quantize.MAXCOVERAGE`:
///
/// 1. the first palette entry is the image color furthest from the mean pixel;
/// 2. each next entry is the image color furthest from every entry so far
///    (farthest-point sampling), so extremes such as white survive intact;
/// 3. every pixel maps to its nearest entry (squared RGB distance).
///
/// Palette entries are real image colors, never averages. Ties go to the
/// lowest RGB value (Pillow breaks them by hash order).
pub fn quantize(p: &Pixels, colors: usize) -> Pixels {
    let colors = colors.clamp(1, 256);
    let mut distinct = p.data.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() <= colors || p.data.is_empty() {
        return p.clone();
    }
    let mut sum = [0u64; 3];
    for c in &p.data {
        for ch in 0..3 {
            sum[ch] += u64::from(c[ch]);
        }
    }
    let mean = sum.map(|s| (s / p.data.len() as u64) as u8);

    // `reach[i]`: distance from distinct[i] to its nearest entry, `owner[i]`
    // that entry. Before the first pick, the distance is to the mean.
    let mut reach: Vec<u32> = distinct.iter().map(|&c| distance2(c, mean)).collect();
    let mut owner = vec![0usize; distinct.len()];
    let mut palette = Vec::with_capacity(colors);
    for k in 0..colors {
        let mut best = 0;
        for i in 1..distinct.len() {
            if reach[i] > reach[best] {
                best = i;
            }
        }
        let pick = distinct[best];
        palette.push(pick);
        for (i, &c) in distinct.iter().enumerate() {
            let d = distance2(c, pick);
            if k == 0 || d < reach[i] {
                reach[i] = d;
                owner[i] = k;
            }
        }
    }
    let data = p
        .data
        .iter()
        .map(|c| {
            let i = distinct.binary_search(c).unwrap_or(0);
            palette[owner[i]]
        })
        .collect();
    Pixels {
        width: p.width,
        height: p.height,
        data,
    }
}

/// `[start, end)` of block `i` out of `n` over `total` pixels, like Python's
/// `int(i * W / n)`, but never empty (tiny images upscale).
fn span(i: usize, n: usize, total: usize) -> (usize, usize) {
    let start = i * total / n;
    let end = ((i + 1) * total / n).max(start + 1).min(total);
    (start, end)
}

/// Python `Counter(items).most_common(1)[0][0]`: ties go to the first seen.
fn most_common<K: Copy + Eq + Hash>(items: impl IntoIterator<Item = K>) -> Option<K> {
    let mut counts: HashMap<K, (u32, usize)> = HashMap::new();
    for (order, key) in items.into_iter().enumerate() {
        counts.entry(key).or_insert((0, order)).0 += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(_, (n, first))| (n, Reverse(first)))
        .map(|(key, _)| key)
}

/// ASCII mode on an already cropped image. A text cell is about twice as
/// tall as wide, so columns = width / height * rows * 2.1.
pub fn ascii_art(src: &Pixels, rows: usize, colors: usize) -> Sprite {
    let q = quantize(src, colors);
    let cols = ((src.width as f64 / src.height as f64 * rows as f64 * 2.1).round_ties_even()
        as usize)
        .max(1);
    let mut out = Vec::with_capacity(rows);
    let (mut nw, mut light) = (Vec::new(), Vec::new());
    for j in 0..rows {
        let ys = span(j, rows, src.height);
        let mut line = String::with_capacity(cols * 24);
        for i in 0..cols {
            let xs = span(i, cols, src.width);
            let total = (xs.1 - xs.0) * (ys.1 - ys.0);
            nw.clear();
            nw.extend(q.block(xs, ys).filter(|c| !is_white(*c)));
            let cov = nw.len() as f64 / total as f64;
            if cov < 0.18 {
                line.push(BLANK);
                continue;
            }
            light.clear();
            light.extend(nw.iter().copied().filter(|c| lum(*c) >= 0.22));
            let pool = if light.len() as f64 >= 0.35 * nw.len() as f64 {
                &light
            } else {
                &nw
            };
            let Some(dom) = most_common(pool.iter().copied()) else {
                line.push(BLANK);
                continue;
            };
            let l = lum(dom);
            let ch = if cov < 0.4 {
                if l < 0.5 { ':' } else { '+' }
            } else if cov < 0.7 {
                if l > 0.6 {
                    's'
                } else if l > 0.4 {
                    'y'
                } else {
                    'h'
                }
            } else if l > 0.6 {
                'M'
            } else if l > 0.4 {
                'N'
            } else if l > 0.25 {
                'm'
            } else {
                'd'
            };
            let [r, g, b] = vivid(dom);
            let _ = write!(line, "\x1b[1;38;2;{r};{g};{b}m{ch}\x1b[0m");
        }
        out.push(line);
    }
    Sprite {
        width: cols,
        rows: out,
    }
}

/// Image -> pixel grid of `height_px` rows by dominant fill-preferring color:
/// a block is transparent when less than `thr` of it is non-white; otherwise
/// its most common coarse color (channels / 28), preferring light pixels,
/// averaged over the pixels of that coarse color.
pub fn pixel_downscale(src: &Pixels, height_px: usize, thr: f64) -> Grid {
    let key = |c: Rgb| c.map(|v| v / 28);
    let wpx =
        (((src.width * height_px) as f64 / src.height as f64).round_ties_even() as usize).max(1);
    let (mut nw, mut light) = (Vec::new(), Vec::new());
    let mut grid = Vec::with_capacity(height_px);
    for j in 0..height_px {
        let ys = span(j, height_px, src.height);
        let mut row = Vec::with_capacity(wpx);
        for i in 0..wpx {
            let xs = span(i, wpx, src.width);
            let total = (xs.1 - xs.0) * (ys.1 - ys.0);
            nw.clear();
            nw.extend(src.block(xs, ys).filter(|c| !is_white(*c)));
            if (nw.len() as f64) < thr * total as f64 {
                row.push(None);
                continue;
            }
            light.clear();
            light.extend(nw.iter().copied().filter(|c| lum(*c) >= 70.0 / 255.0));
            let pool = if (light.len() as f64) < 0.4 * nw.len() as f64 {
                &nw
            } else {
                &light
            };
            let Some(q) = most_common(pool.iter().map(|c| key(*c))) else {
                row.push(None);
                continue;
            };
            let (mut sum, mut n) = ([0usize; 3], 0usize);
            for c in pool.iter().filter(|c| key(**c) == q) {
                for ch in 0..3 {
                    sum[ch] += usize::from(c[ch]);
                }
                n += 1;
            }
            row.push(Some(sum.map(|s| (s / n) as u8)));
        }
        grid.push(row);
    }
    grid
}

/// Pixel grid -> half-block rows, two pixel rows per text row.
pub fn half_blocks(grid: &[Vec<Option<Rgb>>]) -> Vec<String> {
    grid.chunks(2)
        .map(|pair| {
            let top = &pair[0];
            let bottom = pair.get(1);
            let width = top.len().max(bottom.map_or(0, Vec::len));
            let mut s = String::with_capacity(width * 32);
            for x in 0..width {
                let t = top.get(x).copied().flatten();
                let b = bottom.and_then(|row| row.get(x).copied().flatten());
                let _ = match (t, b) {
                    (None, None) => {
                        s.push(BLANK);
                        Ok(())
                    }
                    (Some([r, g, b]), None) => write!(s, "\x1b[38;2;{r};{g};{b}m▀\x1b[0m"),
                    (None, Some([r, g, b])) => write!(s, "\x1b[38;2;{r};{g};{b}m▄\x1b[0m"),
                    (Some([r, g, b]), Some([br, bg, bb])) => {
                        write!(s, "\x1b[38;2;{r};{g};{b};48;2;{br};{bg};{bb}m▀\x1b[0m")
                    }
                };
            }
            s
        })
        .collect()
}

/// Converts an opaque image (white = background) in the given mode.
pub fn convert_pixels(
    p: &Pixels,
    mode: Mode,
    rows: usize,
    colors: usize,
) -> Result<Sprite, String> {
    let rows = rows.max(1);
    let src = crop_to_content(p).ok_or("the image has no non-white content to draw")?;
    Ok(match mode {
        Mode::Ascii => ascii_art(&src, rows, colors),
        Mode::Pixel => Sprite::from_grid(&pixel_downscale(&src, rows * 2, 0.35)),
    })
}

/// Converts an image file (PNG/JPG/GIF) into a sprite. A `.txt` file is read
/// as a text pixel grid and drawn pixel-exact, whatever the mode and rows.
pub fn convert_file(path: &Path, mode: Mode, rows: usize, colors: usize) -> Result<Sprite, String> {
    let shown = path.display();
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
    {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
        let grid = sprite::parse_grid(&text).map_err(|e| format!("{shown}: {e}"))?;
        return Ok(Sprite::from_grid(&grid));
    }
    let img = image::open(path).map_err(|e| format!("cannot open {shown}: {e}"))?;
    convert_pixels(&flatten_alpha(&img.to_rgba8()), mode, rows, colors)
        .map_err(|e| format!("{shown}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite::BLANK;

    const RED: Rgb = [255, 0, 0];
    const BLUE: Rgb = [0, 0, 255];

    /// 4x4, red top-left pixel, blue bottom-right pixel, everything else
    /// fully transparent.
    fn tiny_rgba() -> image::RgbaImage {
        let mut img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(3, 3, image::Rgba([0, 0, 255, 255]));
        img
    }

    fn blanks(n: usize) -> String {
        std::iter::repeat_n(BLANK, n).collect()
    }

    #[test]
    fn white_and_luminance() {
        assert!(is_white([226, 226, 226]));
        assert!(!is_white([225, 255, 255]));
        assert!((lum([255, 255, 255]) - 1.0).abs() < 1e-9);
        assert!((lum(RED) - 0.299).abs() < 1e-9);
    }

    #[test]
    fn alpha_is_flattened_over_white() {
        let mut img = image::RgbaImage::from_pixel(2, 1, image::Rgba([0, 0, 0, 0]));
        img.put_pixel(1, 0, image::Rgba([0, 0, 0, 128]));
        let p = flatten_alpha(&img);
        assert_eq!(p.data[0], [255, 255, 255]);
        assert_eq!(p.data[1], [127, 127, 127]);
    }

    #[test]
    fn crop_keeps_only_content() {
        let mut data = vec![[255, 255, 255]; 5 * 4];
        data[5 + 1] = RED; // (1, 1)
        data[2 * 5 + 3] = BLUE; // (3, 2)
        let p = Pixels {
            width: 5,
            height: 4,
            data,
        };
        let c = crop_to_content(&p).unwrap();
        assert_eq!((c.width, c.height), (3, 2));
        assert_eq!(c.at(0, 0), RED);
        assert_eq!(c.at(2, 1), BLUE);
        let white = Pixels {
            width: 2,
            height: 2,
            data: vec![[255, 255, 255]; 4],
        };
        assert!(crop_to_content(&white).is_none());
    }

    #[test]
    fn vivid_matches_python_colorsys_reference() {
        // Expected values computed with reference_algorithms.vivid().
        assert_eq!(vivid([38, 22, 10]), [145, 72, 17]);
        assert_eq!(vivid([252, 200, 36]), [254, 208, 61]);
        assert_eq!(vivid([128, 128, 128]), [140, 140, 140]);
        assert_eq!(vivid(BLUE), [25, 25, 255]);
        assert_eq!(vivid(RED), [255, 25, 25]);
        assert_eq!(vivid([10, 200, 90]), [0, 231, 97]);
    }

    #[test]
    fn quantize_is_deterministic_and_bounded() {
        let data: Vec<Rgb> = (0..64u32)
            .map(|i| [(i * 4) as u8, (255 - i * 3) as u8, ((i * 37) % 256) as u8])
            .collect();
        let p = Pixels {
            width: 8,
            height: 8,
            data,
        };
        let a = quantize(&p, 5);
        let b = quantize(&p, 5);
        assert_eq!(a, b);
        let mut distinct = a.data.clone();
        distinct.sort();
        distinct.dedup();
        assert!(
            distinct.len() <= 5 && distinct.len() >= 2,
            "{}",
            distinct.len()
        );
        // Few colors already: unchanged.
        let few = Pixels {
            width: 3,
            height: 1,
            data: vec![RED, BLUE, RED],
        };
        assert_eq!(quantize(&few, 14), few);
    }

    #[test]
    fn quantize_matches_pillow_max_coverage() {
        // Expected mapping verified with Pillow 11:
        // img.quantize(colors=3, method=Image.Quantize.MAXCOVERAGE).
        let white = [255, 255, 255];
        let light = [240, 240, 240];
        let dark = [10, 20, 30];
        let red = [200, 40, 40];
        let green = [40, 200, 40];
        let blue = [40, 40, 200];
        let grey = [120, 120, 120];
        let mut data = Vec::new();
        for (c, n) in [
            (white, 50),
            (light, 5),
            (dark, 10),
            (red, 3),
            (green, 3),
            (blue, 3),
            (grey, 1),
        ] {
            data.extend(std::iter::repeat_n(c, n));
        }
        let p = Pixels {
            width: data.len(),
            height: 1,
            data,
        };
        let q = quantize(&p, 3);
        let mapped = |c: Rgb| q.data[p.data.iter().position(|x| *x == c).unwrap()];
        assert_eq!(mapped(white), white);
        assert_eq!(mapped(light), white);
        assert_eq!(mapped(dark), dark);
        assert_eq!(mapped(red), red);
        assert_eq!(mapped(green), dark);
        assert_eq!(mapped(blue), dark);
        assert_eq!(mapped(grey), red);
    }

    #[test]
    fn half_blocks_cover_all_four_cell_kinds() {
        let a = [1, 2, 3];
        let b = [4, 5, 6];
        let grid = vec![
            vec![Some(a), None, Some(a), None],
            vec![Some(b), Some(b), None, None],
            vec![None, Some(a), None, None],
        ];
        let rows = half_blocks(&grid);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            format!(
                "\x1b[38;2;1;2;3;48;2;4;5;6m▀\x1b[0m\x1b[38;2;4;5;6m▄\x1b[0m\x1b[38;2;1;2;3m▀\x1b[0m{BLANK}"
            )
        );
        // Odd height: the missing bottom row is transparent.
        assert_eq!(
            rows[1],
            format!("{BLANK}\x1b[38;2;1;2;3m▀\x1b[0m{}", blanks(2))
        );
    }

    #[test]
    fn pixel_mode_on_tiny_transparent_image() {
        let p = flatten_alpha(&tiny_rgba());
        let sprite = convert_pixels(&p, Mode::Pixel, 2, 14).unwrap();
        // Same output as reference_algorithms.half_blocks(pixel_downscale(...)).
        assert_eq!(sprite.width, 4);
        assert_eq!(
            sprite.rows,
            vec![
                format!("\x1b[38;2;255;0;0m▀\x1b[0m{}", blanks(3)),
                format!("{}\x1b[38;2;0;0;255m▄\x1b[0m", blanks(3)),
            ]
        );
    }

    #[test]
    fn ascii_mode_on_tiny_transparent_image() {
        let p = flatten_alpha(&tiny_rgba());
        let sprite = convert_pixels(&p, Mode::Ascii, 2, 14).unwrap();
        // Same output as reference_algorithms.ascii_art(..., rows=2).
        assert_eq!(sprite.width, 4);
        assert_eq!(
            sprite.rows,
            vec![
                format!("\x1b[1;38;2;255;25;25mh\x1b[0m{}", blanks(3)),
                format!("{}\x1b[1;38;2;25;25;255mh\x1b[0m", blanks(3)),
            ]
        );
    }

    #[test]
    fn downscale_upsamples_tiny_images_without_empty_blocks() {
        let p = Pixels {
            width: 1,
            height: 1,
            data: vec![RED],
        };
        let grid = pixel_downscale(&p, 4, 0.35);
        assert_eq!(grid.len(), 4);
        assert!(grid.iter().all(|row| row.iter().all(|c| *c == Some(RED))));
    }

    #[test]
    fn all_white_image_is_an_error() {
        let white = Pixels {
            width: 3,
            height: 3,
            data: vec![[255, 255, 255]; 9],
        };
        assert!(convert_pixels(&white, Mode::Pixel, 4, 14).is_err());
        assert!(convert_pixels(&white, Mode::Ascii, 4, 14).is_err());
    }

    #[test]
    fn convert_file_reads_png_and_text_grids() {
        let dir = std::env::temp_dir().join(format!(
            "arcade-convert-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("tiny.png");
        tiny_rgba().save(&png).unwrap();
        let sprite = convert_file(&png, Mode::Pixel, 2, 14).unwrap();
        assert_eq!(sprite.rows.len(), 2);

        let txt = dir.join("hero.txt");
        std::fs::write(&txt, "A 9 8 7\n---\nA.\n.A\n").unwrap();
        let hero = convert_file(&txt, Mode::Ascii, 99, 14).unwrap();
        assert_eq!(hero.width, 2);
        // Text grids are drawn pixel-exact as half blocks, whatever the mode.
        assert_eq!(
            hero.rows,
            ["\x1b[38;2;9;8;7m▀\x1b[0m\x1b[38;2;9;8;7m▄\x1b[0m"]
        );

        assert!(convert_file(&dir.join("missing.png"), Mode::Pixel, 2, 14).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
