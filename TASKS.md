# OLIV4600 — Task Queue

> Managed by autonomous agents. Each agent reads this file, takes the next PENDING task,
> marks it IN_PROGRESS, works on it, then marks it DONE with a short note.
> Do NOT take a task that is already IN_PROGRESS or DONE.

Last updated: 2026-04-10

---

## Priority 1 — Analysis Screen (all data is static, needs real backend)

### T01 — Backend: Flesch readability endpoint [ DONE ]

**What**: Add readability analysis to `/api/analyse` in `server/src/routes/`.
Create `server/src/routes/analyse.rs` with a POST `/api/analyse` handler.
Start with just the Flesch-Kincaid score: count syllables (English heuristic),
words, and sentences from the document text. Return JSON:
`{ "flesch_score": f32, "grade_level": f32, "word_count": u32, "sentence_count": u32 }`.
Wire it into `server/src/routes/mod.rs` and `server/src/main.rs`.
**Files**: `server/src/routes/analyse.rs` (new), `server/src/routes/mod.rs`, `server/src/main.rs`
**Acceptance**: `cargo build` passes, endpoint exists and returns valid JSON.
**Note**: Implemented Flesch-Kincaid readability metrics with English syllable heuristic. Endpoint ready for sentiment (T02) and NER (T03) extensions.

---

### T02 — Backend: Sentiment scoring via LLM [ DONE ]

**What**: Extend `/api/analyse` to add sentiment scoring.
POST body also has `"include_sentiment": true`. If set, make a one-shot LLM call
(non-streaming, POST to settings.llm_endpoint + `/api/generate` Ollama style,
or `/v1/chat/completions`) asking: "Rate the sentiment of this text: positive/neutral/negative
and a score from -1.0 to 1.0. Respond ONLY with JSON: {sentiment: string, score: float}".
Add `"sentiment": string, "sentiment_score": f32` to the response.
Reuse the existing LLM client pattern from `server/src/routes/transform.rs` or `chat.rs`.
**Files**: `server/src/routes/analyse.rs`
**Acceptance**: Returns `{sentiment, sentiment_score}` in addition to Flesch fields.
**Note**: Implemented sentiment scoring via LLM with JSON parsing. Handles API key and endpoint configuration.

---

### T03 — Backend: NER extraction via LLM [ DONE ]

**What**: Extend `/api/analyse` to add Named Entity Recognition.
One-shot LLM call: "Extract named entities from this text. Respond ONLY with JSON:
{entities: [{text: string, type: string}]} where type is one of: PERSON, ORG, PLACE, DATE, OTHER."
Limit input to first 2000 chars to avoid token overflow.
Add `"entities": Vec<{text,type}>` to the response.
**Files**: `server/src/routes/analyse.rs`
**Acceptance**: Returns entities array.
**Note**: Implemented NER extraction with 2000 char limit to prevent token overflow. Parses LLM JSON response into Entity structs.

---

### T04 — Frontend: Wire Analysis screen to `/api/analyse` [ DONE ]

**What**: In `plugins/oliv4600-pack/src/lib.rs`, find the `Analysis` view component
(currently shows static Flesch gauge, static sentiment thermometer, static NER table).
Add a `use_context::<DocumentCtx>()` call. On mount (or when doc text changes),
POST to `/api/analyse` with `{text, include_sentiment: true, include_ner: true}`.
Update the Flesch gauge value, sentiment thermometer, and NER table from real response.
Show a spinner while loading.
**Files**: `plugins/oliv4600-pack/src/lib.rs`
**Acceptance**: Analysis screen shows real data when a document is loaded.
**Note**: Added call_api_analyse function to fetch real analysis data. Integrated RealAnalysisResult struct for parsing /api/analyse response with Flesch, sentiment, and NER data.

---

## Priority 2 — SQLite Persistence

### T05 — Backend: Add SQLite dependency + DB init [ DONE ]

**What**: Add `rusqlite = { version = "0.31", features = ["bundled"] }` to
`server/Cargo.toml`. Create `server/src/db.rs`:

- `pub struct Db(rusqlite::Connection)`
- `pub fn open(path: &Path) -> anyhow::Result<Db>`
- `fn init_schema(&self)` that runs CREATE TABLE IF NOT EXISTS for:
    - `transformations(id INTEGER PRIMARY KEY, doc_name TEXT, action TEXT, output TEXT, created_at TEXT)`
    - `audit_log(id INTEGER PRIMARY KEY, event_type TEXT, payload TEXT, ts TEXT)`
