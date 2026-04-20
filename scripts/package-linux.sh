#!/usr/bin/env bash
# package-linux.sh — Empaqueta LocalAiAssistant como tarball autocontenido para Linux
# Uso: make release-linux
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

[ "$(uname)" = "Linux" ] || die "Este script es solo para Linux. En macOS usa: make release-macos"

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
trunk build --release --quiet
WEB_DIST="$ROOT_DIR/web/dist"
[ -f "$WEB_DIST/index.html" ] || die "web/dist/index.html no encontrado"
ok "web/dist/ — $(du -sh "$WEB_DIST" | cut -f1)"

# ── 3. Hacer el frontend autocontenido (sin CDN externos) ────────────────────
info "Preparando build autocontenido..."

INDEX="$WEB_DIST/index.html"

# Tailwind CSS
if grep -q "cdn.tailwindcss.com" "$INDEX"; then
    if curl -sfL "https://cdn.tailwindcss.com" -o "$WEB_DIST/tailwind.js" 2>/dev/null; then
        sed -i 's|<script src="https://cdn.tailwindcss.com"></script>|<script src="/tailwind.js"></script>|g' "$INDEX"
        ok "Tailwind descargado localmente"
    else
        warn "No se pudo descargar Tailwind — la app puede verse sin estilos offline"
    fi
fi

# Material Symbols: ya se incluyen en web/dist/static/fonts/ vía trunk copy-dir
# No hace falta descargarlas en tiempo de release.
if [ -f "$WEB_DIST/static/fonts/material-symbols-outlined.woff2" ]; then
    ok "Fuentes Material Symbols incluidas en el build (sin CDN)"
else
    warn "No se encontraron las fuentes en web/dist/static/fonts/ — los iconos pueden no verse"
fi

# Eliminar atributos SRI (integrity/crossorigin) que WebKitGTK rechaza
# cuando los archivos han sido modificados post-build
python3 -c "
import re
content = open('$INDEX').read()
# Quitar integrity='sha384-...' y crossorigin='anonymous'
content = re.sub(r' integrity=\"sha384-[A-Za-z0-9+/=]+\"', '', content)
content = re.sub(r' crossorigin=\"anonymous\"', '', content)
open('$INDEX', 'w').write(content)
"
ok "SRI hashes eliminados (compatibilidad WebKitGTK)"

ok "Frontend autocontenido"

# ── 4. Ensamblar directorio Linux ─────────────────────────────────────────────
info "Ensamblando directorio Linux..."
cd "$ROOT_DIR"

LINUX_DIR="$ROOT_DIR/build/${BUNDLE_NAME}-linux"
rm -rf "$LINUX_DIR"
mkdir -p "$LINUX_DIR/web/dist"

# Copiar serve-rs
cp "$SERVE_RS" "$LINUX_DIR/serve-rs"
chmod 755 "$LINUX_DIR/serve-rs"
ok "serve-rs copiado"

# Copiar frontend
cp -r "$WEB_DIST/." "$LINUX_DIR/web/dist/"
ok "web/dist/ copiado"

# Launcher principal — kiosk por defecto (fullscreen, sin decoraciones)
cat > "$LINUX_DIR/start.sh" <<'LAUNCHER'
#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
AI_BASE="$HOME/.local-ai"
mkdir -p "$AI_BASE/plugins" "$AI_BASE/workspace" "$AI_BASE/uploads"
exec "$DIR/serve-rs" --web-dist "$DIR/web/dist" --plugins-dir "$AI_BASE/plugins"
LAUNCHER
chmod 755 "$LINUX_DIR/start.sh"
ok "start.sh escrito (kiosk)"

# Launcher ventana normal (para depuración o uso en escritorio)
cat > "$LINUX_DIR/start-windowed.sh" <<'WINDOWED'
#!/usr/bin/env bash
DIR="$(cd "$(dirname "$0")" && pwd)"
AI_BASE="$HOME/.local-ai"
mkdir -p "$AI_BASE/plugins" "$AI_BASE/workspace" "$AI_BASE/uploads"
exec "$DIR/serve-rs" --web-dist "$DIR/web/dist" --plugins-dir "$AI_BASE/plugins"
WINDOWED
chmod 755 "$LINUX_DIR/start-windowed.sh"
ok "start-windowed.sh escrito"

