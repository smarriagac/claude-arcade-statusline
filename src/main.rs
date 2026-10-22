//! `arcade-statusline`: an 8-bit arcade-style HUD for the Claude Code status line.

mod convert;
mod hud;
mod input;
mod install;
mod sprite;

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::convert::Mode;
use crate::input::StatusInput;
use crate::sprite::Sprite;

/// Environment variable naming a sprite file (second in the resolution order).
const SPRITE_ENV: &str = "ARCADE_STATUSLINE_SPRITE";

#[derive(Parser)]
#[command(
    name = "arcade-statusline",
    version,
    about = "8-bit arcade-style HUD for the Claude Code status line",
    long_about = "8-bit arcade-style HUD for the Claude Code status line.\n\n\
        With no subcommand it reads Claude Code's status JSON on stdin and prints the HUD.",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    #[command(flatten)]
    render: RenderArgs,
}

#[derive(Subcommand)]
enum Cmd {
    /// Read the status JSON on stdin and print the HUD (the default).
    Render(RenderArgs),
    /// Convert an image (PNG/JPG/GIF) or a text pixel grid (.txt) into a sprite file.
    Sprite(SpriteArgs),
    /// Point Claude Code's statusLine at this binary (~/.claude/settings.json).
    Install(InstallArgs),
    /// Restore the previous statusLine from the latest backup, or remove it.
    Uninstall,
}

#[derive(Args, Default)]
struct RenderArgs {
    /// Sprite file to draw (overrides $ARCADE_STATUSLINE_SPRITE and the config file).
    #[arg(long, value_name = "FILE")]
    sprite: Option<PathBuf>,
    /// Draw no sprite: compact 2-line HUD.
    #[arg(long, conflicts_with = "sprite")]
    no_sprite: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    /// One colored character per cell.
    Ascii,
    /// Half-block pixels, two vertical pixels per cell.
    Pixel,
}

#[derive(Args)]
struct SpriteArgs {
    /// Source image (PNG, JPG, GIF) or a text pixel grid (.txt).
    image: PathBuf,
    /// Conversion mode.
    #[arg(long, value_enum, default_value_t = ModeArg::Pixel)]
    mode: ModeArg,
    /// Text rows of the sprite [default: 12 for ascii, 10 for pixel].
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=200))]
    rows: Option<u32>,
    /// Palette size for ascii mode.
    #[arg(long, default_value_t = 14, value_parser = clap::value_parser!(u32).range(1..=256))]
    colors: u32,
    /// Output file [default: ~/.config/arcade-statusline/sprite.ansi].
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Print the sprite to stdout. Without --out, nothing is written.
    #[arg(long)]
    preview: bool,
}

#[derive(Args)]
struct InstallArgs {
    /// Show what would change without writing anything.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None => render(&cli.render),
        Some(Cmd::Render(args)) => render(&args),
        Some(Cmd::Sprite(args)) => report(convert_sprite(&args)),
        Some(Cmd::Install(args)) => report(run_install(args.dry_run)),
        Some(Cmd::Uninstall) => report(run_uninstall()),
    }
}

fn report(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("arcade-statusline: {err}");
            ExitCode::FAILURE
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The status line path: never prints errors, always exits 0.
fn render(args: &RenderArgs) -> ExitCode {
    let mut text = String::new();
    let _ = io::stdin().read_to_string(&mut text);
    let input = StatusInput::parse(&text);

    let branch = input.current_dir().and_then(git_branch);
    let sprite = if args.no_sprite {
        None
    } else {
        let env: Option<OsString> = std::env::var_os(SPRITE_ENV).filter(|v| !v.is_empty());
        Some(sprite::resolve(
            args.sprite.as_deref(),
            env.as_deref(),
            home_dir().as_deref(),
        ))
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);

    let out = hud::render(&input, branch.as_deref(), now, sprite.as_ref());
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

/// `git --no-optional-locks -C <dir> branch --show-current`; `None` on any failure.
fn git_branch(dir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["--no-optional-locks", "-C", dir, "branch", "--show-current"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8(output.stdout).ok()?;
    let branch = branch.trim_end_matches(['\n', '\r']);
    (!branch.is_empty()).then(|| branch.to_string())
}

fn convert_sprite(args: &SpriteArgs) -> Result<String, String> {
    let mode = match args.mode {
        ModeArg::Ascii => Mode::Ascii,
        ModeArg::Pixel => Mode::Pixel,
    };
    let rows = args.rows.map_or(mode.default_rows(), |r| r as usize);
    let sprite = convert::convert_file(&args.image, mode, rows, args.colors as usize)?;

    let mut text = String::new();
    if args.preview {
        for row in &sprite.rows {
            text.push_str(row);
            text.push('\n');
        }
    }
    let target = match (&args.out, args.preview) {
        (Some(out), _) => Some(out.clone()),
        (None, true) => None,
        (None, false) => Some(sprite::default_path(
            &home_dir().ok_or("cannot find the home directory; pass --out")?,
        )),
    };
    if let Some(path) = target {
        write_sprite(&path, &sprite)?;
        text.push_str(&format!(
            "wrote {}x{} sprite to {}\n",
            sprite.width,
            sprite.rows.len(),
            path.display()
        ));
    }
    Ok(text)
}

fn write_sprite(path: &Path, sprite: &Sprite) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, sprite.to_file_string())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn run_install(dry_run: bool) -> Result<String, String> {
    let home = home_dir().ok_or("cannot find the home directory")?;
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate this binary: {e}"))?;
    let exe = std::path::absolute(&exe).unwrap_or(exe);
    let command = install::shell_command(&exe);
    let outcome = install::install(&home, &command, dry_run, SystemTime::now())?;
    Ok(install::describe(&outcome, dry_run))
}

fn run_uninstall() -> Result<String, String> {
    let home = home_dir().ok_or("cannot find the home directory")?;
    let outcome = install::uninstall(&home, SystemTime::now())?;
    Ok(install::describe(&outcome, false))
}
