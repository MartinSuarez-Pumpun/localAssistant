# OLIV4600 — Task Queue

> Managed by autonomous agents. Each agent reads this file, takes the next PENDING task,
> marks it IN_PROGRESS, works on it, then marks it DONE with a short note.
> Do NOT take a task that is already IN_PROGRESS or DONE.

Last updated: 2026-04-20 (rev 3 — queue wiped, new bug backlog)

---

## Priority 1 — Render bugs (PDF/DOCX)

### T19 — Render: fix overlapping text and overflow lines [ PENDING ]

**Bug**: Documents generated via `/api/export/render` sometimes show text rendered on top
of previous text, or lines that run past the page margin instead of wrapping.

**Scope**: `server/src/routes/render_pdf.rs` and `server/src/routes/render_docx.rs`.
The pipeline is pure Rust (docx-rs for DOCX, printpdf for PDF) — no Node/LibreOffice.

**Likely culprits**:

- PDF: `Renderer` doesn't advance `y` after every draw, or wraps without measuring string
  width against page width. Check `draw_text` / wrap helpers: width measurement probably
  assumes a monospaced metric instead of asking the font for actual glyph widths.
- PDF: page breaks may not reset `y`; the second page overdraws the first.Es un documento válido paraE
- DOCX: table cells or long code blocks may lack wrapping properties.

**Reproduce**: POST to `/api/export/render` with markdown containing a very long paragraph
(>400 chars without line breaks) and a code block. Inspect the resulting PDF visually.

**Acceptance**: No visual overlap; all lines stay within the printable area; multi-page
content paginates cleanly.

**Note**: —

---

## Priority 2 — Chat bugs

### T20 — Chat: response sometimes stuck inside thinking block [ DONE ]

**Bug**: On both the plain chat (`web-app/src/chat.rs`) and the plugin chat
(`plugins/oliv4600-pack/src/lib.rs` → `ChatView`), the final assistant response sometimes
stays rendered inside the collapsed "Razonamiento" block, so the user sees nothing in the
main bubble.

**Context**: The backend emits a `promote_reasoning` SSE event when it detects the model
never emitted `</think>` and never used `reasoning_content`. The frontend must move the
buffered reasoning into `content` on that event. See `server/src/routes/chat.rs` for the
detection logic and `web-app/src/chat.rs` for the promotion handler.

**Likely causes**:

- `promote_reasoning` not wired in the plugin chat (only in `web-app/src/chat.rs`).
- Detection heuristic misses some models that emit reasoning without the `<think>` tag.
- The server closes the stream on `done` before emitting `promote_reasoning`.

**Acceptance**: With a non-thinking model (plain GPT-style), the final answer always
appears in the main content bubble, never trapped inside the reasoning panel — in both
the plain chat and the plugin chat.

**Note**: Root cause: `promote_reasoning` could be missed if the backend round ended abnormally (error, tool loop) and
`has_think_end` had been set earlier. Added a `done`/`error` safety net in `web-app/src/chat.rs` that promotes
`reasoning → content` when content is empty; plugin ChatView was already safe because its SSE parser treats any event
carrying `text` (token or reasoning) as chat content.

---

## Priority 3 — File I/O inside plugins

### T21 — Plugins: native upload/export without the app chat modal [ PENDING ]

**Bug**: To upload or export files from inside a plugin, the user has to open the app's
chat file modal, which only shows external volumes and the app folder. This forces an
awkward round-trip.

**Goal**: The plugin should be able to trigger file pick / file save on its own, reaching
any folder the user can access.

**Design options to consider** (agent picks one):

1. Expose `POST /api/upload` and `POST /api/export/save-as` to plugins with a path
   argument, and use a WebView-native picker via `wry` bridge from the host.
2. Use the browser `<input type="file">` + `showSaveFilePicker()` if the embedded WebView
   supports it, bypassing the app modal entirely.
3. Add a plugin-scoped endpoint `GET /api/plugin/{id}/file-dialog?mode=open|save` that
   shells out to `zenity`/`osascript` for a real native dialog.

