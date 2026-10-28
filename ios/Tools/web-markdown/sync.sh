#!/usr/bin/env bash
# Copies the Markdown engine that the iOS app bundles out of node_modules: markdown-it and Mermaid, the
# same packages (and so the same versions) as the Mac popup. Run it again after upgrading either one:
#
#   ios/Tools/web-markdown/sync.sh
#
# It rewrites the two libraries, their license texts and VERSIONS.txt in ios/AskHuman/Resources/WebMarkdown.
# The page itself (index.html, style.css, render.js) and NOTICE.md are written by hand and are not touched.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
dest="$root/ios/AskHuman/Resources/WebMarkdown"
mkdir -p "$dest"

field() { # <package> <json field>
  python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' \
    "$root/node_modules/$1/package.json" "$2"
}

copy() { # <package> <file inside the package>
  local source="$root/node_modules/$1/$2"
  if [[ ! -f "$source" ]]; then
    echo "missing $source: install the npm dependencies first (npm install)" >&2
    exit 1
  fi
  cp "$source" "$dest/$(basename "$2")"
}

copy markdown-it dist/markdown-it.min.js
copy mermaid dist/mermaid.min.js
cp "$root/node_modules/markdown-it/LICENSE" "$dest/LICENSE-markdown-it.txt"
cp "$root/node_modules/mermaid/LICENSE" "$dest/LICENSE-mermaid.txt"

{
  echo "markdown-it $(field markdown-it version) ($(field markdown-it license))  node_modules/markdown-it/dist/markdown-it.min.js"
  echo "mermaid $(field mermaid version) ($(field mermaid license))  node_modules/mermaid/dist/mermaid.min.js"
  echo "Written by ios/Tools/web-markdown/sync.sh; do not edit by hand."
} > "$dest/VERSIONS.txt"

echo "Synced into ${dest#"$root/"}:"
cat "$dest/VERSIONS.txt"
