//! Pure HUD rendering. No I/O: the caller injects "now" (epoch seconds) and
//! the git branch, so output is deterministic and testable.

use crate::input::{ContextWindow, Limit, StatusInput};
use crate::sprite::{BLANK, Sprite};

pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
/// Arcade yellow, used for labels.
pub const LABEL: &str = "\x1b[1;38;2;255;215;0m";
/// Arcade orange, used for values.
pub const ORANGE: &str = "\x1b[1;38;2;255;135;0m";
/// Player cyan, used for the model.
pub const CYAN: &str = "\x1b[1;38;2;0;229;255m";
pub const GREEN: &str = "\x1b[1;38;2;57;255;20m";
pub const YELLOW: &str = "\x1b[1;38;2;255;234;0m";
pub const RED: &str = "\x1b[1;38;2;255;49;49m";

/// The HUD rows before layout.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Hud {
    /// `1UP ★ model  STAGE dir  MISSION branch`
    pub header: Option<String>,
    /// HP, ARMS, BOMB and COIN, in that order, when present.
    pub segments: Vec<String>,
    /// `SCORE ...  TIME ...`
    pub tail: Option<String>,
}

impl Hud {
    /// Stacked rows: header, segments, tail.
    pub fn rows(&self) -> impl Iterator<Item = &str> {
        self.header
            .iter()
            .chain(&self.segments)
            .chain(&self.tail)
            .map(String::as_str)
    }
}

/// Bar color by USED percentage: green < 50, yellow 50..=79, red >= 80.
pub fn color_for(used: i64) -> &'static str {
    if used >= 80 {
        RED
    } else if used >= 50 {
        YELLOW
    } else {
        GREEN
    }
}

/// `printf '%.0f'` rounding (ties to even), bounded so later math never overflows.
fn percent(value: f64) -> i64 {
    value.round_ties_even().clamp(-1e6, 1e6) as i64
}

fn bar(filled: usize, cells: usize, full: char, empty: char) -> String {
    let filled = filled.min(cells);
    let mut s = String::with_capacity(cells * 3);
    s.extend(std::iter::repeat_n(full, filled));
    s.extend(std::iter::repeat_n(empty, cells - filled));
    s
}

/// Seconds until `resets_at` as `XdYh`, `XhMMm` or `Mm`; `now` once reached.
pub fn format_reset(resets_at: i64, now: i64) -> String {
    let s = resets_at.saturating_sub(now);
    if s <= 0 {
        return "now".to_string();
    }
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m:02}m")
    } else {
        format!("{m}m")
    }
}

/// Cost in cents, zero-padded to 6 digits: $1.23 -> `000123`.
pub fn format_score(cost_usd: f64) -> String {
    format!("{:06}", (cost_usd * 100.0).round() as i64)
}

