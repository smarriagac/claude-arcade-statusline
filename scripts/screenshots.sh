#!/usr/bin/env bash
# Regenerates the README screenshots in docs/screenshots/ and the converter
# example image docs/examples/character.png.
#
# Requirements:
#   - cargo
#   - freeze  (https://github.com/charmbracelet/freeze): lays out the terminal window
#   - resvg   (brew install resvg, or cargo install resvg): rasterizes the SVG
#   - python3 with Pillow
#   - a monospace font with the block, bar and symbol glyphs. The default is
#     Menlo (macOS); elsewhere set FONT_FAMILY and FONT_FILE, for example
#     FONT_FAMILY="DejaVu Sans Mono" FONT_FILE=/usr/share/fonts/TTF/DejaVuSansMono.ttf
#
# Why not freeze's own PNG output? freeze 0.2 draws each ANSI background color
# as a rect 1.5 cells wide, stacked right to left, so every background color
# shows half a cell to the right of its glyph. Half-block sprites use the
# background for their lower pixel, so they come out jagged. freeze also drops
# SGR 1 (bold) and SGR 2 (faint), and its rasterizer has no font fallback.
# So this script lets freeze lay out the window as SVG, then the helper below:
#   - maps bold to a marker freeze keeps, and faint to an explicit gray;
#   - redraws half-block cells as exact pixel rects on the cell grid, the way
#     modern terminals draw block elements;
#   - draws U+2800 (the sprite's transparent cell) as a plain empty cell.
# resvg then rasterizes the SVG with the system fonts.
#
# Since sprite cells become exact rects, no line height can open gaps between
# half-block rows. LINE_HEIGHT=1.2 keeps the sprite pixels square, close to a
# real terminal (Menlo's own line height is 1.16); 1.0 squashes the sprite.
#
# The HUD scenes run with workspace.current_dir set to this repository, so
# STAGE and MISSION show its directory name and current git branch.
# The install scene runs with HOME set to a fresh temporary directory, so it
# never reads or shows your real ~/.claude/settings.json.

set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
shots="$repo/docs/screenshots"
examples="$repo/docs/examples"

FONT_FAMILY=${FONT_FAMILY:-Menlo}
FONT_FILE=${FONT_FILE:-/System/Library/Fonts/Menlo.ttc}
FONT_SIZE=${FONT_SIZE:-14}
LINE_HEIGHT=${LINE_HEIGHT:-1.2}
ZOOM=${ZOOM:-2}

die() {
  printf 'screenshots.sh: %s\n' "$*" >&2
  exit 1
}

for tool in cargo freeze resvg python3; do
  command -v "$tool" >/dev/null || die "missing '$tool' (see the requirements at the top of this script)"
done
python3 -c 'import PIL' 2>/dev/null || die "python3 cannot import Pillow (pip install pillow)"
[[ -f $FONT_FILE ]] || die "font file not found: $FONT_FILE (set FONT_FILE)"

work=$(mktemp -d "${TMPDIR:-/tmp}/arcade-shots.XXXXXX")
demo_home=$(mktemp -d /tmp/home.XXXXXX)
trap 'rm -rf "$work" "$demo_home"' EXIT
helper="$work/helper.py"

read -r -d '' HELPER_SOURCE <<'PY' || true
"""Helpers for scripts/screenshots.sh (Python 3 + Pillow)."""

import json
import math
import re
import sys
import xml.etree.ElementTree as ET

SVG_NS = "http://www.w3.org/2000/svg"
ET.register_namespace("", SVG_NS)
BLANK = "⠀"
# freeze ignores SGR 2 (faint); terminals draw it dimmer, this gray is close.
FAINT = ("38", "2", "112", "117", "142")


