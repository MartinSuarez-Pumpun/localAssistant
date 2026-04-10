# OLIV4600 Autonomous Agent — Session Instructions

You are working on the OLIV4600 plugin, a local AI document processing suite
built with Rust/Leptos/WASM, running inside a native macOS app (LocalAiAssistant).

## Your job this session

1. Read `TASKS.md` in this directory.
2. Find the first task with status `[ PENDING ]` whose dependencies (if any) are all `[ DONE ]`.
3. Change its status to `[ IN_PROGRESS ]` and save TASKS.md.
4. Implement the task completely — read the relevant files first, then write the code.
5. Run the build check specified in the task's Notes for Agents section.
6. If build passes: change status to `[ DONE ]`, add a note, save TASKS.md.
7. If build fails: fix the errors. If you cannot fix them after 2 attempts, mark `[ BLOCKED ]` with the error in the Note field, then try the next PENDING task.
8. If you have time and credits, repeat from step 2 with the next task.

## Project context

- **Backend**: Rust Axum server at `server/src/`
- **Frontend**: Leptos WASM plugin at `plugins/oliv4600-pack/src/lib.rs` (2500+ lines, single file)
- **Design system**: `First Objective/olivetti_modernist/DESIGN.md` — Olivetti modernist aesthetic, navy primary (#002542), no external APIs, 100% local
- **LLM**: Calls forwarded to Ollama at `settings.llm_endpoint` (default: `http://localhost:11434`)
- **State dir**: `~/.local-ai/`

## Critical rules

- No external API calls. Everything must work offline.
- Follow the existing code patterns — read the surrounding file before writing.
- Rust: match the existing error handling style (anyhow, no panics in handlers).
- Leptos: match the existing signal/component patterns in lib.rs.
- Run `cargo build` after backend changes, `trunk build` after frontend changes.
- Do NOT modify CLAUDE.md or this file (AGENT_PROMPT.md).
- Keep TASKS.md as the single source of truth for progress.