# Icono de la app (PNG 256x256)
ICON_SRC="$ROOT_DIR/assets/icon.png"
ICON_DEST="$LINUX_DIR/icon.png"
if [ -f "$ICON_SRC" ]; then
    cp "$ICON_SRC" "$ICON_DEST"
else
    python3 - "$ICON_DEST" <<'PYICON'
import struct, zlib, sys
def make_png(path, color=(26,86,219)):
    w=h=256
    raw=b''.join(b'\x00'+bytes(color)*w for _ in range(h))
    def chunk(t,d): c=zlib.crc32(t+d)&0xffffffff; return struct.pack('>I',len(d))+t+d+struct.pack('>I',c)
    data=chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))
    data+=chunk(b'IDAT',zlib.compress(raw))
    data+=chunk(b'IEND',b'')
    open(path,'wb').write(b'\x89PNG\r\n\x1a\n'+data)
make_png(sys.argv[1])
PYICON
fi
ok "icon.png generado"

# Script de instalación al sistema
cat > "$LINUX_DIR/install.sh" <<'INSTALL'
#!/usr/bin/env bash
# Instala LocalAiAssistant en ~/.local/share y crea lanzador en el menú de apps
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="$HOME/.local/share/LocalAiAssistant"
APPS_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"

echo "→ Instalando en $INSTALL_DIR ..."
mkdir -p "$INSTALL_DIR"
cp -r "$DIR/." "$INSTALL_DIR/"
chmod 755 "$INSTALL_DIR/serve-rs" \
          "$INSTALL_DIR/start.sh" \
          "$INSTALL_DIR/start-windowed.sh" \
          "$INSTALL_DIR/install.sh" \
          "$INSTALL_DIR/uninstall.sh"

echo "→ Instalando icono ..."
mkdir -p "$ICON_DIR"
cp "$INSTALL_DIR/icon.png" "$ICON_DIR/localaiassistant.png"

echo "→ Creando entradas en el menú de aplicaciones ..."
mkdir -p "$APPS_DIR"

cat > "$APPS_DIR/localaiassistant.desktop" <<EOF
[Desktop Entry]
Version=1.0
Name=LocalAI Assistant
Comment=Local LLM chat interface
Exec=$INSTALL_DIR/start.sh
Icon=localaiassistant
Terminal=false
Type=Application
Categories=Utility;AI;
StartupWMClass=LocalAiAssistant
EOF

cat > "$APPS_DIR/localaiassistant-windowed.desktop" <<EOF
[Desktop Entry]
Version=1.0
Name=LocalAI Assistant (Ventana)
Comment=Local LLM chat interface — ventana normal
Exec=$INSTALL_DIR/start-windowed.sh
Icon=localaiassistant
Terminal=false
Type=Application
Categories=Utility;AI;
StartupWMClass=LocalAiAssistant
EOF

# Actualizar base de datos de aplicaciones
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$APPS_DIR" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi

echo ""
echo "✓ Instalado. Busca 'LocalAI Assistant' en el menú de aplicaciones."
echo "  Para desinstalar: $INSTALL_DIR/uninstall.sh"
INSTALL
chmod 755 "$LINUX_DIR/install.sh"

# Script de desinstalación
cat > "$LINUX_DIR/uninstall.sh" <<'UNINSTALL'
#!/usr/bin/env bash
set -euo pipefail
INSTALL_DIR="$HOME/.local/share/LocalAiAssistant"
rm -f "$HOME/.local/share/applications/localaiassistant.desktop"
rm -f "$HOME/.local/share/applications/localaiassistant-windowed.desktop"
rm -f "$HOME/.local/share/icons/hicolor/256x256/apps/localaiassistant.png"
rm -f "$HOME/.config/autostart/localaiassistant.desktop"
rm -rf "$INSTALL_DIR"
command -v update-desktop-database &>/dev/null && \
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
echo "✓ LocalAI Assistant desinstalado."
UNINSTALL
chmod 755 "$LINUX_DIR/uninstall.sh"
ok "install.sh / uninstall.sh escritos"