def status(kind, repo, now):
    """Sample Claude Code status JSON. resets_at is relative to `now`; the
    extra 30 s keep the RELOAD labels stable while the script runs."""
    now = int(now)
    if kind == "hud":
        ctx, five, seven = 42, 63, 35
        cost, ms = 1.23, 125_000
        five_at, seven_at = now + 8_030, now + 300_030
        extra = {}
    elif kind == "critical":
        ctx, five, seven = 92, 95, 88
        cost, ms = 4.56, 3_725_000
        five_at, seven_at = now + 1_530, now + 90_030
        extra = {"spend_limit": {"used_percentage": 90, "resets_at": now + 1_036_830}}
    else:
        raise SystemExit(f"unknown status kind: {kind}")
    doc = {
        "model": {"display_name": "Opus"},
        "workspace": {"current_dir": repo},
        "cost": {"total_cost_usd": cost, "total_duration_ms": ms},
        "context_window": {
            "used_percentage": ctx,
            "context_window_size": 200_000,
            "total_input_tokens": ctx * 2_000,
        },
        "rate_limits": {
            "five_hour": {"used_percentage": five, "resets_at": five_at},
            "seven_day": {"used_percentage": seven, "resets_at": seven_at},
            **extra,
        },
    }
    print(json.dumps(doc))


def character(grid_path, out_path, scale):
    """Upscales a text pixel grid (assets/default-sprite.txt) to a PNG."""
    from PIL import Image

    scale = int(scale)
    palette, rows, in_grid = {}, [], False
    for raw in open(grid_path, encoding="utf-8").read().splitlines():
        line = raw.rstrip()
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not in_grid:
            if line.strip() == "---":
                in_grid = True
            else:
                key, r, g, b = line.split()
                palette[key] = (int(r), int(g), int(b), 255)
            continue
        rows.append(line)
    width = max(len(row) for row in rows)
    img = Image.new("RGBA", (width, len(rows)), (0, 0, 0, 0))
    for y, row in enumerate(rows):
        for x, key in enumerate(row):
            if key not in ". ":
                img.putpixel((x, y), palette[key])
    img = img.resize((width * scale, len(rows) * scale), Image.NEAREST)
    img.save(out_path, optimize=True)


def sgr():
    """stdin -> stdout. Maps SGR 1 (bold) to SGR 3 (italic), which freeze
    keeps and fix_svg turns back into bold, and SGR 2 (faint) to a gray."""

    def repl(match):
        params = match.group(1).split(";") if match.group(1) else ["0"]
        out, i = [], 0
        while i < len(params):
            p = params[i]
            if p in ("38", "48") and i + 1 < len(params):
                n = 5 if params[i + 1] == "2" else 3
                out += params[i : i + n]
                i += n
                continue
            if p == "1":
                out.append("3")
            elif p == "2":
                out += FAINT
            else:
                out.append(p)
            i += 1
        return "\x1b[" + ";".join(out) + "m"

    data = sys.stdin.read()
    sys.stdout.write(re.sub(r"\x1b\[([0-9;]*)m", repl, data))


def _px(value):
    return float(value.removesuffix("px"))


