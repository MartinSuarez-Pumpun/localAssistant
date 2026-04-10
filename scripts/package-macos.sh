#!/usr/bin/env bash
# package-macos.sh — Empaqueta LocalAiAssistant como .app + .dmg para macOS
#
# Arquitectura del bundle:
#   LocalAiAssistant.app/
#     Contents/
#       MacOS/
#         LocalAiAssistant   ← launcher (shell script)
#         serve-rs           ← servidor Rust
#       Resources/
#         web/dist/          ← frontend Leptos WASM
#         AppIcon.icns
#       Info.plist
#
# Uso: make release-macos
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="${1:-$ROOT_DIR/dist}"
BUNDLE_NAME="LocalAiAssistant"
VERSION=$(date +%Y.%m.%d)

CYAN='\033[36m'; GREEN='\033[32m'; YELLOW='\033[33m'; RED='\033[31m'; RESET='\033[0m'
info() { echo -e "${CYAN}→ $*${RESET}"; }
ok()   { echo -e "${GREEN}✓ $*${RESET}"; }
warn() { echo -e "${YELLOW}⚠ $*${RESET}"; }
die()  { echo -e "${RED}✗ $*${RESET}" >&2; exit 1; }

[ "$(uname)" = "Darwin" ] || die "Solo macOS"

source "$HOME/.cargo/env" 2>/dev/null || true

# ── 1. Build serve-rs (release) ───────────────────────────────────────────────
info "Build serve-rs..."
cd "$ROOT_DIR/server"
cargo build --release --quiet
SERVE_RS="$ROOT_DIR/server/target/release/serve-rs"
[ -f "$SERVE_RS" ] || die "serve-rs no encontrado tras build"
ok "serve-rs: $(du -sh "$SERVE_RS" | cut -f1)"

# ── 2. Build frontend Leptos/WASM (release) ───────────────────────────────────
info "Build frontend Leptos/WASM..."
cd "$ROOT_DIR/web-app"
cargo clean --target wasm32-unknown-unknown --quiet 2>/dev/null || true
trunk build --release --quiet
WEB_DIST="$ROOT_DIR/web/dist"
[ -f "$WEB_DIST/index.html" ] || die "web/dist/index.html no encontrado"
ok "web/dist/ — $(du -sh "$WEB_DIST" | cut -f1)"

# ── 3. Crear .app bundle ──────────────────────────────────────────────────────
info "Creando .app bundle..."
cd "$ROOT_DIR"

BUNDLE="$ROOT_DIR/build/${BUNDLE_NAME}.app"
MACOS_DIR="$BUNDLE/Contents/MacOS"
RES_DIR="$BUNDLE/Contents/Resources"

rm -rf "$BUNDLE"
mkdir -p "$MACOS_DIR" "$RES_DIR"

# Copiar serve-rs
cp "$SERVE_RS" "$MACOS_DIR/serve-rs"
chmod 755 "$MACOS_DIR/serve-rs"

# Copiar frontend
mkdir -p "$RES_DIR/web/dist"
cp -r "$WEB_DIST/." "$RES_DIR/web/dist/"

# Launcher script — arranca serve-rs con las rutas correctas del bundle
cat > "$MACOS_DIR/$BUNDLE_NAME" <<'LAUNCHER'
#!/usr/bin/env bash
# Launcher de LocalAiAssistant
DIR="$(cd "$(dirname "$0")" && pwd)"
RES="$DIR/../Resources"
AI_BASE="$HOME/.local-ai"
mkdir -p "$AI_BASE/plugins" "$AI_BASE/workspace" "$AI_BASE/uploads"

exec "$DIR/serve-rs" \
  --web-dist "$RES/web/dist" \
  --plugins-dir "$AI_BASE/plugins"
LAUNCHER
chmod 755 "$MACOS_DIR/$BUNDLE_NAME"

# ── 4. Info.plist ─────────────────────────────────────────────────────────────
cat > "$BUNDLE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>              <string>LocalAiAssistant</string>
  <key>CFBundleDisplayName</key>       <string>LocalAiAssistant</string>
  <key>CFBundleIdentifier</key>        <string>com.localaiassistant.app</string>
  <key>CFBundleVersion</key>           <string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key>        <string>${BUNDLE_NAME}</string>
  <key>CFBundlePackageType</key>       <string>APPL</string>
  <key>CFBundleIconFile</key>          <string>AppIcon</string>
  <key>LSMinimumSystemVersion</key>    <string>12.0</string>
  <key>NSHighResolutionCapable</key>   <true/>
  <key>LSUIElement</key>               <false/>
</dict>
</plist>
PLIST

# ── 5. Icono ──────────────────────────────────────────────────────────────────
ICON_SRC="$SCRIPT_DIR/llm-assistant.png"
if [ -f "$ICON_SRC" ] && command -v sips &>/dev/null && command -v iconutil &>/dev/null; then
    info "Generando AppIcon.icns..."
    ICONSET="$ROOT_DIR/build/AppIcon.iconset"
    rm -rf "$ICONSET"; mkdir -p "$ICONSET"
    for size in 16 32 64 128 256 512; do
        sips -z $size $size "$ICON_SRC" --out "$ICONSET/icon_${size}x${size}.png" &>/dev/null
        sips -z $((size*2)) $((size*2)) "$ICON_SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" &>/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$RES_DIR/AppIcon.icns"
    ok "AppIcon.icns"
else
    warn "Sin icono (necesita sips + iconutil + llm-assistant.png)"
fi

ok ".app bundle: $BUNDLE"

# ── 6. .dmg ───────────────────────────────────────────────────────────────────
info "Creando .dmg..."
mkdir -p "$DIST_DIR"
DMG_OUT="$DIST_DIR/${BUNDLE_NAME}-macos.dmg"
DMG_STAGING="$ROOT_DIR/build/dmg-staging"

rm -rf "$DMG_STAGING"
mkdir -p "$DMG_STAGING"
cp -r "$BUNDLE" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"

hdiutil create \
    -volname "LocalAiAssistant" \
    -srcfolder "$DMG_STAGING" \
    -ov -format UDZO \
    -o "$DMG_OUT" &>/dev/null \
    && ok ".dmg: $DMG_OUT ($(du -sh "$DMG_OUT" | cut -f1))" \
    || warn "hdiutil falló — el .app sigue disponible"

cp -r "$BUNDLE" "$DIST_DIR/"
ok ".app copiado a dist/"

echo ""
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${GREEN}  dist/${BUNDLE_NAME}.app${RESET}"
echo -e "${GREEN}  dist/${BUNDLE_NAME}-macos.dmg${RESET}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${CYAN}  Doble clic en .app → abre http://localhost:8080${RESET}"