# README
cat > "$LINUX_DIR/README.txt" <<'README'
LocalAiAssistant — Linux
========================

Uso rápido:
  ./start.sh            — abre en modo kiosk (fullscreen, sin decoraciones)
  ./start-windowed.sh   — abre en ventana normal (para depuración)

Instalar en el sistema (menú de apps):
  ./install.sh          — instala en ~/.local/share/ y crea accesos directos
  ./uninstall.sh        — desinstala

Al primer arranque, la app pregunta si debe abrirse automáticamente al encender.

Datos almacenados en ~/.local-ai/
README
ok "README.txt escrito"

# ── 5. Empaquetar tarball ─────────────────────────────────────────────────────
info "Empaquetando tarball..."
mkdir -p "$DIST_DIR"
TAR_OUT="$DIST_DIR/${BUNDLE_NAME}-linux-amd64.tar.gz"
tar czf "$TAR_OUT" -C "$ROOT_DIR/build" "${BUNDLE_NAME}-linux/"
ok "tarball: $TAR_OUT ($(du -sh "$TAR_OUT" | cut -f1))"

# ── 6. Paquetes .deb / .rpm (instalación nativa) ─────────────────────────────
info "Construyendo paquetes nativos (.deb / .rpm)..."

DEB_STAGING="$ROOT_DIR/build/deb-staging"
rm -rf "$DEB_STAGING"

# Estructura FHS
USR_BIN="$DEB_STAGING/usr/bin"
USR_LIB="$DEB_STAGING/usr/lib/localaiassistant"
SHARE_APPS="$DEB_STAGING/usr/share/applications"
SHARE_ICONS_BASE="$DEB_STAGING/usr/share/icons/hicolor"
DEBIAN_DIR="$DEB_STAGING/DEBIAN"

mkdir -p "$USR_BIN" "$USR_LIB/web/dist" "$SHARE_APPS" "$DEBIAN_DIR"

# ── Binario
cp "$SERVE_RS" "$USR_LIB/serve-rs"
chmod 755 "$USR_LIB/serve-rs"

# ── Frontend embebido
cp -r "$WEB_DIST/." "$USR_LIB/web/dist/"

# ── Wrapper /usr/bin/localaiassistant
# Acepta --kiosk como argumento para lanzar en modo kiosk
cat > "$USR_BIN/localaiassistant" <<'WRAPPER'
#!/bin/bash
AI_BASE="$HOME/.local-ai"
mkdir -p "$AI_BASE/plugins" "$AI_BASE/workspace" "$AI_BASE/uploads"
exec /usr/lib/localaiassistant/serve-rs \
  --web-dist /usr/lib/localaiassistant/web/dist \
  --plugins-dir "$AI_BASE/plugins" "$@"
WRAPPER
chmod 755 "$USR_BIN/localaiassistant"

# ── Iconos — redimensionar desde llm-assistant.png
# Prioridad: ImageMagick convert → Python Pillow → copia directa (solo 256)
ICON_SRC="$SCRIPT_DIR/llm-assistant.png"
[ -f "$ICON_SRC" ] || ICON_SRC="$LINUX_DIR/icon.png"

resize_icon() {
    local SIZE=$1 OUT=$2
    mkdir -p "$(dirname "$OUT")"
    if command -v convert &>/dev/null; then
        convert "$ICON_SRC" -resize "${SIZE}x${SIZE}!" "$OUT" 2>/dev/null && return 0
    fi
    python3 - "$SIZE" "$ICON_SRC" "$OUT" <<'PYRESIZE' 2>/dev/null && return 0
import sys
try:
    from PIL import Image
    size = int(sys.argv[1])
    img  = Image.open(sys.argv[2]).convert("RGBA").resize((size, size))
    img.save(sys.argv[3])
except ImportError:
    import shutil; shutil.copy(sys.argv[2], sys.argv[3])
PYRESIZE
    cp "$ICON_SRC" "$OUT"  # fallback: copia sin redimensionar
}

