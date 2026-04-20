.PHONY: serve serve-dev serve-kiosk web web-dev release release-macos release-linux clean help

# ── Colores ───────────────────────────────────────────────────────────────────
CYAN  := \033[36m
GREEN := \033[32m
RESET := \033[0m

# ── Desarrollo ────────────────────────────────────────────────────────────────

serve: web ## Build frontend + arrancar serve-rs en :8080
	@echo "$(CYAN)→ Build serve-rs$(RESET)"
	@cd server && PATH="$(HOME)/.cargo/bin:$$PATH" cargo build --release
	@echo "$(CYAN)→ Sirviendo en http://localhost:8080$(RESET)"
	@WEB_DIST=$(PWD)/web/dist \
	 PLUGINS_DIR=$(HOME)/.local-ai/plugins \
	 $(PWD)/server/target/release/serve-rs

serve-kiosk: web ## Build frontend + arrancar serve-rs en modo kiosk (fullscreen)
	@echo "$(CYAN)→ Build serve-rs$(RESET)"
	@cd server && PATH="$(HOME)/.cargo/bin:$$PATH" cargo build --release
	@echo "$(CYAN)→ Sirviendo en modo kiosk$(RESET)"
	@WEB_DIST=$(PWD)/web/dist \
	 PLUGINS_DIR=$(HOME)/.local-ai/plugins \
	 $(PWD)/server/target/release/serve-rs --kiosk

serve-dev: web-dev ## Dev: frontend sin optimizar + serve-rs (debug)
	@echo "$(CYAN)→ Build serve-rs (debug)$(RESET)"
	@cd server && PATH="$(HOME)/.cargo/bin:$$PATH" cargo build
	@echo "$(CYAN)→ Sirviendo en http://localhost:8080$(RESET)"
	@WEB_DIST=$(PWD)/web/dist \
	 PLUGINS_DIR=$(HOME)/.local-ai/plugins \
	 $(PWD)/server/target/debug/serve-rs

web: ## Build frontend Leptos/WASM optimizado → web/dist/
	@echo "$(CYAN)→ Build frontend Leptos/WASM (release)$(RESET)"
	@cd web-app && PATH="$(HOME)/.cargo/bin:$$PATH" trunk build --release
	@echo "$(GREEN)✓ web/dist/$(RESET)"

web-dev: ## Build frontend Leptos/WASM sin optimizar → web/dist/
	@echo "$(CYAN)→ Build frontend Leptos/WASM (dev)$(RESET)"
	@cd web-app && PATH="$(HOME)/.cargo/bin:$$PATH" trunk build
	@echo "$(GREEN)✓ web/dist/$(RESET)"

# ── Release ───────────────────────────────────────────────────────────────────

release: ## Empaquetar para la plataforma actual (macOS → .dmg, Linux → .tar.gz)
	@if [ "$$(uname)" = "Darwin" ]; then $(MAKE) release-macos; \
	 elif [ "$$(uname)" = "Linux"  ]; then $(MAKE) release-linux; \
	 else echo "Plataforma no soportada: $$(uname)"; exit 1; fi

release-macos: ## Empaquetar .app + .dmg para macOS
	@echo "$(CYAN)→ Empaquetando macOS$(RESET)"
	@chmod +x scripts/package-macos.sh
	@bash scripts/package-macos.sh
	@echo "$(GREEN)✓ dist/ — artefactos macOS$(RESET)"

release-linux: ## Empaquetar tarball autocontenido para Linux
	@echo "$(CYAN)→ Empaquetando Linux$(RESET)"
	@chmod +x scripts/package-linux.sh
	@PATH="$(HOME)/.cargo/bin:$$PATH" bash scripts/package-linux.sh
	@echo "$(GREEN)✓ dist/ — artefactos Linux$(RESET)"

# ── Limpieza ──────────────────────────────────────────────────────────────────

clean: ## Limpiar builds (build/, web/dist/, dist/)
	@rm -rf build/ web/dist/ dist/
	@echo "$(GREEN)✓ Limpio$(RESET)"

# ── Ayuda ─────────────────────────────────────────────────────────────────────

help: ## Mostrar esta ayuda
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
	    awk 'BEGIN {FS = ":.*?## "}; {printf "$(CYAN)%-15s$(RESET) %s\n", $$1, $$2}'
