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
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

THEME_NAME = "badi-omarchy"
MARKER = "# Written by Badi's fcitx-theme.py; delete this file to use Fcitx's own theme."
MIN_CONTRAST = 4.5
HEX = re.compile(r"^#[0-9a-fA-F]{6}$")
TERMINALS = {
    "com.mitchellh.ghostty.desktop": "ghostty",
    "alacritty.desktop": "alacritty",
    "kitty.desktop": "kitty",
    "foot.desktop": "foot",
    "footclient.desktop": "foot",
}


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


def valid_size(value):
    """An integer font size in 5..40, rounded from an int, float or numeric text."""
    if isinstance(value, bool):
        return None
    try:
        number = float(value)
    except (TypeError, ValueError):
        return None
    if not math.isfinite(number):
        return None
    # Half points round up, as people read them (10.5 is 11, not 10).
    size = math.floor(number + 0.5)
    return size if 5 <= size <= 40 else None


def read_lines(path):
    """Lines of a UTF-8 config file, or empty when missing or unreadable."""
    try:
        return path.read_text(encoding="utf-8", errors="replace").splitlines() if path.is_file() else []
    except OSError:
        return []


def config_home(home, environ):
    return Path(environ.get("XDG_CONFIG_HOME") or home / ".config")


def search_dirs(value, default):
    """The absolute directories of a colon-separated XDG search path."""
    return [Path(entry) for entry in (value or default).split(":") if entry.startswith("/")]


def ghostty_size(home, environ):
    size = None
    for line in read_lines(config_home(home, environ) / "ghostty/config"):
        match = re.match(r"\s*font-size\s*=\s*([+-]?\d+(?:\.\d+)?)\s*$", line)
        if match:
            # The last setting wins, even one Badi cannot use.
            size = valid_size(match.group(1))
    return size


def alacritty_size(home, environ):
    user = config_home(home, environ)
    directories = [user, *search_dirs(environ.get("XDG_CONFIG_DIRS"), "/etc/xdg")]
    # Alacritty's own search: the first of these files that exists.
    path = next((path for directory in directories
                 for path in (directory / "alacritty/alacritty.toml", directory / "alacritty.toml")
                 if path.is_file()), None)
    try:
        if path is None:
            return None
        config = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError):
        return None
    font_table = config.get("font")
    if not isinstance(font_table, dict):
        return None
    raw = font_table.get("size")
    return valid_size(raw) if isinstance(raw, (int, float)) else None


def first_config(home, environ, relative, explicit=None):
    """The one config file a terminal loads: the first that exists in the
    user's then the system's config directories, never merged."""
    candidates = [config_home(home, environ) / relative,
                  *(directory / relative for directory in search_dirs(environ.get("XDG_CONFIG_DIRS"), "/etc/xdg"))]
    if explicit:
        candidates.insert(0, Path(explicit) / Path(relative).name)
    return next((path for path in candidates if path.is_file()), None)


def kitty_configs(home, environ):
    """Every kitty.conf Kitty applies, lowest priority first: the system's,
    then the user's, whose keys override them (Omarchy keeps its defaults in
    /etc/xdg/kitty). KITTY_CONFIG_DIRECTORY replaces them all."""
    if explicit := environ.get("KITTY_CONFIG_DIRECTORY"):
        return [Path(explicit) / "kitty.conf"]
    system = search_dirs(environ.get("XDG_CONFIG_DIRS"), "/etc/xdg")
    return [*(directory / "kitty/kitty.conf" for directory in reversed(system)),
            config_home(home, environ) / "kitty/kitty.conf"]


def kitty_size(home, environ):
    size = None
    for line in (line for path in kitty_configs(home, environ) for line in read_lines(path)):
        match = re.match(r"\s*font_size\s+([+-]?\d+(?:\.\d+)?)\s*$", line)
        if match:
            size = valid_size(match.group(1))
    return size


def foot_size(home, environ):
    size = None
    section = "main"
    path = first_config(home, environ, "foot/foot.ini")
    for line in read_lines(path) if path else []:
        header = re.match(r"\s*\[([^\]]+)\]\s*(?:#.*)?$", line)
        if header:
            section = header.group(1).strip()
            continue
        if section != "main":
            continue
        match = re.match(r"\s*font\s*=\s*(.+)$", line)
        if not match:
            continue
        primary = match.group(1).split("#", 1)[0].split(",", 1)[0]
        # Each font= line replaces the last one whole, size included.
        size = None
        for attribute in primary.split(":")[1:]:
            sized = re.match(r"\s*size\s*=\s*([+-]?\d+(?:\.\d+)?)\s*$", attribute)
            if sized and (parsed := valid_size(sized.group(1))) is not None:
                size = parsed
    return size


