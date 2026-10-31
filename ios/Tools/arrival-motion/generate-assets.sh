#!/bin/sh
# Regenerate the exact Lucide stencils used by the Mac arrival bubble.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
catalog="$here/../../AskHuman/Resources/Assets.xcassets"
for name in body hole; do
  rsvg-convert -f pdf -o "$catalog/arrival-$name.imageset/arrival-$name.pdf" "$here/arrival-$name.svg"
done
cp "$here/NOTICE.txt" "$here/../../AskHuman/Resources/ArrivalLucideLicense.txt"
