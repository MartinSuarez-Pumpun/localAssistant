.PHONY: serve serve-dev web web-dev release release-macos clean help

# ── Colores ───────────────────────────────────────────────────────────────────
CYAN  := \033[36m
GREEN := \033[32m
RESET := \033[0m

# ── Desarrollo ────────────────────────────────────────────────────────────────

serve: web ## Build frontend + arrancar serve-rs en :8080
	@echo "$(CYAN)→ Build serve-rs$(RESET)"
	@cd server && source "$(HOME)/.cargo/env" && cargo build --release
	@echo "$(CYAN)→ Sirviendo en http://localhost:8080$(RESET)"
	@WEB_DIST=$(PWD)/web/dist \
	 PLUGINS_DIR=$(HOME)/.local-ai/plugins \
	 $(PWD)/server/target/release/serve-rs

serve-dev: web-dev ## Dev: frontend sin optimizar + serve-rs (debug)
	@echo "$(CYAN)→ Build serve-rs (debug)$(RESET)"
	@cd server && source "$(HOME)/.cargo/env" && cargo build
	@echo "$(CYAN)→ Sirviendo en http://localhost:8080$(RESET)"
	@WEB_DIST=$(PWD)/web/dist \
	 PLUGINS_DIR=$(HOME)/.local-ai/plugins \
	 $(PWD)/server/target/debug/serve-rs

web: ## Build frontend Leptos/WASM optimizado → web/dist/
	@echo "$(CYAN)→ Build frontend Leptos/WASM (release)$(RESET)"
	@cd web-app && source "$(HOME)/.cargo/env" && trunk build --release
	@echo "$(GREEN)✓ web/dist/$(RESET)"

web-dev: ## Build frontend Leptos/WASM sin optimizar → web/dist/
	@echo "$(CYAN)→ Build frontend Leptos/WASM (dev)$(RESET)"
	@cd web-app && source "$(HOME)/.cargo/env" && trunk build
	@echo "$(GREEN)✓ web/dist/$(RESET)"

# ── Release ───────────────────────────────────────────────────────────────────

release: release-macos ## Alias → release-macos

release-macos: ## Empaquetar .app + .dmg para macOS
	@echo "$(CYAN)→ Empaquetando macOS$(RESET)"
	@chmod +x scripts/package-macos.sh
	@bash scripts/package-macos.sh
	@echo "$(GREEN)✓ dist/ — artefactos macOS$(RESET)"

# ── Limpieza ──────────────────────────────────────────────────────────────────

clean: ## Limpiar builds (build/, web/dist/, dist/)
	@rm -rf build/ web/dist/ dist/
	@echo "$(GREEN)✓ Limpio$(RESET)"

# ── Ayuda ─────────────────────────────────────────────────────────────────────

help: ## Mostrar esta ayuda
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
	    awk 'BEGIN {FS = ":.*?## "}; {printf "$(CYAN)%-15s$(RESET) %s\n", $$1, $$2}'