**Files**: likely `server/src/routes/upload.rs`, `server/src/routes/export.rs`,
`server/src/main.rs`, and plugin JS/WASM shims.

**Acceptance**: From `plugins/oliv4600-pack`, user can upload from Documents/Downloads
and export anywhere on disk without touching the app's chat modal.

**Note**: —

---

## Priority 4 — Project management

### T22 — Projects: delete project option [ PENDING ]

**What**: Add a "Delete project" action.

**Backend**:

- New endpoint `DELETE /api/plugin/{plugin_id}/projects/{doc_hash}` (or a generic
  `POST /api/plugin/db/query` with the right DELETE SQL — check what the plugin DB API
  already exposes).
- Remove the row from `oliv_projects`.
- Remove the associated folder in `~/.local-ai/workspace/{doc_hash}/` (see T23 for the
  new per-project layout).
- Log a `log_event("project_delete", ...)` entry.

**Frontend** (`plugins/oliv4600-pack/src/lib.rs`):

- Trash icon on each row of the Sidebar recent-projects list + confirm modal.
- After success, increment `ctx.refresh_projects` so the list re-queries.
- If the deleted project is the one currently open in the Editor, clear `ctx.doc_hash` /
  `ctx.text` / `ctx.filename`.

**Acceptance**: User can delete a project; row disappears from the sidebar, workspace
folder is gone from disk, and open editor state is reset if it referenced the deleted one.

**Note**: —

---

### T23 — Projects: per-project folder keeps a copy of every generated output [ PENDING ]

**Context**: Right now a transform output only persists if the user hits "Export" and
chooses a destination, or via the background save from T13/T14 which writes flat files
to `~/.local-ai/workspace/{doc_hash}/`. The requirement is stronger: every project should
have a real folder that accumulates the original upload AND every generated artifact
(press release, summary, etc.), even when the user also exports elsewhere.

**Target layout**:

```
~/.local-ai/projects/{doc_hash}/
  original.{ext}                          ← the uploaded source
  meta.json                               ← { filename, uploaded_at, original_path }
  outputs/
    press_release_20260420T103000Z.docx
    press_release_20260420T103000Z.pdf
    resumen_ejecutivo_20260420T111500Z.txt
    ...
```

**Work**:

1. **Backend**: update upload handler (`server/src/routes/upload.rs`) to copy the source
   into `~/.local-ai/projects/{doc_hash}/original.{ext}` and write `meta.json`.
2. **Backend**: update `/api/export/render` (`server/src/routes/render.rs`) to ALSO write
   a copy into `~/.local-ai/projects/{doc_hash}/outputs/` when the request carries a
   `doc_hash` (extend `RenderRequest` with `doc_hash: Option<String>`). The user-chosen
   export path stays as today.
3. **Backend**: update the `workspace/save` endpoint from T13 to write to the new
   `projects/{doc_hash}/outputs/` path (keep `workspace/` as a legacy alias or migrate).
4. **Frontend**: pass `doc_hash` in every render request from the plugin.
5. **Frontend**: add a "Open project folder" button in the Editor header that opens the
   OS file manager at `~/.local-ai/projects/{doc_hash}/` (reuse `reveal_in_files`).
6. **T22 integration**: `DELETE` must recursively drop the whole folder.

**Acceptance**: After uploading a doc and running a transform + export, the folder
`~/.local-ai/projects/{doc_hash}/outputs/` contains a copy of the rendered file (DOCX or
PDF) regardless of what path the user exported to.

**Note**: —

---

---

## Priority 2 — Chat bugs

### T24 — Chat: thinking content dumped into output panel after sessions [ PENDING ]

**Bug**: After a chat finishes (both plain app chat and plugin chat), the accumulated "thinking"/reasoning text is being placed into the main output box (the place where the final answer should appear), cluttering the output with internal reasoning and irrelevant tokens.

**Scope**: `web-app/src/chat.rs`, plugin chat view in `plugins/oliv4600-pack/src/lib.rs` (ChatView), and `server/src/routes/chat.rs` (SSE event handling).