for SIZE in 16 32 48 128 256 512; do
    resize_icon "$SIZE" "$SHARE_ICONS_BASE/${SIZE}x${SIZE}/apps/localaiassistant.png"
done
ok "Iconos generados (16 32 48 128 256 512 px)"

# ── Entrada de escritorio .desktop
cat > "$SHARE_APPS/localaiassistant.desktop" <<DESKTOP
[Desktop Entry]
Version=1.0
Name=LocalAI Assistant
GenericName=LLM Chat
Comment=Local LLM chat interface with plugin support
Exec=localaiassistant
Icon=localaiassistant
Terminal=false
Type=Application
Categories=Utility;Network;
Keywords=AI;LLM;chat;ollama;
StartupWMClass=LocalAiAssistant
DESKTOP

# ── DEBIAN/control
# Dependencias: wry/WebKitGTK puede ser 4.0 o 4.1 según distro
DEB_SIZE_KB=$(du -sk "$DEB_STAGING/usr" | cut -f1)
cat > "$DEBIAN_DIR/control" <<CONTROL
Package: localaiassistant
Version: ${VERSION}
Architecture: amd64
Maintainer: LocalAI Assistant <martin260602@gmail.com>
Description: LocalAI Assistant
 Plugin-first local LLM chat interface with WebKit UI.
 Connects to any OpenAI-compatible API (Ollama by default).
Section: utils
Priority: optional
Depends: libgtk-3-0, libwebkit2gtk-4.1-0 | libwebkit2gtk-4.0-37
Installed-Size: ${DEB_SIZE_KB}
CONTROL

# ── DEBIAN/postinst — actualizar base de datos de escritorio e iconos
cat > "$DEBIAN_DIR/postinst" <<'POSTINST'
#!/bin/bash
set -e
if [ "$1" = "configure" ]; then
    update-desktop-database /usr/share/applications/ 2>/dev/null || true
    gtk-update-icon-cache -f /usr/share/icons/hicolor/ 2>/dev/null || true
fi
POSTINST
chmod 755 "$DEBIAN_DIR/postinst"

cat > "$DEBIAN_DIR/prerm" <<'PRERM'
#!/bin/bash
set -e
PRERM
chmod 755 "$DEBIAN_DIR/prerm"

# Permisos FHS: directorios 755, ficheros no ejecutables 644
find "$DEB_STAGING/usr" -type d | xargs chmod 755
find "$DEB_STAGING/usr/lib/localaiassistant/web" -type f | xargs chmod 644
find "$DEB_STAGING/usr/share" -type f | xargs chmod 644
chmod 755 "$USR_LIB/serve-rs" "$USR_BIN/localaiassistant"

# ── Construir .deb
DEB_OUT="$DIST_DIR/localaiassistant_${VERSION}_amd64.deb"
if command -v dpkg-deb &>/dev/null; then
    dpkg-deb --build --root-owner-group "$DEB_STAGING" "$DEB_OUT" 2>/dev/null \
        && ok ".deb: $DEB_OUT ($(du -sh "$DEB_OUT" | cut -f1))" \
        || warn "dpkg-deb falló — revisa el staging en $DEB_STAGING"
else
    warn "dpkg-deb no encontrado — omitiendo .deb (instala: apt install dpkg)"
fi

