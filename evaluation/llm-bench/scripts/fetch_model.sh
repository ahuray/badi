#!/usr/bin/env bash
# fetch_model.sh LABEL... — download each listed model from models.json into the data
# directory and keep it only if it matches the Hugging Face LFS sha256.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
for label in "$@"; do
  read -r repo file < <(python3 -I "$here/bench_paths.py" fetch-spec "$label")
  dest=$(python3 -I "$here/bench_paths.py" model "$label")
  mkdir -p "$(dirname "$dest")"
  expected=$(curl -fsS "https://huggingface.co/api/models/$repo/tree/main?recursive=1" |
    python3 -I -c 'import sys,json; f=sys.argv[1]; print(next(i["lfs"]["oid"] for i in json.load(sys.stdin) if i["path"]==f))' "$file")
  if [[ -f "$dest" ]] && [[ $(sha256sum "$dest" | cut -d' ' -f1) == "$expected" ]]; then
    echo "present $label"; continue
  fi
  curl -fL --retry 3 -sS -o "$dest.part" "https://huggingface.co/$repo/resolve/main/$file"
  actual=$(sha256sum "$dest.part" | cut -d' ' -f1)
  [[ "$actual" == "$expected" ]] || { rm -f "$dest.part"; echo "SHA MISMATCH $label $actual != $expected" >&2; exit 1; }
  mv "$dest.part" "$dest"
  echo "ok $label $(stat -c %s "$dest") $actual"
done
