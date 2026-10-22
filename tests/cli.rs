//! End-to-end tests of the `arcade-statusline` binary. Every run uses a
//! temporary HOME, so the real ~/.claude/settings.json is never touched.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const BIN: &str = env!("CARGO_BIN_EXE_arcade-statusline");

struct TempHome(PathBuf);

impl TempHome {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("arcade-cli-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempHome(dir)
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(home: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ARCADE_STATUSLINE_SPRITE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn sample(dir: &Path) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!(
        r#"{{"model":{{"display_name":"Opus"}},"workspace":{{"current_dir":{}}},
            "cost":{{"total_cost_usd":1.23,"total_duration_ms":125000}},
            "context_window":{{"used_percentage":42,"context_window_size":200000,"total_input_tokens":84000}},
            "rate_limits":{{"five_hour":{{"used_percentage":63,"resets_at":{}}},
                            "seven_day":{{"used_percentage":85,"resets_at":{}}}}}}}"#,
        serde_json::to_string(&dir.to_string_lossy()).unwrap(),
        now + 8000,
        now + 300_000
    )
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn assert_no_leading_ascii_whitespace(stdout: &str) {
    for row in stdout.lines() {
        assert!(
            !row.starts_with(|c: char| c.is_ascii_whitespace()),
            "raw row starts with whitespace: {row:?}"
        );
        assert!(
            !strip_ansi(row).starts_with(|c: char| c.is_ascii_whitespace()),
            "visible row starts with whitespace: {row:?}"
        );
    }
}

#[test]
fn renders_default_sprite_with_centered_hud() {
    let home = TempHome::new("render");
    let out = run(&home.0, &[], &sample(&home.0));
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_no_leading_ascii_whitespace(&stdout);
    let rows: Vec<String> = stdout.lines().map(strip_ansi).collect();
    // Default soldier: 9 rows; 5 HUD rows centered at offset 2.
    assert_eq!(rows.len(), 9);
    assert!(rows[2].contains("  1UP ★ Opus  STAGE arcade-cli-render"));
    assert!(!rows[2].contains("MISSION"), "temp dir is not a git repo");
    assert!(rows[3].contains("♥ HP ▰▰▰▰▰▰▱▱▱▱ 58% (84k/200k)"));
    assert!(rows[4].contains("▸ ARMS ▮▮▯▯▯▯ 37% RELOAD 2h13m"));
    assert!(rows[5].contains("● BOMB ▮▯▯▯▯▯ 15% RELOAD 3d11h"));
    assert!(rows[6].ends_with("  SCORE 000123  TIME 02:05"));
}

#[test]
fn render_subcommand_and_no_sprite_give_compact_layout() {
    let home = TempHome::new("compact");
    for args in [&["--no-sprite"][..], &["render", "--no-sprite"][..]] {
        let out = run(&home.0, args, &sample(&home.0));
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_no_leading_ascii_whitespace(&stdout);
        let rows: Vec<String> = stdout.lines().map(strip_ansi).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows[0].starts_with("1UP ★ Opus"));
        assert!(rows[1].contains(" | ▸ ARMS "));
    }
}

#[test]
fn sprite_flag_and_config_file_are_used() {
    let home = TempHome::new("sprite-file");
    let file = home.0.join("mine.ansi");
    std::fs::write(&file, "#width 2\nAB\n").unwrap();
    let out = run(&home.0, &["--sprite", file.to_str().unwrap()], "{}");
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "AB\n");

    let config = home.0.join(".config/arcade-statusline/sprite.ansi");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "CD\n").unwrap();
    let out = run(&home.0, &[], "{}");
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "CD\n");
}

#[test]
fn garbage_input_never_prints_errors() {
    let home = TempHome::new("garbage");
    for input in [
        "",
        "not json",
        "[1,2]",
        r#"{"model":null,"rate_limits":"x"}"#,
    ] {
        let out = run(&home.0, &["--no-sprite"], input);
        assert!(out.status.success());
        assert!(out.stdout.is_empty(), "{input}");
        assert!(out.stderr.is_empty(), "{input}");
    }
}

#[test]
fn install_dry_run_then_install_and_uninstall_round_trip() {
    let home = TempHome::new("install");
    let settings = home.0.join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, "{\"theme\": \"dark\"}\n").unwrap();

    let out = run(&home.0, &["install", "--dry-run"], "");
    assert!(out.status.success());
    let report = String::from_utf8(out.stdout).unwrap();
    assert!(report.contains("+ \"statusLine\""), "{report}");
    assert!(report.contains("arcade-statusline"));
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "{\"theme\": \"dark\"}\n"
    );
    assert_eq!(
        std::fs::read_dir(settings.parent().unwrap())
            .unwrap()
            .count(),
        1
    );

    let out = run(&home.0, &["install"], "");
    assert!(out.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(json["theme"], "dark");
    assert_eq!(json["statusLine"]["type"], "command");
    assert_eq!(json["statusLine"]["refreshInterval"], 1);
    let command = json["statusLine"]["command"].as_str().unwrap();
    assert!(
        Path::new(command.trim_matches('\'')).is_absolute(),
        "{command}"
    );

    let out = run(&home.0, &["uninstall"], "");
    assert!(out.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(json, serde_json::json!({"theme": "dark"}));
}

#[test]
fn sprite_command_converts_png_and_previews() {
    let home = TempHome::new("convert");
    let png = home.0.join("hero.png");
    let mut img = image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
    for y in 2..6 {
        for x in 2..6 {
            img.put_pixel(x, y, image::Rgba([200, 40, 40, 255]));
        }
    }
    img.save(&png).unwrap();

    let out_file = home.0.join("out.ansi");
    let out = run(
        &home.0,
        &[
            "sprite",
            png.to_str().unwrap(),
            "--rows",
            "2",
            "--out",
            out_file.to_str().unwrap(),
        ],
        "",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(&out_file).unwrap();
    assert!(text.starts_with("#width "));
    assert_eq!(text.lines().count(), 3);

    let out = run(
        &home.0,
        &[
            "sprite",
            png.to_str().unwrap(),
            "--mode",
            "ascii",
            "--rows",
            "3",
            "--preview",
        ],
        "",
    );
    assert!(out.status.success());
    let preview = String::from_utf8(out.stdout).unwrap();
    assert_eq!(preview.lines().count(), 3);
    assert_no_leading_ascii_whitespace(&preview);
    // --preview without --out writes nothing.
    assert!(!home.0.join(".config").exists());

    let out = run(
        &home.0,
        &["sprite", "/definitely/missing.png", "--preview"],
        "",
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot open"));
}