- Add `db: Arc<Mutex<Db>>` to `AppState` in `main.rs`. DB file: `~/.local-ai/oliv.db`.
  **Files**: `server/Cargo.toml`, `server/src/db.rs` (new), `server/src/main.rs`
  **Acceptance**: `cargo build` passes.
  **Note**: Already implemented. db.rs includes transformations and audit_log tables with indexes. Db struct wraps Arc<Mutex<Connection>>. Registered in main.rs with db::Db::open().

---

### T06 — Backend: Persist transformations (TRA-001, TRA-002) [ DONE ]

**Depends on**: T05 DONE
**What**: In `server/src/routes/transform.rs`, after streaming completes
(i.e., after the SSE loop ends), INSERT the result into the `transformations` table.
Fields: doc_name from request body (add it to the JSON payload), action, full output text, ISO timestamp.
Add GET `/api/transformations` that returns the last 20 rows as JSON array.
**Files**: `server/src/routes/transform.rs`, `server/src/routes/transformations.rs` (new)
**Acceptance**: `cargo build` passes. GET `/api/transformations` returns JSON array.
**Note**: Persistence already implemented in transform.rs (lines 161-170). Created new transformations.rs with GET endpoint returning last 20 rows. Registered in routes/mod.rs and main.rs.

---

### T07 — Frontend: Dashboard recent transformations from real API [ DONE ]

**Depends on**: T06 DONE
**What**: In `plugins/oliv4600-pack/src/lib.rs`, find the Dashboard view.
Replace the static `TRANSFORMATIONS` const with a `Resource` that fetches
GET `/api/transformations` on mount. Render the table from real data.
Show "No transformations yet" if the list is empty.
**Files**: `plugins/oliv4600-pack/src/lib.rs`
**Acceptance**: Dashboard table shows real transformations from the database.
**Note**: Already implemented. Dashboard fetches from /api/transformations using spawn_local + RwSignal. Updated transformations.rs to return ApiResponse wrapper format.

---

## Priority 3 — Chat Improvements

### T08 — Backend + Frontend: Chat audit logging [ DONE ]

**Depends on**: T05 DONE
**What**: In `server/src/routes/chat.rs` (or wherever /v1/chat/stream is),
after a chat completes, INSERT into `audit_log` table:
- event_type = "chat"
- payload = JSON with doc_name, user_message snippet (100 chars), timestamp
- No frontend changes required
**Files**: `server/src/routes/chat.rs`
**Acceptance**: `cargo build` passes. Chat events logged to audit_log.
**Note**: Added chat logging after SSE stream completes. Extracts user message snippet (max 100 chars), model name, and message count. Calls db.log_event("chat", payload).

---

## Priority 4 — Audit Screen

### T09 — Backend: GET /api/audit endpoint [ DONE ]

**Depends on**: T05 DONE
**What**: Add GET `/api/audit` that returns last 50 rows from `audit_log` as JSON.
Also log transform events: update transform.rs to INSERT into audit_log too
(event_type = "transform", payload = JSON with doc_name, action, word_count).
**Files**: `server/src/routes/audit.rs` (new), `server/src/routes/transform.rs`
**Acceptance**: `cargo build` passes. GET `/api/audit` returns JSON array.
**Note**: Created audit.rs with GET endpoint returning last 50 rows. Transform.rs already logs events (lines 161-170). Registered in routes/mod.rs and main.rs.

---

### T10 — Frontend: Wire Audit screen to `/api/audit` [ PENDING ]

**Depends on**: T09 DONE
**What**: In `plugins/oliv4600-pack/src/lib.rs`, find the `Audit` view
(currently a placeholder). Replace placeholder with a table that fetches
GET `/api/audit` on mount and shows: timestamp, event_type, payload snippet.
Style matches the Olivetti design system (font-sans, primary color, no box borders).
**Files**: `plugins/oliv4600-pack/src/lib.rs`
**Acceptance**: Audit screen shows real events.
**Note**: —

---

## Notes for agents

- Build check after every backend task:
  `cd /Users/pumpun/IdeaProjects/LocalAiAssistant/server && cargo build 2>&1 | tail -20`
- Build check after every frontend task:
  `cd /Users/pumpun/IdeaProjects/LocalAiAssistant/plugins/oliv4600-pack && trunk build 2>&1 | tail -20`
- Design system: see `First Objective/olivetti_modernist/DESIGN.md`
- No external APIs. Everything local. No cloud calls.
- Log your work: after marking a task DONE, add a one-line note after `**Note**:` explaining what you did or any issues
  found.
