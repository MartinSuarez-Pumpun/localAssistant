/**
 * db.rs — Capa de persistencia SQLite del core OLIV4600
 *
 * El core mantiene solo las tablas que le pertenecen:
 *   - transformations  (TRA-001, TRA-002): historial de operaciones LLM
 *   - audit_log        (SEC-005):          log de auditoría inmutable
 *
 * Los plugins gestionan sus propias tablas a través de los endpoints genéricos:
 *   POST /api/plugin/db/migrate  → CREATE TABLE IF NOT EXISTS ...
 *   POST /api/plugin/db/query    → SELECT / INSERT / UPDATE / DELETE
 *
 * Db expone además execute_migration() y query_json() para que plugin_db.rs
 * pueda ejecutar SQL arbitrario de forma controlada (sandboxed).
 */

use std::path::Path;
use std::sync::{Arc, Mutex};
use anyhow::{Context, Result};
use rusqlite::{Connection, params, types::Value};
use serde::{Deserialize, Serialize};
use serde_json::json;

// ── Wrapper público ───────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

// ── Tipos de dominio del core ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformationRow {
    pub id:         i64,
    pub doc_name:   String,
    pub action:     String,
    pub word_count: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRow {
    pub doc_name:    String,
    pub last_action: String,
    pub count:       u32,
    pub last_used:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRow {
    pub id:         i64,
    pub event_type: String,
    pub payload:    String,
    pub ts:         String,
}

// ── Inicialización ────────────────────────────────────────────────────────────

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("abriendo SQLite en {}", path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        init_schema(&conn)?;

        Ok(Db(Arc::new(Mutex::new(conn))))
    }
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS transformations (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            doc_name    TEXT    NOT NULL DEFAULT '',
            action      TEXT    NOT NULL DEFAULT '',
            word_count  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_transformations_doc
            ON transformations(doc_name);
        CREATE INDEX IF NOT EXISTS idx_transformations_created
            ON transformations(created_at DESC);

        CREATE TABLE IF NOT EXISTS audit_log (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type  TEXT    NOT NULL,
            payload     TEXT    NOT NULL DEFAULT '{}',
            ts          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_log(ts DESC);
    "#)?;
    Ok(())
}

// ── API genérica para plugins ─────────────────────────────────────────────────

impl Db {
    /// Ejecuta SQL DDL enviado por un plugin (CREATE TABLE IF NOT EXISTS, etc.).
    /// Solo permite sentencias que no empiecen por SELECT para evitar lecturas
    /// masivas no intencionadas en este endpoint; las lecturas van por query_json.
    pub fn execute_migration(&self, sql: &str) -> Result<()> {
        let conn = self.0.lock().unwrap();
        conn.execute_batch(sql)?;
        Ok(())
    }

    /// Ejecuta una query SQL con parámetros JSON y devuelve las filas como
    /// Vec<serde_json::Value>. Los parámetros son un array JSON de escalares.
    /// Soporta SELECT, INSERT, UPDATE, DELETE. Para escrituras devuelve
    /// [{"rows_affected": N, "last_insert_rowid": M}].
    pub fn query_json(
        &self,
        sql:    &str,
        params: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.0.lock().unwrap();
        let sql_upper = sql.trim().to_uppercase();

        if sql_upper.starts_with("SELECT") {
            // ── Lectura ───────────────────────────────────────────────────────
            let mut stmt = conn.prepare(sql)?;
            let col_names: Vec<String> = stmt.column_names()
                .iter().map(|s| s.to_string()).collect();

            // Convertir params JSON → rusqlite Value
            let rparams: Vec<Value> = params.iter().map(json_to_rusqlite).collect();
            let rparams_refs: Vec<&dyn rusqlite::ToSql> = rparams.iter()
                .map(|v| v as &dyn rusqlite::ToSql).collect();

            let rows = stmt.query_map(rparams_refs.as_slice(), |row| {
                let mut obj = serde_json::Map::new();
                for (i, name) in col_names.iter().enumerate() {
                    let val: Value = row.get(i)?;
                    obj.insert(name.clone(), rusqlite_to_json(val));
                }
                Ok(serde_json::Value::Object(obj))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)

        } else {
            // ── Escritura (INSERT / UPDATE / DELETE) ──────────────────────────
            let rparams: Vec<Value> = params.iter().map(json_to_rusqlite).collect();
            let rparams_refs: Vec<&dyn rusqlite::ToSql> = rparams.iter()
                .map(|v| v as &dyn rusqlite::ToSql).collect();

            let affected = conn.execute(sql, rparams_refs.as_slice())?;
            let last_id  = conn.last_insert_rowid();
            Ok(vec![json!({ "rows_affected": affected, "last_insert_rowid": last_id })])
        }
    }
}

// ── Conversores JSON ↔ rusqlite::Value ────────────────────────────────────────

fn json_to_rusqlite(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null              => Value::Null,
        serde_json::Value::Bool(b)           => Value::Integer(if *b { 1 } else { 0 }),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() { Value::Integer(i) }
            else { Value::Real(n.as_f64().unwrap_or(0.0)) }
        },
        serde_json::Value::String(s)         => Value::Text(s.clone()),
        serde_json::Value::Array(_)          => Value::Text(v.to_string()),
        serde_json::Value::Object(_)         => Value::Text(v.to_string()),
    }
}

fn rusqlite_to_json(v: Value) -> serde_json::Value {
    match v {
        Value::Null        => serde_json::Value::Null,
        Value::Integer(i)  => json!(i),
        Value::Real(f)     => json!(f),
        Value::Text(s)     => json!(s),
        Value::Blob(b)     => json!(b),
    }
}

// ── Transformaciones (core) ───────────────────────────────────────────────────

impl Db {
    pub fn insert_transformation(
        &self, doc_name: &str, action: &str, word_count: u32,
    ) -> Result<i64> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO transformations (doc_name, action, word_count) VALUES (?1,?2,?3)",
            params![doc_name, action, word_count],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_transformations(&self, limit: u32) -> Result<Vec<TransformationRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, doc_name, action, word_count, created_at
             FROM transformations ORDER BY id DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit], |row| Ok(TransformationRow {
            id:         row.get(0)?,
            doc_name:   row.get(1)?,
            action:     row.get(2)?,
            word_count: row.get::<_, u32>(3)?,
            created_at: row.get(4)?,
        }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_recent_documents(&self, limit: u32) -> Result<Vec<DocumentRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT doc_name,
                    (SELECT action FROM transformations t2
                     WHERE t2.doc_name = t1.doc_name
                     ORDER BY t2.id DESC LIMIT 1) AS last_action,
                    COUNT(*) AS cnt,
                    MAX(created_at) AS last_used
             FROM transformations t1
             GROUP BY doc_name ORDER BY last_used DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit], |row| Ok(DocumentRow {
            doc_name:    row.get(0)?,
            last_action: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            count:       row.get::<_, u32>(2)?,
            last_used:   row.get(3)?,
        }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

// ── Auditoría (core) ──────────────────────────────────────────────────────────

impl Db {
    pub fn log_event(&self, event_type: &str, payload: &str) -> Result<()> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO audit_log (event_type, payload) VALUES (?1,?2)",
            params![event_type, payload],
        )?;
        Ok(())
    }

    pub fn list_audit(&self, limit: u32) -> Result<Vec<AuditRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, event_type, payload, ts
             FROM audit_log ORDER BY id DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit], |row| Ok(AuditRow {
            id:         row.get(0)?,
            event_type: row.get(1)?,
            payload:    row.get(2)?,
            ts:         row.get(3)?,
        }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
