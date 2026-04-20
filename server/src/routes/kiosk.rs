use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::AppState;

// ─── Estado de bloqueo (en memoria, por sesión) ───────────────────────────────

#[derive(Default)]
pub struct KioskLockout {
    pub fail_count:   u32,
    pub locked_until: u64, // unix timestamp en segundos
}

impl KioskLockout {
    /// Segundos de bloqueo según número acumulado de fallos.
    fn lockout_secs(fails: u32) -> u64 {
        match fails {
            0..=4  => 0,
            5..=9  => 30,       // 30 s
            10..=14 => 300,     // 5 min
            _      => 86_400,   // 24 h
        }
    }

    /// Intentos restantes antes del próximo bloqueo.
    fn attempts_left(fails: u32) -> u32 {
        5u32.saturating_sub(fails % 5)
    }
}

// ─── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct VerifyBody {
    pub pin: String,
}

#[derive(Deserialize)]
pub struct SetPinBody {
    pub pin:     String,
    pub old_pin: Option<String>,
}

#[derive(Serialize)]
pub struct VerifyResp {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_until: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_left: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct StatusResp {
    pub has_pin:  bool,
    pub is_kiosk: bool,
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/kiosk/status", get(get_status))
        .route("/api/kiosk/pin",    post(set_pin))
        .route("/api/kiosk/verify", post(verify_pin))
}

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn get_status(State(state): State<AppState>) -> impl IntoResponse {
    let s = state.settings.read().unwrap();
    Json(StatusResp {
        has_pin:  s.kiosk_pin_hash.is_some(),
        is_kiosk: state.is_kiosk,
    })
}

async fn set_pin(
    State(state): State<AppState>,
    Json(body): Json<SetPinBody>,
) -> impl IntoResponse {
    // Si ya hay un hash, exigir el PIN actual
    {
        let s = state.settings.read().unwrap();
        if let Some(ref existing) = s.kiosk_pin_hash {
            let old = match body.old_pin.as_deref() {
                Some(p) => p,
                None => return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Se requiere el PIN actual"})),
                ).into_response(),
            };
            if !pin_matches(old, existing) {
                log_attempt(&state, false, "SET_PIN_FAIL");
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "PIN actual incorrecto"})),
                ).into_response();
            }
        }
    }

    if body.pin.len() < 4 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "El PIN debe tener al menos 4 dígitos"})),
        ).into_response();
    }

    let hash = hash_pin(&body.pin);
    let path = state.config.settings_path();
    {
        let mut s = state.settings.write().unwrap();
        s.kiosk_pin_hash = Some(hash);
        if let Err(e) = s.save(&path) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            ).into_response();
        }
    }
    log_attempt(&state, true, "SET_PIN_OK");
    Json(serde_json::json!({"ok": true})).into_response()
}

async fn verify_pin(
    State(state): State<AppState>,
    Json(body): Json<VerifyBody>,
) -> impl IntoResponse {
    let now = unix_now();

    // Comprobar bloqueo activo
    {
        let lock = state.kiosk_lockout.lock().unwrap();
        if lock.locked_until > now {
            log_attempt(&state, false, "VERIFY_LOCKED");
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(VerifyResp {
                    ok:            false,
                    locked_until:  Some(lock.locked_until),
                    attempts_left: Some(0),
                    error:         Some("Cuenta bloqueada temporalmente".into()),
                }),
            ).into_response();
        }
    }

    let pin_hash = {
        let s = state.settings.read().unwrap();
        match s.kiosk_pin_hash.clone() {
            Some(h) => h,
            None => return (
                StatusCode::NOT_FOUND,
                Json(VerifyResp {
                    ok:            false,
                    locked_until:  None,
                    attempts_left: None,
                    error:         Some("PIN no configurado".into()),
                }),
            ).into_response(),
        }
    };

    if pin_matches(&body.pin, &pin_hash) {
        // Éxito: resetear contador
        {
            let mut lock = state.kiosk_lockout.lock().unwrap();
            lock.fail_count   = 0;
            lock.locked_until = 0;
        }
        log_attempt(&state, true, "VERIFY_OK");

        // Señalizar al event loop nativo para salir del modo kiosk
        if let Some(ref proxy) = state.event_proxy {
            proxy.send_event(crate::UserEvent::ExitKiosk).ok();
        }

        Json(VerifyResp { ok: true, locked_until: None, attempts_left: None, error: None })
            .into_response()
    } else {
        let (locked_until_abs, attempts_left) = {
            let mut lock = state.kiosk_lockout.lock().unwrap();
            lock.fail_count += 1;
            let secs = KioskLockout::lockout_secs(lock.fail_count);
            let until = if secs > 0 { now + secs } else { 0 };
            if secs > 0 {
                lock.locked_until = until;
            }
            (if secs > 0 { Some(until) } else { None },
             KioskLockout::attempts_left(lock.fail_count))
        };
        log_attempt(&state, false, "VERIFY_FAIL");

        (
            StatusCode::UNAUTHORIZED,
            Json(VerifyResp {
                ok:            false,
                locked_until:  locked_until_abs,
                attempts_left: Some(attempts_left),
                error:         Some("PIN incorrecto".into()),
            }),
        ).into_response()
    }
}

// ─── Utilidades ───────────────────────────────────────────────────────────────

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn hash_pin(pin: &str) -> String {
    let salt   = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(pin.as_bytes(), &salt)
        .expect("argon2 hash")
        .to_string()
}

fn pin_matches(pin: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(h)  => h,
        Err(_) => return false,
    };
    Argon2::default().verify_password(pin.as_bytes(), &parsed).is_ok()
}

/// Registra un intento en ~/.local-ai/kiosk_exit.log
fn log_attempt(state: &AppState, success: bool, event: &str) {
    let path = state.config.ai_base.join("kiosk_exit.log");
    let ts   = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%z");
    let line = format!("{} {} {}\n", ts, if success { "OK  " } else { "FAIL" }, event);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}