def fix_svg(src, dst, font_file, size, line_height, zoom):
    """Redraws freeze's sprite cells on an exact cell grid. See the header of
    scripts/screenshots.sh for why."""
    from PIL import ImageFont

    size, line_height = float(size), float(line_height)
    font = ImageFont.truetype(font_file, 1000)
    ascent, descent = font.getmetrics()
    adv = font.getlength("M") / 1000 * size
    asc_px, desc_px = ascent / 1000 * size, descent / 1000 * size
    pitch = size * line_height
    # Distance from a cell's top edge to the text baseline: extra leading is
    # split evenly above and below the glyphs, like a terminal does.
    lead = asc_px + (pitch - asc_px - desc_px) / 2
    freeze_cw = size / 1.68  # freeze's fixed cell-width ratio

    tree = ET.parse(src)
    root = tree.getroot()
    q = lambda tag: f"{{{SVG_NS}}}{tag}"
    group = root.find(q("g"))
    group.attrib.pop("clip-path", None)  # freeze never defines #terminalMask
    texts = group.findall(q("text"))
    left, base0 = _px(texts[0].get("x")), _px(texts[0].get("y"))

    # freeze background rects -> {(row, col): fill}
    bg = {}
    for rect in group.findall(q("rect")):
        x, y, w = _px(rect.get("x")), _px(rect.get("y")), _px(rect.get("width"))
        row = math.floor((y - base0 + pitch) / pitch)
        col = round((x - left) / freeze_cw)
        for k in range(max(1, round(w / freeze_cw - 0.5))):
            bg[(row, col + k)] = rect.get("fill")
        group.remove(rect)

    pixels = {}  # (pixel_row, col) -> fill; two pixel rows per text row
    max_cols = 0
    for row, text in enumerate(texts):
        col = 0

        def walk(s, fill):
            nonlocal col
            if not s:
                return s
            out = []
            for ch in s:
                back = bg.get((row, col))
                top = bottom = None
                if ch == "▀":  # upper half block
                    top, bottom = fill, back
                elif ch == "▄":  # lower half block
                    top, bottom = back, fill
                elif ch == "█":  # full block
                    top = bottom = fill
                elif back:
                    top = bottom = back
                for half, color in ((0, top), (1, bottom)):
                    if color:
                        pixels[(2 * row + half, col)] = color
                out.append(" " if ch in "▀▄█" + BLANK else ch)
                col += 1
            return "".join(out)

        base_fill = text.get("fill") or group.get("fill")
        text.text = walk(text.text, base_fill)
        for span in text:
            if span.get("font-style") == "italic":  # bold marker from sgr()
                del span.attrib["font-style"]
                span.set("font-weight", "bold")
            span.text = walk(span.text, span.get("fill") or base_fill)
            span.tail = walk(span.tail, base_fill)
        max_cols = max(max_cols, col)

    # Pixel rects, snapped to whole output pixels so that neighbors share
    # their edges exactly: no anti-aliased seams, no overlaps.
    zoom = float(zoom)
    snap = lambda v: round(v * zoom) / zoom
    rects = []
    for (prow, col), fill in sorted(pixels.items()):
        row, half = divmod(prow, 2)
        x0 = snap(left + col * adv)
        x1 = snap(left + (col + 1) * adv)
        y0 = snap(base0 + row * pitch - lead + half * pitch / 2)
        y1 = snap(base0 + row * pitch - lead + (half + 1) * pitch / 2)
        rect = ET.Element(q("rect"))
        rect.attrib.update(
            {
                "x": f"{x0:.3f}",
                "y": f"{y0:.3f}",
                "width": f"{x1 - x0:.3f}",
                "height": f"{y1 - y0:.3f}",
                "fill": fill,
                "shape-rendering": "crispEdges",
            }
        )
        rects.append(rect)
    for i, rect in enumerate(rects):
        group.insert(i, rect)

    # freeze sizes the window with its own cell width; widen it to the font's.
    extra = math.ceil(max(0.0, max_cols * (adv - freeze_cw)))
    if extra:
        root.set("width", f"{_px(root.get('width')) + extra:.2f}")
        window = root.find(q("rect"))
        window.set("width", f"{_px(window.get('width')) + extra:.2f}")
    tree.write(dst, encoding="utf-8", xml_declaration=True)


if __name__ == "__main__":
    command, *args = sys.argv[1:]
    {"status": status, "character": character, "sgr": sgr, "fix-svg": fix_svg}[command](*args)
PY
printf '%s\n' "$HELPER_SOURCE" >"$helper"

echo "==> cargo build --release"
cargo build --release --manifest-path "$repo/Cargo.toml"
bin="$repo/target/release/arcade-statusline"

mkdir -p "$shots" "$examples"