/// `MM:SS`, or `H:MM:SS` from one hour on. Fractional milliseconds truncate.
pub fn format_time(duration_ms: f64) -> String {
    let ms = (duration_ms as i64).max(0);
    let (m, s) = (ms % 3_600_000 / 60_000, ms % 60_000 / 1000);
    if ms >= 3_600_000 {
        format!("{}:{m:02}:{s:02}", ms / 3_600_000)
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `♥ HP` context remaining: 10-cell bar, percentage and dimmed token counts.
pub fn hp_segment(cw: &ContextWindow) -> Option<String> {
    let used = percent(cw.used_percentage?);
    let remaining = (100 - used).clamp(0, 100);
    let filled = ((remaining + 5) / 10) as usize;
    let mut seg = format!(
        "{LABEL}♥ HP{RESET} {}{} {remaining}%{RESET}",
        color_for(used),
        bar(filled, 10, '▰', '▱')
    );
    if let (Some(tokens), Some(size)) = (cw.total_input_tokens, cw.context_window_size) {
        let (tokens, size) = (tokens as i64 / 1000, size as i64 / 1000);
        seg.push_str(&format!(" {DIM}({tokens}k/{size}k){RESET}"));
    }
    Some(seg)
}

/// A rate limit as a 6-cell ammo counter of what REMAINS, plus a dim reload timer.
pub fn limit_segment(label: &str, limit: &Limit, now: i64) -> Option<String> {
    let used = percent(limit.used_percentage?);
    let remaining = (100 - used).clamp(0, 100);
    let filled = ((remaining * 6 + 50) / 100) as usize;
    let mut seg = format!(
        "{LABEL}{label}{RESET} {}{} {remaining}%{RESET}",
        color_for(used),
        bar(filled, 6, '▮', '▯')
    );
    if let Some(at) = limit.resets_at {
        let reload = format_reset(at as i64, now);
        seg.push_str(&format!("{DIM} RELOAD {reload}{RESET}"));
    }
    Some(seg)
}

/// Drops control characters (newlines, escapes) from external text so it can
/// neither break the row layout nor inject terminal sequences.
fn clean(text: &str) -> std::borrow::Cow<'_, str> {
    if text.chars().any(char::is_control) {
        text.chars().filter(|c| !c.is_control()).collect()
    } else {
        text.into()
    }
}

fn basename(dir: &str) -> &str {
    dir.rsplit(['/', '\\']).next().unwrap_or(dir)
}

fn header(model: Option<&str>, dir: Option<&str>, branch: Option<&str>) -> Option<String> {
    if model.is_none() && dir.is_none() {
        return None;
    }
    let mut line = format!("{LABEL}1UP{RESET}");
    if let Some(model) = model {
        line.push_str(&format!(" {CYAN}★ {}{RESET}", clean(model)));
    }
    if let Some(dir) = dir {
        line.push_str(&format!("  {ORANGE}STAGE{RESET} {}", clean(basename(dir))));
        if let Some(branch) = branch.filter(|b| !b.is_empty()) {
            line.push_str(&format!("  {ORANGE}MISSION{RESET} {}", clean(branch)));
        }
    }
    Some(line)
}

fn tail(input: &StatusInput) -> Option<String> {
    let cost = input.cost.as_ref()?;
    let score = cost
        .total_cost_usd
        .map(|c| format!("{LABEL}SCORE{RESET} {ORANGE}{}{RESET}", format_score(c)));
    let time = cost
        .total_duration_ms
        .map(|d| format!("{LABEL}TIME{RESET} {ORANGE}{}{RESET}", format_time(d)));
    match (score, time) {
        (Some(score), Some(time)) => Some(format!("{score}  {time}")),
        (score, time) => score.or(time),
    }
}

pub fn build(input: &StatusInput, branch: Option<&str>, now: i64) -> Hud {
    let mut segments = Vec::with_capacity(4);
    if let Some(cw) = &input.context_window {
        segments.extend(hp_segment(cw));
    }
    if let Some(limits) = &input.rate_limits {
        let named = [
            ("▸ ARMS", &limits.five_hour),
            ("● BOMB", &limits.seven_day),
            ("¢ COIN", &limits.spend_limit),
        ];
        for (label, limit) in named {
            if let Some(limit) = limit {
                segments.extend(limit_segment(label, limit, now));
            }
        }
    }
    Hud {
        header: header(input.model_name(), input.current_dir(), branch),
        segments,
        tail: tail(input),
    }
}

/// Ends the row that starts at byte `start`. Leading ASCII whitespace would be
/// trimmed by Claude Code and skew the drawing, so it becomes braille blanks.
fn end_row(out: &mut String, start: usize) {
    let leading = out[start..]
        .bytes()
        .take_while(u8::is_ascii_whitespace)
        .count();
    if leading > 0 {
        let blanks: String = std::iter::repeat_n(BLANK, leading).collect();
        out.replace_range(start..start + leading, &blanks);
    }
    out.push('\n');
}

/// Lays the HUD out next to the sprite (or in the compact 2-line form).
pub fn compose(hud: &Hud, sprite: Option<&Sprite>) -> String {
    let mut out = String::with_capacity(2048);
    let rows: Vec<&str> = hud.rows().collect();
    match sprite.filter(|s| s.width > 0 && !s.rows.is_empty()) {
        Some(sprite) => {
            let height = sprite.rows.len().max(rows.len());
            // Center the HUD vertically against the sprite.
            let offset = sprite.rows.len().saturating_sub(rows.len()) / 2;
            for i in 0..height {
                let start = out.len();
                match sprite.rows.get(i) {
                    Some(row) => out.push_str(row),
                    None => out.extend(std::iter::repeat_n(BLANK, sprite.width)),
                }
                if let Some(row) = i.checked_sub(offset).and_then(|k| rows.get(k)) {
                    out.push_str("  ");
                    out.push_str(row);
                }
                end_row(&mut out, start);
            }
        }
        None => {
            if let Some(header) = &hud.header {
                let start = out.len();
                out.push_str(header);
                end_row(&mut out, start);
            }
            let start = out.len();
            for (i, seg) in hud.segments.iter().chain(&hud.tail).enumerate() {
                if i > 0 {
                    out.push_str(" | ");
                }
                out.push_str(seg);
            }
            if out.len() > start {
                end_row(&mut out, start);
            }
        }
    }
    out
}

/// Full pipeline: input -> HUD -> laid out text, one row per line.
pub fn render(
    input: &StatusInput,
    branch: Option<&str>,
    now: i64,
    sprite: Option<&Sprite>,
) -> String {
    compose(&build(input, branch, now), sprite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite::{BLANK, strip_ansi};

    const NOW: i64 = 1_800_000_000;

    fn sample(now: i64) -> StatusInput {
        StatusInput::parse(&format!(
            r#"{{"model":{{"display_name":"Opus"}},"workspace":{{"current_dir":"/Users/silver"}},
                "cost":{{"total_cost_usd":1.23,"total_duration_ms":125000}},
                "context_window":{{"used_percentage":42,"context_window_size":200000,"total_input_tokens":84000}},
                "rate_limits":{{"five_hour":{{"used_percentage":63,"resets_at":{}}},
                                "seven_day":{{"used_percentage":85,"resets_at":{}}}}}}}"#,
            now + 8000,
            now + 300_000
        ))
    }

    fn plain(text: &str) -> Vec<String> {
        text.lines().map(strip_ansi).collect()
    }

    fn limit(used: f64, resets_at: Option<f64>) -> Limit {
        Limit {
            used_percentage: Some(used),
            resets_at,
        }
    }

    #[test]
    fn color_thresholds_follow_used_percentage() {
        assert_eq!(color_for(0), GREEN);
        assert_eq!(color_for(49), GREEN);
        assert_eq!(color_for(50), YELLOW);
        assert_eq!(color_for(79), YELLOW);
        assert_eq!(color_for(80), RED);
        assert_eq!(color_for(150), RED);
    }

    #[test]
    fn reset_formats() {
        assert_eq!(format_reset(NOW + 300_000, NOW), "3d11h");
        assert_eq!(format_reset(NOW + 8000, NOW), "2h13m");
        assert_eq!(format_reset(NOW + 3600, NOW), "1h00m");
        assert_eq!(format_reset(NOW + 45 * 60 + 5, NOW), "45m");
        assert_eq!(format_reset(NOW + 59, NOW), "0m");
        assert_eq!(format_reset(NOW, NOW), "now");
        assert_eq!(format_reset(NOW - 10, NOW), "now");
    }

    #[test]
    fn score_is_zero_padded_cents() {
        assert_eq!(format_score(1.23), "000123");
        assert_eq!(format_score(0.0), "000000");
        assert_eq!(format_score(0.005), "000001");
        assert_eq!(format_score(12345.678), "1234568");
    }

    #[test]
    fn time_switches_to_hours_at_one_hour() {
        assert_eq!(format_time(0.0), "00:00");
        assert_eq!(format_time(125_000.0), "02:05");
        assert_eq!(format_time(3_599_999.0), "59:59");
        assert_eq!(format_time(3_600_000.0), "1:00:00");
        assert_eq!(format_time(3_725_000.9), "1:02:05");
        assert_eq!(format_time(-5.0), "00:00");
    }

    #[test]
    fn hp_bar_shows_remaining_context() {
        let cw = ContextWindow {
            used_percentage: Some(42.0),
            context_window_size: Some(200_000.0),
            total_input_tokens: Some(84_000.0),
        };
        let seg = hp_segment(&cw).unwrap();
        assert_eq!(strip_ansi(&seg), "♥ HP ▰▰▰▰▰▰▱▱▱▱ 58% (84k/200k)");
        assert!(seg.contains(&format!("{GREEN}▰▰▰▰▰▰▱▱▱▱ 58%")));
        assert!(seg.contains(&format!("{DIM}(84k/200k){RESET}")));
    }

    #[test]
    fn hp_bar_clamps_and_hides_tokens_when_partial() {
        let full = ContextWindow {
            used_percentage: Some(0.0),
            ..Default::default()
        };
        assert_eq!(
            strip_ansi(&hp_segment(&full).unwrap()),
            "♥ HP ▰▰▰▰▰▰▰▰▰▰ 100%"
        );
        let over = ContextWindow {
            used_percentage: Some(130.0),
            total_input_tokens: Some(1.0),
            ..Default::default()
        };
        let seg = hp_segment(&over).unwrap();
        assert_eq!(strip_ansi(&seg), "♥ HP ▱▱▱▱▱▱▱▱▱▱ 0%");
        assert!(seg.contains(RED));
        assert!(hp_segment(&ContextWindow::default()).is_none());
    }

    #[test]
    fn limit_segment_is_an_ammo_counter_of_what_remains() {
        let arms = limit_segment("▸ ARMS", &limit(63.0, Some((NOW + 8000) as f64)), NOW).unwrap();
        assert_eq!(strip_ansi(&arms), "▸ ARMS ▮▮▯▯▯▯ 37% RELOAD 2h13m");
        assert!(arms.starts_with(&format!("{LABEL}▸ ARMS{RESET}")));
        assert!(arms.contains(&format!("{YELLOW}▮▮▯▯▯▯ 37%{RESET}")));
        assert!(arms.contains(&format!("{DIM} RELOAD 2h13m{RESET}")));

        let bomb = limit_segment("● BOMB", &limit(85.0, None), NOW).unwrap();
        assert_eq!(strip_ansi(&bomb), "● BOMB ▮▯▯▯▯▯ 15%");
        assert!(bomb.contains(RED));

        let fresh = limit_segment("¢ COIN", &limit(10.0, Some(1.0)), NOW).unwrap();
        assert_eq!(strip_ansi(&fresh), "¢ COIN ▮▮▮▮▮▯ 90% RELOAD now");
        assert!(fresh.contains(GREEN));

        let empty = Limit::default();
        assert!(limit_segment("▸ ARMS", &empty, NOW).is_none());
    }

    #[test]
    fn printf_style_rounding_ties_to_even() {
        // printf '%.0f' 62.5 -> 62, so 38% remains.
        let seg = limit_segment("▸ ARMS", &limit(62.5, None), NOW).unwrap();
        assert!(strip_ansi(&seg).ends_with(" 38%"));
    }

    #[test]
    fn header_shows_model_stage_and_mission() {
        let hud = build(&sample(NOW), Some("main"), NOW);
        assert_eq!(
            strip_ansi(hud.header.as_deref().unwrap()),
            "1UP ★ Opus  STAGE silver  MISSION main"
        );
        let header = hud.header.unwrap();
        assert!(header.contains(&format!("{CYAN}★ Opus{RESET}")));
        assert!(header.starts_with(&format!("{LABEL}1UP{RESET}")));
    }

    #[test]
    fn full_compact_layout_has_two_lines() {
        let out = render(&sample(NOW), Some("main"), NOW, None);
        assert_eq!(
            plain(&out),
            vec![
                "1UP ★ Opus  STAGE silver  MISSION main".to_string(),
                "♥ HP ▰▰▰▰▰▰▱▱▱▱ 58% (84k/200k) | ▸ ARMS ▮▮▯▯▯▯ 37% RELOAD 2h13m | \
                 ● BOMB ▮▯▯▯▯▯ 15% RELOAD 3d11h | SCORE 000123  TIME 02:05"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn missing_fields_hide_their_segments() {
        assert_eq!(
            render(&StatusInput::parse("{}"), Some("main"), NOW, None),
            ""
        );
        assert_eq!(render(&StatusInput::parse("garbage"), None, NOW, None), "");

        let model_only = StatusInput::parse(r#"{"model":{"display_name":"Opus"}}"#);
        assert_eq!(
            plain(&render(&model_only, Some("main"), NOW, None)),
            ["1UP ★ Opus"]
        );

        let dir_only = StatusInput::parse(r#"{"workspace":{"current_dir":"/a/proj"}}"#);
        assert_eq!(
            plain(&render(&dir_only, None, NOW, None)),
            ["1UP  STAGE proj"]
        );
        assert_eq!(
            plain(&render(&dir_only, Some(""), NOW, None)),
            ["1UP  STAGE proj"]
        );

        let cost_only = StatusInput::parse(r#"{"cost":{"total_cost_usd":0.5}}"#);
        assert_eq!(
            plain(&render(&cost_only, None, NOW, None)),
            ["SCORE 000050"]
        );

        let time_only = StatusInput::parse(r#"{"cost":{"total_duration_ms":61000}}"#);
        assert_eq!(plain(&render(&time_only, None, NOW, None)), ["TIME 01:01"]);

        let null_limits = StatusInput::parse(
            r#"{"rate_limits":{"five_hour":null,"seven_day":{"used_percentage":null},
                "spend_limit":{"used_percentage":20}}}"#,
        );
        assert_eq!(
            plain(&render(&null_limits, None, NOW, None)),
            ["¢ COIN ▮▮▮▮▮▯ 80%"]
        );
    }

    #[test]
    fn coin_only_when_spend_limit_present() {
        let out = render(&sample(NOW), None, NOW, None);
        assert!(!strip_ansi(&out).contains("COIN"));
        let with_coin = StatusInput::parse(&format!(
            r#"{{"rate_limits":{{"spend_limit":{{"used_percentage":55,"resets_at":{}}}}}}}"#,
            NOW + 120
        ));
        assert_eq!(
            plain(&render(&with_coin, None, NOW, None)),
            ["¢ COIN ▮▮▮▯▯▯ 45% RELOAD 2m"]
        );
    }

    fn sprite_of(rows: usize, width: usize) -> Sprite {
        Sprite {
            width,
            rows: (0..rows).map(|i| format!("{i}").repeat(width)).collect(),
        }
    }

    #[test]
    fn hud_is_centered_vertically_against_the_sprite() {
        let sprite = sprite_of(9, 3);
        let out = plain(&render(&sample(NOW), Some("main"), NOW, Some(&sprite)));
        // 5 HUD rows against 9 sprite rows: offset (9 - 5) / 2 = 2.
        assert_eq!(out.len(), 9);
        assert_eq!(out[0], "000");
        assert_eq!(out[1], "111");
        assert_eq!(out[2], "222  1UP ★ Opus  STAGE silver  MISSION main");
        assert!(out[3].starts_with("333  ♥ HP"));
        assert!(out[4].starts_with("444  ▸ ARMS"));
        assert!(out[5].starts_with("555  ● BOMB"));
        assert_eq!(out[6], "666  SCORE 000123  TIME 02:05");
        assert_eq!(out[7], "777");
        assert_eq!(out[8], "888");
    }

    #[test]
    fn sprite_shorter_than_hud_is_padded_with_braille_blanks() {
        let sprite = sprite_of(2, 3);
        let out = plain(&render(&sample(NOW), Some("main"), NOW, Some(&sprite)));
        assert_eq!(out.len(), 5);
        assert!(out[0].starts_with("000  1UP"));
        let pad: String = std::iter::repeat_n(BLANK, 3).collect();
        assert_eq!(out[4], format!("{pad}  SCORE 000123  TIME 02:05"));
    }

    #[test]
    fn empty_hud_with_sprite_prints_only_the_sprite() {
        let sprite = sprite_of(2, 1);
        assert_eq!(
            render(&StatusInput::default(), None, NOW, Some(&sprite)),
            "0\n1\n"
        );
    }

    #[test]
    fn no_row_ever_starts_with_ascii_whitespace() {
        let inputs = [
            sample(NOW),
            StatusInput::default(),
            StatusInput::parse(r#"{"cost":{"total_cost_usd":0.1}}"#),
            StatusInput::parse(r#"{"workspace":{"current_dir":"/x"}}"#),
        ];
        let sprites = [
            None,
            Some(Sprite::default_soldier()),
            Sprite::parse("  a\n\n   \n"),
            Sprite::parse("x\n"),
        ];
        let mut rows_checked = 0;
        for input in &inputs {
            for sprite in &sprites {
                let out = render(input, Some("dev"), NOW, sprite.as_ref());
                for row in out.lines() {
                    rows_checked += 1;
                    let first_raw = row.chars().next().unwrap();
                    assert!(
                        !first_raw.is_ascii_whitespace(),
                        "raw row starts with whitespace: {row:?}"
                    );
                    let visible = strip_ansi(row);
                    let first_visible = visible.chars().next().unwrap();
                    assert!(
                        !first_visible.is_ascii_whitespace(),
                        "visible row starts with whitespace: {row:?}"
                    );
                }
            }
        }
        assert!(rows_checked > 30);
    }
}
