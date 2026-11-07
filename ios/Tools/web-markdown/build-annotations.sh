#!/usr/bin/env bash
# Builds the annotation engine of the iOS Markdown page from src/lib/annotations.ts, the very file the Mac popup
# imports, so that a passage is found, numbered and drawn by the same code on both:
#
#   ios/Tools/web-markdown/build-annotations.sh
#
# It writes ios/AskHuman/Resources/WebMarkdown/annotations.js (a classic script that defines `AHAnnotations`).
# The first line of the result names the source and its SHA-256, and src/lib/annotations.bundle.test.ts fails
# when the source has moved on: after any change to the engine, run this again and commit the result.
# The page itself (notes.js, index.html, style.css) is written by hand and is not touched.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
source_file="src/lib/annotations.ts"
dest="$root/ios/AskHuman/Resources/WebMarkdown/annotations.js"

cd "$root"
if [[ ! -x node_modules/.bin/vite ]]; then
  echo "missing node_modules/.bin/vite: install the npm dependencies first (npm install)" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
node_modules/.bin/vite build --config vite.annotations.config.ts --outDir "$work" --emptyOutDir --logLevel warn

if [[ ! -s "$work/annotations.js" ]]; then
  echo "the build produced no annotations.js" >&2
  exit 1
fi

hash="$(shasum -a 256 "$source_file" | cut -d' ' -f1)"
{
  echo "// Generated from $source_file (sha256 $hash) by ios/Tools/web-markdown/build-annotations.sh; do not edit by hand."
  cat "$work/annotations.js"
} > "$dest"

echo "Wrote ${dest#"$root/"} from $source_file (sha256 $hash)"