**Likely causes**:

- The SSE `promote_reasoning` event is promoting reasoning into the wrong UI slot (output panel) instead of merging it properly with the assistant content.
- The final content-promotion safety net fires but copies the entire reasoning buffer rather than only the assistant's final message.
- Plugin chat SSE parser and the plain chat handler diverge in how they classify reasoning vs final content.

**Reproduce**: Start a chat with a model that emits reasoning tokens (or triggers the thinking flow). Let the session finish or force an early stream close. Observe that the output box (not the collapsed "Razonamiento" panel) contains the verbose reasoning content.

**Acceptance**: After session end, the main output box shows only the assistant's final reply; internal reasoning remains in the collapsed reasoning panel (or is omitted entirely). This must hold for both the plain chat UI and the plugin chat view.

**Note**: —

---

## Priority 3 — Plugin bugs

### T25 — Plugin Analysis: Export Report returns HTTP 500 [ PENDING ]

**Bug**: From the plugin Analysis screen, using the "Export Report" action yields a server error (HTTP 500) instead of returning a generated report or a downloadable file.

**Scope**: Plugin frontend (`plugins/oliv4600-pack` Analysis screen), server plugin export handlers (likely `server/src/routes/plugins.rs` or the plugin-specific export route), and any backend export/render path used by the plugin.

**Likely causes**:

- Backend handler throws an unhandled error (missing fields, null pointer, or serialization failure) when building the report payload.
- The plugin frontend sends malformed request parameters, or a required field (e.g., `doc_hash` or analysis id) is missing.
- File generation code fails for certain inputs and bubbles up as a 500.

**Reproduce**: Open the plugin Analysis screen in the app, click "Export Report", confirm that DevTools / network shows a 500 response, and inspect server logs for a stack trace.

**Acceptance**: "Export Report" completes with HTTP 200 (or triggers a native save/download) and produces the expected report file. No 500 responses for valid analysis inputs.

**Note**: —

---

### T26 — Plugin Audit: Export JSON button no-op [ PENDING ]

**Bug**: In the plugin's Audit screen, clicking the "Export JSON" button does nothing (no network request, no file download, no error message).

**Scope**: `plugins/oliv4600-pack` Audit UI code and any client-side handlers that should trigger JSON export; possible server-side export endpoint if the client calls one.

**Likely causes**:

- The click handler is not wired up to the button (missing onClick or broken selector).
- A JavaScript/wasm runtime error prevents the handler from running; errors are swallowed silently.
- The frontend attempts to call an endpoint that doesn't exist or returns immediately with no side effects.

**Reproduce**: Open plugin Audit screen, click "Export JSON", observe no network activity and no download. Check browser console for JS/WASM errors.

**Acceptance**: Clicking "Export JSON" triggers a file download (or save dialog) containing the audit JSON, or opens a save-as flow. The action should surface an error message if the export fails.

**Note**: —


### T27 — Chat init: reload currently resumes previous session; add "New conversation" + explicit restore flow [ PENDING ]

**Bug / request**: On app init (page load / native restart) the plain chat may automatically restore the previous conversation, so a page reload is not a reliable way to start a fresh session. Add a clear "New conversation" button and an explicit "Restore previous conversation" opt-in flow.

**Scope**: `web-app/src/chat.rs` (chat UI & state), client persistence layers (localStorage/sessionStorage/indexedDB), any native app restore logic (wry window state), and server-side session handling (cookies/session ids, any persisted conversation ID returned by the server).

**Observed behaviour**:

- Reloading often brings back the prior messages, making it appear the conversation continued across reloads.

**Likely causes**:

- Client-side state (conversation id or message history) is automatically rehydrated on init.
- Server associates requests with a persistent conversation id (cookie or stored token) and returns the previous context.
- No user-visible control exists to explicitly clear or choose whether to restore prior state on startup.

**Reproduce**: Start a chat, reload the page or quit+reopen the native app, observe that previous messages are visible and the conversation continues.

**Work / Implementation notes**:

