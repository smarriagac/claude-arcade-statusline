# claude-arcade-statusline

An 8-bit arcade HUD for the [Claude Code](https://docs.claude.com/en/docs/claude-code) status line.
It turns your context window into a health bar and your rate limits into ammo counters. Session cost
becomes your score, and a pixel-art character stands next to it all. Inspired by classic 90s arcade HUDs.

It ships as one small Rust binary, `arcade-statusline`. It makes no network calls, renders in a few
milliseconds and hides any field Claude Code does not send.

```text
⠀⠀▄▀▀▀▀▀▀▄⠀⠀⠀⠀⠀⠀
⠀▀▀▀▀▀▀▀▀▀▀⠀⠀⠀⠀⠀
⠀▀▀▀▀▀▀▀▀▀▀⠀⠀⠀⠀⠀  1UP ★ Opus  STAGE silver  MISSION main
⠀▀▀▀▀▀▀▀▀▀▀⠀⠀⠀⠀⠀  ♥ HP ▰▰▰▰▰▰▱▱▱▱ 58% (84k/200k)
▄▀▀▀▀▀▀▀▀▀▀▄⠀⠀⠀⠀  ▸ ARMS ▮▮▯▯▯▯ 37% RELOAD 2h13m
▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀  ● BOMB ▮▯▯▯▯▯ 15% RELOAD 3d11h
⠀▀▀▄▀▀▀▀▀▄▀▀▀▀⠀⠀  SCORE 000123  TIME 02:05
⠀⠀⠀▀▀▀▀▀▀▀⠀⠀⠀⠀⠀⠀
⠀⠀▀▀▀▀▀▀▀▀▀⠀⠀⠀⠀⠀
```

*(Colors stripped. In a terminal, the sprite is a full-color soldier and the bars turn green,
yellow or red.)*

With `--no-sprite` you get the compact two-line layout:

```text
1UP ★ Opus  STAGE silver  MISSION main
♥ HP ▰▰▰▰▰▰▱▱▱▱ 58% (84k/200k) | ▸ ARMS ▮▮▯▯▯▯ 37% RELOAD 2h13m | ● BOMB ▮▯▯▯▯▯ 15% RELOAD 3d11h | SCORE 000123  TIME 02:05
```

## Quick start

1. Install the binary (needs a [Rust toolchain](https://rustup.rs)):

   ```sh
   cargo install --git https://github.com/<owner>/claude-arcade-statusline
   # or, from a local clone:
   cargo install --path .
   ```

2. Point Claude Code at it. Preview the change first, then apply it:

   ```sh
   arcade-statusline install --dry-run
   arcade-statusline install
   ```

3. Open Claude Code. The HUD appears in the status line.

`install` merges this entry into `~/.claude/settings.json` and leaves every other key and its order
as it was:

```json
"statusLine": {
  "type": "command",
  "command": "/absolute/path/to/arcade-statusline",
  "refreshInterval": 1
}
```

Before writing, it saves a timestamped backup next to the file, for example
`settings.json.arcade-backup-20261022-142233-120`. To undo it, run `arcade-statusline uninstall`.

## What the HUD shows

| Row | Source field | Rendering |
|-----|--------------|-----------|
| `1UP ★ <model>` | `model.display_name` | The model, in cyan |
| `STAGE <dir>` | `workspace.current_dir` | Basename of the working directory |
| `MISSION <branch>` | `git branch --show-current` | Hidden outside a git repository |
| `♥ HP` | `context_window.*` | Context **remaining**, 10-cell bar, `(used k / window k)` |
| `▸ ARMS` | `rate_limits.five_hour` | 5-hour limit **remaining**, 6-cell bar, `RELOAD` countdown |
| `● BOMB` | `rate_limits.seven_day` | Weekly limit, same as ARMS |
| `¢ COIN` | `rate_limits.spend_limit` | Spend limit, only when Claude Code sends it |
| `SCORE` | `cost.total_cost_usd` | Cost in cents, 6 digits: $1.23 → `000123` |
| `TIME` | `cost.total_duration_ms` | `MM:SS`, or `H:MM:SS` from one hour on |

| Detail | Behavior |
|--------|----------|
| Bar colors | Green while less than 50% is used, yellow from 50 to 79%, red from 80% |
| Reload times | `3d11h`, `2h13m` or `45m`; `now` once the reset time has passed |
| Missing data | Any missing, `null` or malformed field hides only its own segment |
| Errors | The render path never prints errors and always exits 0 |
| Layout | The sprite sits on the left; the HUD rows are centered vertically to its right |

## Commands

| Command | What it does |
|---------|--------------|
| `arcade-statusline` | Reads the status JSON on stdin and prints the HUD (same as `render`) |
| `arcade-statusline render [--sprite FILE] [--no-sprite]` | Same, with an explicit sprite or none |
| `arcade-statusline sprite IMAGE [--mode ascii\|pixel] [--rows N] [--out FILE] [--preview]` | Converts an image into a sprite file |
| `arcade-statusline install [--dry-run]` | Writes the `statusLine` entry, after a backup |
| `arcade-statusline uninstall` | Restores the `statusLine` you had before, or removes the key |

To try the HUD without Claude Code, pipe in a sample:

```sh
echo '{"model":{"display_name":"Opus"},"cost":{"total_cost_usd":1.23,"total_duration_ms":125000},
"context_window":{"used_percentage":42,"context_window_size":200000,"total_input_tokens":84000}}' \
  | arcade-statusline
```

## Sprites

The renderer looks for a sprite in this order and uses the first one it can read:

1. `--sprite <file>`
2. the `ARCADE_STATUSLINE_SPRITE` environment variable
3. `~/.config/arcade-statusline/sprite.ansi`
4. the embedded default soldier, an original character drawn for this project
   ([`assets/default-sprite.txt`](assets/default-sprite.txt))

### Convert an image

```sh
# Pixel mode (default): half blocks, two pixels per text cell. Best for pixel art.
arcade-statusline sprite hero.png --rows 9

# ASCII mode: one colored letter per cell, like a fastfetch logo. Best for logos.
arcade-statusline sprite logo.png --mode ascii --rows 12

# Preview only; nothing is written unless you also pass --out.
arcade-statusline sprite hero.png --preview
```

| Option | Default | Notes |
|--------|---------|-------|
| `--mode` | `pixel` | `pixel` or `ascii` |
| `--rows` | 10 (pixel), 12 (ascii) | Text rows. The HUD has at most 6 rows, so 6 to 10 rows looks best |
| `--colors` | 14 | ASCII mode palette size |
| `--out` | `~/.config/arcade-statusline/sprite.ansi` | Where the sprite file is written |
| `--preview` | off | Prints the sprite to stdout |

The converter reads PNG, JPG and GIF files. White and transparent pixels count as background and
come out see-through. The image is cropped to its content first, so extra margins do not matter.

### Make your own character

The easiest route is a **text pixel grid**. Copy
[`assets/default-sprite.txt`](assets/default-sprite.txt), edit it, then convert it:

```text
# my-hero.txt: palette lines are "<key> <r> <g> <b>", then "---", then the pixels.
K 20 20 30
R 230 60 50
S 240 190 140
---
..KKKK..
.KSSSSK.
.KSKSKK.
..KRRK..
.KRRRRK.
..K..K..
```

```sh
arcade-statusline sprite my-hero.txt --out ~/.config/arcade-statusline/sprite.ansi
```

- Every character is one pixel. `.` (or a space) is transparent; use `.` for an empty row.
- Two grid rows make one text row, so an 18-pixel-tall grid is 9 rows high.
- A text grid is drawn exactly as written; `--mode` and `--rows` do not apply.

To start from a drawing, use any pixel-art editor with a white or transparent background. Draw
small (about 16 x 18 pixels). Then convert with `--rows` set to half the height of the drawing
itself (`--rows 9` for 18 pixels), so every pixel maps to exactly one half block. Margins are
cropped before measuring.

### Sprite file format

A sprite file is plain text. Each line is one row of the sprite, with ANSI truecolor escape codes.
An optional first line, `#width N`, declares the width in cells. Rows shorter than the widest row
are padded, so the HUD always lines up.

> **Why braille blanks?** Claude Code trims leading ASCII whitespace from every status line row.
> That would shift a sprite whose first column is transparent. So transparent cells are written as
> U+2800 BRAILLE PATTERN BLANK, which renders as an empty cell but is not whitespace. Any ASCII
> space in a sprite file is converted the same way.

## Requirements

- A terminal with truecolor (24-bit) support. Most modern terminals have it.
- A font with the block (`▀ ▄`), bar (`▰ ▱ ▮ ▯`) and braille (`⠀`) glyphs.
- `git` on `PATH` for the `MISSION` segment (optional).

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

The code is split into small modules:

| Module | Role |
|--------|------|
| `input` | Lenient serde structs for the status JSON |
| `hud` | Pure rendering; "now" and the git branch are injected |
| `sprite` | The sprite file format, the text grid format and the default character |
| `convert` | Image algorithms: crop, quantize, ASCII mode and half-block pixel mode |
| `install` | `settings.json` merge, backups and restore |

ASCII mode quantizes the image the same way as Pillow's `Quantize.MAXCOVERAGE`. The first palette
color is the image color furthest from the mean. Each next color is the one furthest from all colors
picked so far. Every pixel then maps to its nearest palette color. When two colors tie, the lowest
RGB value wins, so output is deterministic.

## License

[MIT](LICENSE) © Sebastian Marriaga