TERMINAL_SIZES = {
    "ghostty": ghostty_size,
    "alacritty": alacritty_size,
    "kitty": kitty_size,
    "foot": foot_size,
}


def terminal_lists(home, environ):
    """xdg-terminal-exec's preference lists, most preferred first: each config
    directory, then each data directory's xdg-terminal-exec folder, with the
    current desktop's own list before the shared one."""
    desktops = [name for name in environ.get("XDG_CURRENT_DESKTOP", "").lower().split(":") if name]
    names = [f"{desktop}-xdg-terminals.list" for desktop in desktops] + ["xdg-terminals.list"]
    configs = [config_home(home, environ), *search_dirs(environ.get("XDG_CONFIG_DIRS"), "/etc/xdg")]
    data = search_dirs(environ.get("XDG_DATA_DIRS"), "/usr/local/share:/usr/share")
    return ([directory / name for directory in configs for name in names]
            + [directory / "xdg-terminal-exec" / name for directory in data for name in names])


def installed(entry, home, environ):
    """Whether xdg-terminal-exec would run this desktop entry: its first file
    is a terminal emulator that is not hidden and whose TryExec is found."""
    if "/" in entry:
        return False
    data_home = Path(environ.get("XDG_DATA_HOME") or home / ".local/share")
    directories = [data_home, *search_dirs(environ.get("XDG_DATA_DIRS"), "/usr/local/share:/usr/share")]
    path = next((directory / "applications" / entry for directory in directories
                 if (directory / "applications" / entry).is_file()), None)
    if path is None:
        return False
    fields, section = {}, None
    for line in read_lines(path):
        line = line.strip()
        if line.startswith("["):
            section = line
        elif section == "[Desktop Entry]" and "=" in line and not line.startswith("#"):
            key, value = line.split("=", 1)
            fields.setdefault(key.strip(), value.strip())
    search = environ.get("PATH", os.defpath)
    try_exec = fields.get("TryExec")
    command = fields.get("Exec", "").split()
    return (fields.get("Hidden", "").lower() != "true"
            and "TerminalEmulator" in fields.get("Categories", "").split(";")
            and bool(command) and shutil.which(command[0].strip('"'), path=search) is not None
            and not (try_exec and shutil.which(try_exec, path=search) is None))


def preferred_terminal(home, environ):
    """The terminal xdg-terminal-exec opens, when Badi knows it: the first
    installed entry of its lists; `-` and `+` lines only filter fallbacks."""
    for path in terminal_lists(home, environ):
        for line in read_lines(path):
            entry = line.strip().split(":", 1)[0].strip()
            if not entry or entry.startswith(("#", "-", "+")) or not installed(entry, home, environ):
                continue
            return TERMINALS.get(entry.lower())
    return None


def terminal_size(home, environ):
    """The preferred terminal's font size, falling back to Ghostty, then 10."""
    terminal = preferred_terminal(home, environ)
    if terminal and (size := TERMINAL_SIZES[terminal](home, environ)) is not None:
        return size
    if terminal != "ghostty" and (size := ghostty_size(home, environ)) is not None:
        return size
    return 10


def font(home, environ=os.environ):
    """The Omarchy font at the terminal's size: the panel mostly appears in
    terminals, beside their text."""
    try:
        family = subprocess.run(["omarchy", "font", "current"], capture_output=True, text=True,
                                timeout=5, check=True).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        family = ""
    family = family if re.fullmatch(r"[\w .+-]{1,80}", family) else "Sans"
    return f"{family} {terminal_size(home, environ)}"


def files(home, environ=os.environ):
    """The theme file and, when it is Badi's to write, Fcitx's classicui.conf."""
    state = Path(environ.get("XDG_STATE_HOME") or home / ".local/state")
    data = Path(environ.get("XDG_DATA_HOME") or home / ".local/share")
    config = Path(environ.get("XDG_CONFIG_HOME") or home / ".config")
    colors = palette(tomllib.loads((state / "omarchy/current/theme/colors.toml").read_text()))
    written = [(data / "fcitx5/themes" / THEME_NAME / "theme.conf", theme(colors))]
    classicui = config / "fcitx5/conf/classicui.conf"
    if not classicui.exists() or classicui.read_text(errors="replace").startswith(MARKER):
        face = font(home, environ)
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
    """Replaces `path` in one rename from a new, unpredictably named file, so
    a planted symlink is never followed and readers never see half a theme."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", dir=path.parent, prefix=f".{path.name}.", delete=False) as stream:
        staged = Path(stream.name)
        try:
            os.fchmod(stream.fileno(), 0o644)
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            staged.unlink(missing_ok=True)
            raise
    try:
        os.replace(staged, path)
    finally:
        staged.unlink(missing_ok=True)


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