1. Add a prominent "New conversation" button in the chat header that:
   - Clears client-side conversation state (remove keys from localStorage/sessionStorage/indexedDB).
   - Resets any in-memory conversation id and sends a lightweight API call (e.g., `POST /api/chat/new` or `POST /api/chat/clear`) to ensure server-side context is dropped or a new conversation id is issued.
   - Optionally shows a confirmation modal if there is unsaved content.
2. On app init, do NOT auto-rehydrate conversations by default. Instead, if previous state exists show a small banner: "Restore previous conversation" with a button to restore. Restoring should be explicit and gated by user action.
3. If the app must support an automatic restore preference, add a persisted setting "Auto-restore last conversation" (off by default) in settings; when off, init does not rehydrate.
4. Update tests/docs and ensure plugin chat behavior is considered separately (plugins may intentionally persist state).

**Acceptance**:

- Clicking "New conversation" clears UI and client/server conversation state; subsequent messages belong to a fresh conversation id.
- Page reload or native restart does not silently bring back the previous conversation unless the user explicitly clicks "Restore previous conversation" or has enabled "Auto-restore" in settings.
- Behaviour is consistent across web-app and native packaged app.

**Note**: —


### T28 — Plugin Editor: cannot type into editor or add content after upload [ PENDING ]

**Bug**: In the plugin Editor screen (plugins/oliv4600-pack), new documents start empty but typing a character freezes the editor. Uploading a file shows the file contents but the user cannot insert or append text afterwards — the editor becomes effectively read-only.

**Scope**: `plugins/oliv4600-pack` Editor UI (likely `plugins/oliv4600-pack/src/lib.rs`), editor initialization and event bindings, any WASM/JS shims that handle input events, and file upload handlers that replace editor state.

**Observed behaviour**:

- Creating a "Nuevo Documento" and typing a single character causes the editor to lock; further input is ignored.
- Uploading a document populates the editor view, but attempting to modify the content is not allowed.

**Likely causes**:

- Editor component's `readonly` flag or equivalent is being set erroneously after first input or after upload.
- Input event handlers (key events, composition events) are swallowed by a higher-level overlay or by a failing WASM callback.
- State update replaces the editor model with an immutable snapshot or resets cursor position in a way that prevents further edits.
- Race condition between upload completion and editor rehydration that leaves the editor detached from its input handlers.

**Reproduce**: Open plugin Editor, create "Nuevo Documento", type one character — observe freeze. Upload a file and try to modify it — observe inability to type or append.

**Work / Investigation notes**:

- Inspect editor initialization code in `lib.rs` and any helper modules handling contenteditable or textareas. Add logging around setReadonly/setEditable calls.
- Check browser console for WASM panics or JS exceptions during first keypress or upload flow.
- Verify that the upload flow doesn't replace the editor element DOM node without reattaching handlers.

**Acceptance**: The Editor allows normal typing and editing for new documents and uploaded documents. No freeze occurs after a single keypress; uploads populate content and remain editable.

**Note**: —


## Notes for agents

- Build check after a backend task:
  `cd /home/aistudio/LocalAssistant/server && cargo build 2>&1 | tail -20`
- Build check after a frontend task (plain chat UI):
  `cd /home/aistudio/LocalAssistant/web-app && trunk build 2>&1 | tail -20`
- Build check after a plugin task:
  `cd /home/aistudio/LocalAssistant/plugins/oliv4600-pack && trunk build 2>&1 | tail -20`
- Design system: see `First Objective/olivetti_modernist/DESIGN.md`.
- No external APIs. Everything local. No cloud calls.
- After marking a task DONE, replace `**Note**: —` with a one-line summary of what you did
  or any issues found.
- The plugin frontend (`lib.rs`) is a single large file (~3800 lines). Read the section
  you need before editing.
- Generic plugin DB API: `POST /api/plugin/db/migrate` and `POST /api/plugin/db/query` —
  use these from the frontend instead of direct SQLite calls.
- The export pipeline is 100% Rust (docx-rs + printpdf). No Node, no LibreOffice. Do not
  reintroduce shell-outs.
