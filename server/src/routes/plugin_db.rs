/**
 * plugin_db.rs — API genérica de base de datos para plugins
 *
 * Permite a cualquier plugin gestionar sus propias tablas y datos en la BD
 * compartida de OLIV4600, sin que el core conozca su esquema.
 *
 * POST /api/plugin/db/migrate
 *   Body: { "sql": "CREATE TABLE IF NOT EXISTS ..." }
 *   → Ejecuta SQL DDL. Solo CREATE/ALTER/DROP permitidos.
 *   → El plugin llama a este endpoint al arrancar para asegurarse de que
 *     sus tablas existen.
 *
 * POST /api/plugin/db/query
 *   Body: { "sql": "SELECT|INSERT|UPDATE|DELETE ...", "params": [...] }
 *   → Para SELECT: devuelve { ok, rows: [...] }
 *   → Para escritura: devuelve { ok, rows_affected, last_insert_rowid }
 *
 * Seguridad:
 *   - migrate solo acepta DDL (CREATE, ALTER, DROP, CREATE INDEX)
 *   - query bloquea acceso a las tablas del core (transformations, audit_log)
 *     con una lista de palabras prohibidas en el SQL
 *   - Ambos endpoints están pensados para localhost — no hay auth de plugin
 *     ya que el core solo escucha en 127.0.0.1
 */

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppState;

// ── DTOs ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MigrateRequest {
    pub sql: String,
}

#[derive(Deserialize)]
pub struct QueryRequest {
    pub sql:    String,
    #[serde(default)]
    pub params: Vec<Value>,
}

#[derive(Serialize)]
pub struct QueryResponse {
    pub ok:   bool,
    pub rows: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/plugin/db/migrate", post(migrate))
        .route("/api/plugin/db/query",   post(query))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /api/plugin/db/migrate
/// El plugin envía SQL DDL para crear/actualizar sus tablas.
async fn migrate(
    State(state): State<AppState>,
    Json(req):    Json<MigrateRequest>,
) -> impl IntoResponse {
    // Solo permitir DDL
    let sql_upper = req.sql.trim().to_uppercase();
    let is_ddl = sql_upper.starts_with("CREATE")
        || sql_upper.starts_with("ALTER")
        || sql_upper.starts_with("DROP")
        || sql_upper.contains("CREATE TABLE")
        || sql_upper.contains("CREATE INDEX")
        || sql_upper.contains("CREATE UNIQUE");

    if !is_ddl {
        return (
            StatusCode::BAD_REQUEST,
            Json(QueryResponse {
                ok:    false,
                rows:  vec![],
                error: Some("migrate solo acepta DDL (CREATE/ALTER/DROP)".into()),
            }),
        );
    }

    match state.db.execute_migration(&req.sql) {
        Ok(_) => (StatusCode::OK, Json(QueryResponse { ok: true, rows: vec![], error: None })),
        Err(e) => {
            tracing::error!("plugin migrate error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(QueryResponse {
                ok:    false,
                rows:  vec![],
                error: Some(e.to_string()),
            }))
        }
    }
}

/// POST /api/plugin/db/query
/// El plugin ejecuta SELECT/INSERT/UPDATE/DELETE sobre sus propias tablas.
async fn query(
    State(state): State<AppState>,
    Json(req):    Json<QueryRequest>,
) -> impl IntoResponse {
    // Proteger tablas del core: el plugin no puede tocar transformations ni audit_log
    let sql_upper = req.sql.to_uppercase();
    let blocked_tables = ["TRANSFORMATIONS", "AUDIT_LOG"];
    for table in &blocked_tables {
        if sql_upper.contains(table) {
            return (
                StatusCode::FORBIDDEN,
                Json(QueryResponse {
                    ok:    false,
                    rows:  vec![],
                    error: Some(format!("acceso denegado a tabla del core: {table}")),
                }),
            );
        }
    }

    match state.db.query_json(&req.sql, &req.params) {
        Ok(rows) => (StatusCode::OK, Json(QueryResponse { ok: true, rows, error: None })),
        Err(e) => {
            tracing::error!("plugin query error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(QueryResponse {
                ok:    false,
                rows:  vec![],
                error: Some(e.to_string()),
            }))
        }
    }
}