# The example input for the converter: the default soldier, 16x upscaled.
python3 "$helper" character "$repo/assets/default-sprite.txt" "$examples/character.png" 16

now=$(date +%s)
python3 "$helper" status hud "$repo" "$now" >"$work/hud.json"
python3 "$helper" status critical "$repo" "$now" >"$work/critical.json"

# A fake home for the install scene, seeded with a sample settings.json that
# already has a statusLine, and a copy of the binary where cargo install puts it.
mkdir -p "$demo_home/.claude" "$demo_home/.cargo/bin"
cp "$bin" "$demo_home/.cargo/bin/arcade-statusline"
printf '%s\n' '{' '  "model": "opus",' '  "statusLine": {' '    "type": "command",' \
  '    "command": "~/.claude/statusline.sh"' '  }' '}' >"$demo_home/.claude/settings.json"

# scene NAME BODY: writes the bash script that freeze --execute runs for a
# scene. Its output goes through the helper's SGR filter (see the header).
scene() {
  {
    printf '#!/usr/bin/env bash\nset -euo pipefail\ncd %q\n{\n' "$repo"
    printf '%s\n' "$2"
    printf '} 2>&1 | python3 %q sgr\n' "$helper"
  } >"$work/$1.sh"
}

# A shell prompt line, so a scene shows the command that produced it.
prompt() {
  printf "printf '\\\\033[1;38;2;57;255;20m\$\\\\033[0m %%s\\\\n' %q" "$1"
}

q_bin=$(printf %q "$bin")
scene hud "$q_bin <$(printf %q "$work/hud.json")"
scene hud-critical "$q_bin <$(printf %q "$work/critical.json")"
scene hud-compact "$q_bin --no-sprite <$(printf %q "$work/hud.json")"
for mode in ascii pixel; do
  # Pixel mode: 9 rows = half the 18-pixel drawing, so each pixel maps to
  # exactly one half block.
  rows=12
  [[ $mode == pixel ]] && rows=9
  args="sprite docs/examples/character.png --mode $mode --rows $rows --preview"
  scene "sprite-$mode" "export HOME=$(printf %q "$demo_home")
$(prompt "arcade-statusline $args")
echo
$q_bin $args"
done
q_demo_bin=$(printf %q "$demo_home/.cargo/bin/arcade-statusline")
scene install "export HOME=$(printf %q "$demo_home")
$(prompt "arcade-statusline install --dry-run")
$q_demo_bin install --dry-run
echo
$(prompt "arcade-statusline install")
$q_demo_bin install"

freeze_flags=(
  --window
  --font.family "$FONT_FAMILY" --font.size "$FONT_SIZE" --line-height "$LINE_HEIGHT"
  --background "#1a1b26" --border.radius 10 --border.width 1 --border.color "#2f334d"
  --padding "22,28" --margin 32 --shadow.blur 20 --shadow.y 10
)

capture() {
  local name=$1 log="$work/$1.log"
  echo "==> docs/screenshots/$name.png"
  # freeze reads stdin when it is not a terminal; give it /dev/null.
  if ! freeze --execute "bash $work/$name.sh" "${freeze_flags[@]}" -o "$work/$name.svg" \
    </dev/null >"$log" 2>&1; then
    printf '%s\n' "$(<"$log")" >&2
    die "freeze failed for $name"
  fi
  python3 "$helper" fix-svg "$work/$name.svg" "$work/$name.fixed.svg" "$FONT_FILE" "$FONT_SIZE" "$LINE_HEIGHT" "$ZOOM"
  if ! resvg --zoom "$ZOOM" --use-font-file "$FONT_FILE" "$work/$name.fixed.svg" "$shots/$name.png" \
    >"$log" 2>&1; then
    printf '%s\n' "$(<"$log")" >&2
    die "resvg failed for $name"
  fi
}

for name in hud hud-critical hud-compact sprite-ascii sprite-pixel install; do
  capture "$name"
done

echo "==> done"