# ── Construir .rpm (requiere fpm: gem install fpm)
if command -v fpm &>/dev/null; then
    RPM_OUT="$DIST_DIR/localaiassistant-${VERSION}-1.x86_64.rpm"
    POSTINST_RPM=$(mktemp)
    echo '#!/bin/bash' > "$POSTINST_RPM"
    echo 'update-desktop-database /usr/share/applications/ 2>/dev/null || true' >> "$POSTINST_RPM"
    echo 'gtk-update-icon-cache -f /usr/share/icons/hicolor/ 2>/dev/null || true' >> "$POSTINST_RPM"
    fpm -s dir -t rpm \
        --name localaiassistant \
        --version "$VERSION" \
        --architecture x86_64 \
        --description "LocalAI Assistant — local LLM chat interface" \
        --maintainer "LocalAI <martin260602@gmail.com>" \
        --category "Applications/Internet" \
        --depends "webkit2gtk3" \
        --after-install "$POSTINST_RPM" \
        --package "$RPM_OUT" \
        -C "$DEB_STAGING" \
        usr 2>/dev/null \
        && ok ".rpm: $RPM_OUT ($(du -sh "$RPM_OUT" | cut -f1))" \
        || warn "fpm (rpm) falló — el .deb sigue disponible"
    rm -f "$POSTINST_RPM"
else
    warn "fpm no encontrado — omitiendo .rpm  (instala: gem install fpm)"
fi

# ── 7. AppImage opcional ──────────────────────────────────────────────────────
if command -v appimagetool &>/dev/null; then
    info "Construyendo AppImage..."
    APPDIR="$ROOT_DIR/build/${BUNDLE_NAME}.AppDir"
    rm -rf "$APPDIR"
    mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/web/dist"

    cp "$SERVE_RS" "$APPDIR/usr/bin/serve-rs"
    chmod 755 "$APPDIR/usr/bin/serve-rs"
    cp -r "$WEB_DIST/." "$APPDIR/usr/share/web/dist/"

    cat > "$APPDIR/AppRun" <<'APPRUN'
#!/usr/bin/env bash
HERE="$(cd "$(dirname "$0")" && pwd)"
AI_BASE="$HOME/.local-ai"
mkdir -p "$AI_BASE/plugins" "$AI_BASE/workspace" "$AI_BASE/uploads"
exec "$HERE/usr/bin/serve-rs" \
  --web-dist "$HERE/usr/share/web/dist" \
  --plugins-dir "$AI_BASE/plugins"
APPRUN
    chmod 755 "$APPDIR/AppRun"

    cat > "$APPDIR/${BUNDLE_NAME}.desktop" <<DESKTOP
[Desktop Entry]
Name=LocalAiAssistant
Exec=AppRun
Icon=LocalAiAssistant
Type=Application
Categories=Utility;
DESKTOP

    # Icono — reutilizar el ya generado en el paso anterior
    cp "$LINUX_DIR/icon.png" "$APPDIR/${BUNDLE_NAME}.png"

    APPIMAGE_OUT="$DIST_DIR/${BUNDLE_NAME}-linux-amd64.AppImage"
    appimagetool "$APPDIR" "$APPIMAGE_OUT" &>/dev/null \
        && ok "AppImage: $APPIMAGE_OUT ($(du -sh "$APPIMAGE_OUT" | cut -f1))" \
        || warn "appimagetool falló — el tarball sigue disponible"
else
    warn "appimagetool no encontrado — omitiendo AppImage (solo se genera el tarball)"
fi

# ── 8. Resumen ────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${GREEN}  Artefactos generados en dist/${RESET}"
echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
ls -lh "$DIST_DIR/" 2>/dev/null | grep -v '^total' | awk '{print "  " $NF "  (" $5 ")"}' || true
echo ""
echo -e "${CYAN}  .deb  →  sudo dpkg -i localaiassistant_*.deb${RESET}"
echo -e "${CYAN}  .rpm  →  sudo rpm -i localaiassistant-*.rpm${RESET}"
echo -e "${CYAN}  tar   →  ./install.sh  (sin root, instala en ~/.local/share/)${RESET}"
echo -e "${CYAN}  Tras instalar el .deb/.rpm: busca 'LocalAI Assistant' en el lanzador${RESET}"
