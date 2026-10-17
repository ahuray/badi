#!/usr/bin/env python3
"""Dress Fcitx's panel, where terminals show Badi's suggestion, in the
current Omarchy theme: its background, a soft tooltip border, the Omarchy
font, and the accent for the word Tab takes. Omarchy's theme-set and
font-set hooks run this again, so the panel follows every theme change.

The theme lives in Fcitx's own theme directory. Fcitx's classicui.conf is
written only when it is absent or was written here; a configuration the
user chose is left alone.
"""

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

THEME_NAME = "badi-omarchy"
MARKER = "# Written by Badi's fcitx-theme.py; delete this file to use Fcitx's own theme."
MIN_CONTRAST = 4.5
HEX = re.compile(r"^#[0-9a-fA-F]{6}$")


def rgb(color):
    return tuple(int(color[index:index + 2], 16) for index in (1, 3, 5))


def mix(color, base, share):
    """`share` of `color` over `base`, as an opaque hex colour."""
    return "#" + "".join(f"{round(a * share + b * (1 - share)):02x}" for a, b in zip(rgb(color), rgb(base)))


def luminance(color):
    def channel(value):
        value /= 255
        return value / 12.92 if value <= 0.03928 else ((value + 0.055) / 1.055) ** 2.4
    red, green, blue = (channel(value) for value in rgb(color))
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue


def contrast(first, second):
    light, dark = sorted((luminance(first), luminance(second)), reverse=True)
    return (light + 0.05) / (dark + 0.05)


def softened(foreground, background):
    """The softest mix of the foreground that still reads at 4.5:1."""
    for share in (0.7, 0.75, 0.8, 0.85, 0.9, 0.95):
        color = mix(foreground, background, share)
        if contrast(color, background) >= MIN_CONTRAST:
            return color
    return foreground


def palette(colors):
    background = colors["background"]
    foreground = colors["foreground"]
    accent = colors.get("accent", colors.get("blue", foreground))
    if not all(HEX.match(value or "") for value in (background, foreground, accent)):
        raise ValueError("Omarchy colors.toml needs #rrggbb background, foreground and accent")
    if contrast(accent, background) < MIN_CONTRAST:
        accent = foreground
    return {
        "background": background,
        "text": softened(foreground, background),
        "strong": accent,
        # Omarchy's control border: the foreground at 40% over the surface.
        "border": mix(foreground, background, 0.4),
        "selection": colors.get("selection") if HEX.match(colors.get("selection", "")) else mix(foreground, background, 0.18),
    }


def theme(colors):
    return f"""[Metadata]
Name=Badi (Omarchy)
Version=1
Author=Badi
Description=Generated from the current Omarchy theme
ScaleWithDPI=True

[InputPanel]
NormalColor={colors["text"]}
HighlightCandidateColor={colors["strong"]}
HighlightColor={colors["strong"]}
HighlightBackgroundColor={colors["background"]}
EnableBlur=False

[InputPanel/TextMargin]
Left=9
Right=9
Top=5
Bottom=5

[InputPanel/ContentMargin]
Left=0
Right=0
Top=0
Bottom=0

[InputPanel/Background]
Color={colors["background"]}
BorderColor={colors["border"]}
BorderWidth=1

[InputPanel/Background/Margin]
Left=1
Right=1
Top=1
Bottom=1

[InputPanel/Highlight]
Color={colors["background"]}

[InputPanel/Highlight/Margin]
Left=0
Right=0
Top=0
Bottom=0

[Menu]
NormalColor={colors["text"]}
HighlightCandidateColor={colors["strong"]}

[Menu/Background]
Color={colors["background"]}
BorderColor={colors["border"]}
BorderWidth=1

[Menu/Background/Margin]
Left=1
Right=1
Top=1
Bottom=1

[Menu/ContentMargin]
Left=1
Right=1
Top=1
Bottom=1

[Menu/Highlight]
Color={colors["selection"]}

[Menu/Separator]
Color={colors["border"]}

[Menu/TextMargin]
Left=9
Right=9
Top=5
Bottom=5
"""


def font(home):
    """The Omarchy font at the terminal's size: the panel mostly appears in
    terminals, beside their text."""
    try:
        family = subprocess.run(["omarchy", "font", "current"], capture_output=True, text=True,
                                timeout=5, check=True).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        family = ""
    size = 10
    ghostty = home / ".config/ghostty/config"
    if ghostty.is_file():
        for line in ghostty.read_text(errors="replace").splitlines():
            match = re.match(r"\s*font-size\s*=\s*(\d{1,2})(?:\.\d+)?\s*$", line)
            if match:
                size = int(match.group(1))
    family = family if re.fullmatch(r"[\w .+-]{1,80}", family) else "Sans"
    return f"{family} {size}"


def files(home, environ=os.environ):
    """The theme file and, when it is Badi's to write, Fcitx's classicui.conf."""
    state = Path(environ.get("XDG_STATE_HOME") or home / ".local/state")
    data = Path(environ.get("XDG_DATA_HOME") or home / ".local/share")
    config = Path(environ.get("XDG_CONFIG_HOME") or home / ".config")
    colors = palette(tomllib.loads((state / "omarchy/current/theme/colors.toml").read_text()))
    written = [(data / "fcitx5/themes" / THEME_NAME / "theme.conf", theme(colors))]
    classicui = config / "fcitx5/conf/classicui.conf"
    if not classicui.exists() or classicui.read_text(errors="replace").startswith(MARKER):
        face = font(home)
        written.append((classicui, f'{MARKER}\nTheme={THEME_NAME}\nDarkTheme={THEME_NAME}\n'
                                   f'UseDarkTheme=False\nFont="{face}"\nMenuFont="{face}"\n'))
    else:
        print(f"{classicui} is yours: set Theme={THEME_NAME} there to use Badi's panel theme.")
    return written


def reload():
    """A running Fcitx picks the theme up at once; otherwise at its start."""
    subprocess.run(["busctl", "--user", "call", "org.fcitx.Fcitx5", "/controller",
                    "org.fcitx.Fcitx.Controller1", "ReloadAddonConfig", "s", "classicui"],
                   capture_output=True, timeout=5, check=False)


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.badi-{os.getpid()}")
    temporary.write_text(text)
    os.replace(temporary, path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("theme", nargs="?", help="Omarchy's hook argument; the current theme is read")
    parser.parse_args()
    try:
        written = files(Path.home())
    except (OSError, KeyError, ValueError, tomllib.TOMLDecodeError) as error:
        print(f"Badi Fcitx theme unchanged: {error}", file=sys.stderr)
        return 1
    for path, text in written:
        write(path, text)
    reload()
    return 0


if __name__ == "__main__":
    sys.exit(main())
