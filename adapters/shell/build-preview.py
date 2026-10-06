#!/usr/bin/env python3
"""Build Badi's display-only Bash builtin using this system's Bash headers."""
from pathlib import Path
import os
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
def main():
    output = ROOT / 'build'
    output.mkdir(exist_ok=True)
    headers = Path(os.environ.get('BADI_BASH_INCLUDE_DIR', '/usr/include/bash'))
    if not (headers / 'builtins.h').is_file():
        raise SystemExit('Bash development headers are required (Ubuntu: bash-builtins; Arch: bash).')
    with tempfile.TemporaryDirectory(prefix='.preview-', dir=output) as temporary:
        library = Path(temporary) / 'badi-preview.so'
        subprocess.run([*shlex.split(os.environ.get('CC', 'cc')), '-std=c11', '-O2', '-fPIC', '-shared',
                        '-Wall', '-Wextra', '-Werror', f'-I{headers}', f'-I{headers / "include"}',
                        str(ROOT / 'preview.c'), '-o', str(library)], check=True)
        library.replace(output / library.name)
    print(output / 'badi-preview.so')


if __name__ == '__main__':
    main()
