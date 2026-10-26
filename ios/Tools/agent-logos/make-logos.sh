#!/bin/sh
# Rebuilds the vector assets of the Agent logos (Assets.xcassets/AgentLogos) from the SVG sources
# next to this script. Needs `rsvg-convert` (brew install librsvg). Sources and licenses: NOTICE.md.
#
# The mono marks are black shapes on a 24-unit grid, drawn as template images so the badge can color
# them; the Pi mark keeps its own three colors.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
catalog="$here/../../AskHuman/Resources/Assets.xcassets/AgentLogos"
mkdir -p "$catalog"
printf '{\n  "info" : { "author" : "xcode", "version" : 1 }\n}\n' > "$catalog/Contents.json"

for svg in "$here"/*.svg; do
  name=$(basename "$svg" .svg)
  set="$catalog/agent-$name.imageset"
  mkdir -p "$set"
  rsvg-convert -f pdf -o "$set/agent-$name.pdf" "$svg"
  intent=template
  [ "$name" = pi ] && intent=original
  cat > "$set/Contents.json" <<JSON
{
  "images" : [
    { "filename" : "agent-$name.pdf", "idiom" : "universal" }
  ],
  "info" : { "author" : "xcode", "version" : 1 },
  "properties" : {
    "preserves-vector-representation" : true,
    "template-rendering-intent" : "$intent"
  }
}
JSON
done
