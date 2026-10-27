#!/usr/bin/env bash
# Builds the Tencent SCF upload package: dist/askhuman-relay.zip
# (src/ + scf_bootstrap). No secrets are read or embedded; configuration comes from the
# function's environment variables at runtime.
set -euo pipefail

cd "$(dirname "$0")/.."
STAGE=dist/stage
rm -rf dist
mkdir -p "$STAGE"
cp -R src "$STAGE/src"

# Web functions start via scf_bootstrap and must listen on 0.0.0.0:9000.
# NODE_BIN follows Tencent's documented runtime layout; verify it against the current SCF docs
# when deploying (the fallback uses whatever `node` is on PATH).
cat > "$STAGE/scf_bootstrap" <<'EOF'
#!/bin/bash
NODE_BIN=/var/lang/node20/bin/node
[ -x "$NODE_BIN" ] || NODE_BIN="$(command -v node)"
export PORT=9000
exec "$NODE_BIN" src/adapters/scf.js
EOF
chmod 755 "$STAGE/scf_bootstrap"

(cd "$STAGE" && zip -qrX ../askhuman-relay.zip scf_bootstrap src)
echo "wrote relay/dist/askhuman-relay.zip ($(wc -c < dist/askhuman-relay.zip | tr -d ' ') bytes)"
