// routes/transform.rs
// POST /api/transform — Motor de transformación documental con prompts parametrizados (SRS §8.3)
//
// Body: {
//   "text":        String,         // texto del documento fuente
//   "action":      String,         // acción a realizar (ver build_prompt)
//   "length_words": u32,           // extensión objetivo en palabras (50-500, default 250)
//   "tone":        String,         // "1"..="5" (coloquial → formal institucional)
//   "audience":    String,         // "executive" | "technical" | "citizen" | "press"
//   "language":    String,         // "es" | "en" | "gl" | "ca" | "eu" | "pt"
// }
//
// Response: SSE stream con los mismos eventos que /v1/chat/stream:
//   event: token    data: {"text":"..."}
//   event: done     data: {}
//   event: error    data: {"message":"..."}

use axum::{
    Router,
    extract::State,
    routing::post,
    response::{IntoResponse, Response},
    http::{StatusCode, HeaderMap, header},
    body::Body,
};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::error;

use crate::AppState;

// ─── Tipos de petición ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TransformRequest {
    pub text:         String,
    pub action:       String,
    /// Nombre del archivo fuente (para persistencia TRA-001)
    #[serde(default)]
    pub doc_name:     String,
    #[serde(default = "default_length")]
    pub length_words: u32,
    #[serde(default = "default_tone")]
    pub tone:         String,
    #[serde(default = "default_audience")]
    pub audience:     String,
    #[serde(default = "default_language")]
    pub language:     String,
}

fn default_length()   -> u32    { 250 }
fn default_tone()     -> String { "4".to_string() }
fn default_audience() -> String { "technical".to_string() }
fn default_language() -> String { "es".to_string() }

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new().route("/api/transform", post(transform_handler))
}

// ─── Handler ──────────────────────────────────────────────────────────────────

async fn transform_handler(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<TransformRequest>,
) -> Response {
    let (tx, rx) = mpsc::channel::<String>(64);

    let snapshot  = state.settings.read().unwrap().clone();
    let endpoint  = snapshot.llm_endpoint.clone();
    let model     = if snapshot.llm_model.is_empty() { "llama3".to_string() } else { snapshot.llm_model.clone() };
    let api_key   = snapshot.api_key.clone();
    let db        = state.db.clone();
    let doc_name  = if req.doc_name.is_empty() { "Sin título".to_string() } else { req.doc_name.clone() };
    let action    = req.action.clone();
    let word_count = req.text.split_whitespace().count() as u32;

    let (system_prompt, user_msg) = build_prompt(&req);

    tokio::spawn(async move {
        let messages = vec![
            serde_json::json!({"role": "system", "content": system_prompt}),
            serde_json::json!({"role": "user",   "content": user_msg}),
        ];

        let client = reqwest::Client::new();
        let body   = serde_json::json!({
            "model":    model,
            "messages": messages,
            "stream":   true,
        });

        let mut req_builder = client
            .post(format!("{endpoint}/v1/chat/completions"))
            .json(&body);
        if !api_key.is_empty() {
            req_builder = req_builder.header("Authorization", format!("Bearer {api_key}"));
        }

        let resp = match req_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                error!("transform LLM error: {e}");
                let _ = tx.send(format!(
                    "event: error\ndata: {}\n\n",
                    serde_json::to_string(&serde_json::json!({"message": e.to_string()})).unwrap()
                )).await;
                let _ = tx.send("event: done\ndata: {}\n\n".into()).await;
                return;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text   = resp.text().await.unwrap_or_default();
            let _ = tx.send(format!(
                "event: error\ndata: {}\n\n",
                serde_json::to_string(&serde_json::json!({
                    "message": format!("LLM {status}: {text}")
                })).unwrap()
            )).await;
            let _ = tx.send("event: done\ndata: {}\n\n".into()).await;
            return;
        }

        let mut stream  = resp.bytes_stream();
        let mut buf     = String::new();
        let mut full_output = String::new(); // TRA-001: acumulamos salida completa

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c)  => c,
                Err(e) => { error!("stream error: {e}"); break; }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf = buf[pos + 1..].to_string();

                if line.is_empty() || line == "data: [DONE]" { continue; }

                let json_str = line.strip_prefix("data: ").unwrap_or(&line);
                let Ok(val)  = serde_json::from_str::<serde_json::Value>(json_str) else { continue; };

                if let Some(token) = val["choices"][0]["delta"]["content"].as_str() {
                    if !token.is_empty() {
                        full_output.push_str(token);
                        let _ = tx.send(format!(
                            "event: token\ndata: {}\n\n",
                            serde_json::to_string(&serde_json::json!({"text": token})).unwrap()
                        )).await;
                    }
                }
            }
        }

        // ── TRA-001 / TRA-002: Persistir transformación completada ────────────
        if !full_output.is_empty() {
            let _ = db.insert_transformation(&doc_name, &action, word_count);
            let payload = serde_json::json!({
                "doc_name": doc_name,
                "action":   action,
                "words_in": word_count,
            }).to_string();
            let _ = db.log_event("transform", &payload);
        }

        let _ = tx.send("event: done\ndata: {}\n\n".into()).await;
    });

    let stream = ReceiverStream::new(rx).map(|s| Ok::<_, std::convert::Infallible>(s));
    let body   = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    headers.insert("X-Accel-Buffering", "no".parse().unwrap());

    (StatusCode::OK, headers, body).into_response()
}

// ─── Mapeo de parámetros a etiquetas ─────────────────────────────────────────

fn tone_label(tone: &str) -> &'static str {
    match tone {
        "1" => "coloquial y cercano",
        "2" => "periodístico e informativo",
        "3" => "divulgativo y accesible",
        "4" => "técnico y especializado",
        "5" => "formal institucional",
        _   => "profesional y claro",
    }
}

fn audience_label(audience: &str) -> &'static str {
    match audience {
        "executive" => "directivos y responsables de decisión",
        "technical"  => "público técnico y especializado",
        "citizen"    => "ciudadanía general",
        "press"      => "periodistas y medios de comunicación",
        _            => "público general",
    }
}

// ─── Motor de prompts parametrizados (SRS §8.3) ───────────────────────────────
//
// Variables inyectadas: {texto_fuente}, {longitud_objetivo}, {tono},
// {idioma}, {publico_objetivo}.
// Nuevas acciones se añaden aquí sin modificar nada más.

fn build_prompt(req: &TransformRequest) -> (String, String) {
    let tone     = tone_label(&req.tone);
    let audience = audience_label(&req.audience);
    let lang     = &req.language;
    let len      = req.length_words;
    let text     = &req.text;

    // Sistema base compartido por todas las acciones
    let base_system = format!(
        "Eres OLIV4600, un procesador documental de inteligencia soberana. \
         Opera en modo 100% local, sin conexión a internet. \
         Responde siempre en el idioma indicado ({lang}). \
         Tono objetivo: {tone}. \
         Público objetivo: {audience}. \
         Extensión máxima objetivo: {len} palabras. \
         No añadas comentarios sobre lo que vas a hacer ni frases de introducción. \
         Responde directamente con el resultado solicitado, sin preámbulo."
    );

    let (action_instr, user_msg): (&str, String) = match req.action.as_str() {

        // ── Módulo 2: Resúmenes Avanzados (RES-001..RES-009) ─────────────────
        "executive_summary" => (
            "Genera un RESUMEN EJECUTIVO de 1 párrafo, orientado a la toma de decisión. \
             Destaca el dato más relevante, la acción recomendada y el impacto principal.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "technical_summary" => (
            "Genera un RESUMEN TÉCNICO que conserve todos los datos, cifras, terminología \
             sectorial y referencias precisas. No simplifiques conceptos técnicos.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "divulgative_summary" => (
            "Genera un RESUMEN DIVULGATIVO en lenguaje simple y accesible, \
             evitando tecnicismos. Apto para ciudadanía general sin conocimiento previo.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "bullet_summary" => (
            "Extrae los PUNTOS CLAVE del documento en formato de lista con viñetas. \
             Máximo 8 puntos. Cada punto debe ser una frase corta y directa.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "chronological_summary" => (
            "Genera un RESUMEN CRONOLÓGICO ordenando los eventos y hechos mencionados \
             en orden temporal. Indica fechas o períodos cuando estén disponibles.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "conclusions_summary" => (
            "Genera un resumen con CONCLUSIONES Y RECOMENDACIONES claras. \
             Separa en dos secciones: '## Conclusiones' y '## Recomendaciones'.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "briefing_2min" => (
            "Genera un BRIEFING DE 2 MINUTOS en formato oral. \
             Usa marcas de pausa [PAUSA] entre ideas principales. \
             Máximo 300 palabras. Adecuado para leer en voz alta en presentación.",
            format!("DOCUMENTO:\n\n{text}"),
        ),

        // ── Módulo 3: Generación de Contenidos Derivados (GEN-001..GEN-011) ──
        "press_release" => (
            "Redacta una NOTA DE PRENSA profesional con estructura de pirámide invertida: \
             1. Titular impactante \
             2. Entradilla (quién, qué, cuándo, dónde, por qué) en 2-3 frases \
             3. Cuerpo con información relevante \
             4. Boilerplate institucional al final. \
             Usa estilo periodístico formal.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "headlines" => (
            "Genera entre 5 y 10 TITULARES para prensa sobre este contenido. \
             Varía el enfoque: informativo directo, llamativo/impactante, SEO optimizado. \
             Numera cada titular. Sin explicaciones adicionales.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "linkedin_post" => (
            "Redacta un POST DE LINKEDIN profesional y efectivo. \
             Estructura: gancho inicial (primera línea impactante), \
             desarrollo en párrafos cortos, conclusión/llamada a la acción, \
             y 5 hashtags relevantes al final. Tono profesional pero humano.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "twitter_thread" => (
            "Crea un HILO DE TWITTER/X. Máximo 280 caracteres por tweet. \
             Numera los tweets (1/, 2/, etc.). \
             El primer tweet debe ser el gancho. Mínimo 5 tweets.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "blog_article" => (
            "Escribe un ARTÍCULO DE BLOG con estructura SEO: \
             título H1, introducción, secciones con subtítulos H2, conclusión. \
             Palabras clave integradas de forma natural.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "instagram_post" => (
            "Redacta un POST DE INSTAGRAM atractivo. \
             Texto de máximo 2200 caracteres, emojis estratégicos, \
             y 10-15 hashtags relevantes al final agrupados.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "email_newsletter" => (
            "Redacta un EMAIL INSTITUCIONAL / NEWSLETTER informativa. \
             Primera línea: Asunto sugerido (Asunto: ...). \
             Después: saludo formal, cuerpo estructurado, cierre y firma.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "speech" => (
            "Redacta un DISCURSO / INTERVENCIÓN PÚBLICA en formato oral. \
             Usa marcas de pausa [PAUSA], énfasis [ÉNFASIS: texto] y \
             cambio de ritmo [PAUSA LARGA]. Estructura: apertura, desarrollo, cierre.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "faqs" => (
            "Genera una lista de PREGUNTAS FRECUENTES (FAQs) basadas en este contenido. \
             Formato: **Pregunta** seguida de respuesta en párrafo. \
             Mínimo 5, máximo 10. Ordena de más a menos relevante.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "one_pager" => (
            "Genera una FICHA RESUMEN / ONE-PAGER estructurada. \
             Secciones: ¿Qué es?, Puntos clave (bullets), Datos relevantes, \
             Próximos pasos. Formato compacto y visual con markdown.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "key_quotes" => (
            "Extrae las CITAS TEXTUALES más reutilizables del documento. \
             Incluye: frases citables para prensa, destacados para publicación, \
             recuadros para maquetación. Formato: '\"cita literal\" — contexto'.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),

        // ── Módulo 4: Administración Pública (ADM-001..ADM-009) ──────────────
        "official_report" => (
            "Genera un INFORME OFICIAL con estructura institucional estándar: \
             portada (título, fecha, organismo), antecedentes, objeto, \
             análisis, conclusiones y firma. Lenguaje jurídico-administrativo formal.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "meeting_minutes" => (
            "Genera un ACTA DE REUNIÓN formal. \
             Incluye: fecha, asistentes (si se mencionan), orden del día, \
             acuerdos adoptados, puntos pendientes y próxima reunión si se indica.",
            format!("NOTAS / TRANSCRIPCIÓN:\n\n{text}"),
        ),
        "administrative_resolution" => (
            "Genera una RESOLUCIÓN ADMINISTRATIVA con formato legal. \
             Partes: encabezado, antecedentes, fundamentos de derecho, \
             parte dispositiva (RESUELVO:), recursos y firma.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),
        "internal_memo" => (
            "Redacta un MEMORANDO INTERNO formal. \
             Cabecera: PARA / DE / FECHA / ASUNTO. \
             Cuerpo: antecedentes, objeto, resolución/recomendación, cierre.",
            format!("CONTENIDO BASE:\n\n{text}"),
        ),
        "allegations_response" => (
            "Modo NEGOCIACIÓN/ALEGACIONES. Redacta una respuesta formal con: \
             1. Alegaciones punto por punto \
             2. Comparativa propuesta vs observaciones recibidas \
             3. Versión final con cambios justificados.",
            format!("PROPUESTA / OBSERVACIONES:\n\n{text}"),
        ),
        "extract_commitments" => (
            "Extrae los COMPROMISOS VERIFICABLES del documento en tabla: \
             | Quién | Se compromete a | Para cuándo | Dependencia | Criterio de cumplimiento |",
            format!("DOCUMENTO:\n\n{text}"),
        ),

        // ── Módulo 5: Tono, Estilo e Idioma (TON-001..TON-006) ───────────────
        "rewrite_formal" => (
            "REESCRIBE el texto con tono más FORMAL e INSTITUCIONAL. \
             Mantén el significado exacto. Elimina coloquialismos, usa voz activa.",
            format!("TEXTO A REESCRIBIR:\n\n{text}"),
        ),
        "rewrite_shorter" => (
            "REESCRIBE el texto de forma más CONCISA. \
             Elimina redundancias y relleno. Conserva toda la información clave.",
            format!("TEXTO A REESCRIBIR:\n\n{text}"),
        ),
        "rewrite_persuasive" => (
            "REESCRIBE el texto de forma más PERSUASIVA Y CONVINCENTE. \
             Usa lógica, ejemplos concretos y llamadas a la acción. Mantén veracidad.",
            format!("TEXTO A REESCRIBIR:\n\n{text}"),
        ),
        "rewrite_clearer" => (
            "REESCRIBE el texto de forma más CLARA Y COMPRENSIBLE. \
             Frases cortas, lenguaje sencillo, explica términos técnicos si los hay.",
            format!("TEXTO A REESCRIBIR:\n\n{text}"),
        ),
        "detect_redundancies" => (
            "Detecta REDUNDANCIAS en el texto. \
             1. Lista de redundancias encontradas con fragmento y descripción. \
             2. Versión del texto con redundancias eliminadas.",
            format!("TEXTO:\n\n{text}"),
        ),
        "translate_language" => (
            &format!("TRADUCE el texto al idioma indicado ({lang}). \
             Preserva el registro y el tono del original. \
             Si hay términos técnicos o institucionales, úsalos correctamente en el idioma destino."),
            format!("TEXTO A TRADUCIR:\n\n{text}"),
        ),
        "sentiment_analysis" => (
            "Realiza un ANÁLISIS DE SENTIMIENTO Y TONO EMOCIONAL del texto. \
             Puntúa en escala 0-100: hostilidad, formalidad, persuasión, urgencia. \
             Termómetro emocional: ¿suena alarmista, frío, cercano, neutro? Justifica.",
            format!("TEXTO:\n\n{text}"),
        ),

        // ── Módulo 6: Edición Inteligente Asistida (EDI-001..EDI-007) ────────
        "grammar_check" => (
            "CORRIGE todos los errores ortográficos y gramaticales. \
             Mejora también puntuación y estructura sintáctica. \
             Devuelve el texto corregido completo, sin comentarios.",
            format!("TEXTO A CORREGIR:\n\n{text}"),
        ),
        "simplify" => (
            "SIMPLIFICA el texto aplicando principios de LENGUAJE CLARO (UNE 153101). \
             Frases máx. 20 palabras, voz activa, términos comunes, sin nominalizaciones.",
            format!("TEXTO:\n\n{text}"),
        ),
        "detect_inconsistencies" => (
            "Detecta INCONSISTENCIAS: fechas contradictorias, nombres que cambian, \
             datos que se contradicen, afirmaciones que se anulan. \
             Lista cada caso con la referencia al texto original.",
            format!("TEXTO:\n\n{text}"),
        ),
        "reformulate_paragraph" => (
            "REFORMULA el texto manteniendo el mismo significado \
             con redacción completamente diferente: estructura, orden e ideas.",
            format!("TEXTO A REFORMULAR:\n\n{text}"),
        ),
        "detect_ambiguities" => (
            "Detecta AMBIGÜEDADES y COMPROMISOS VACÍOS: frases sin sujeto claro, \
             compromisos sin plazo, afirmaciones sin cifras, responsables sin nombre, \
             promesas sin criterio de cumplimiento.",
            format!("TEXTO:\n\n{text}"),
        ),
        "improve_suggestions" => (
            "Genera SUGERENCIAS DE MEJORA concretas sobre claridad, \
             estructura y coherencia del texto. Lista numerada con: \
             problema detectado, fragmento afectado y sugerencia de corrección.",
            format!("TEXTO:\n\n{text}"),
        ),

        // ── Módulo 7: Análisis Forense del Texto (FOR-001..FOR-005) ──────────
        "readability_analysis" => (
            "Realiza un ANÁLISIS DE LEGIBILIDAD completo: \
             1. Índice Flesch-Szigriszt estimado (0-100) \
             2. Longitud media de frase (palabras) \
             3. Densidad léxica estimada (%) \
             4. Nivel lector recomendado \
             5. Recomendaciones de mejora.",
            format!("TEXTO:\n\n{text}"),
        ),
        "detect_evasive_language" => (
            "Detecta LENGUAJE EVASIVO: pasivas que ocultan responsables, \
             condicionales excesivos, generalizaciones sin datos, eufemismos. \
             Cita cada caso con el fragmento original.",
            format!("TEXTO:\n\n{text}"),
        ),
        "semantic_versioning" => (
            "Realiza un VERSIONADO SEMÁNTICO entre los dos textos. \
             Clasifica cada cambio como: tono, compromiso, alcance, político/jurídico o cosmético. \
             Genera resumen ejecutivo de los cambios de mayor impacto.",
            format!("TEXTOS A COMPARAR (separados por ---VERSIÓN 2---):\n\n{text}"),
        ),

        // ── Módulo 8: Aritmética Textual (ARI-001..ARI-006) ──────────────────
        "merge_documents" => (
            "FUSIONA INTELIGENTEMENTE los dos textos eliminando redundancias. \
             El resultado debe ser un documento único coherente, sin repeticiones.",
            format!("DOCUMENTOS A FUSIONAR (separados por ---DOC 2---):\n\n{text}"),
        ),
        "semantic_diff" => (
            "Genera un DIFERENCIAL SEMÁNTICO entre las dos versiones: \
             qué cambió en el significado (no solo en las palabras). \
             Clasifica: añadido, eliminado, modificado, reformulado.",
            format!("VERSIONES (separadas por ---V2---):\n\n{text}"),
        ),
        "document_intersection" => (
            "Determina la INTERSECCIÓN entre los documentos: \
             qué información aparece en todos ellos (hechos, conclusiones, datos comunes). \
             Lista solo lo que está en todos, no lo específico de cada uno.",
            format!("DOCUMENTOS (separados por ---DOC N---):\n\n{text}"),
        ),
        "detect_contradictions" => (
            "Detecta CONTRADICCIONES Y DESACUERDOS al combinar los textos. \
             1. Hechos comunes vs conflictivos \
             2. Propuesta de versión conservadora que no contradiga ninguno \
             3. Puntos que requieren validación humana.",
            format!("TEXTOS (separados por ---FUENTE N---):\n\n{text}"),
        ),
        "versions_compare" => (
            "Compara las versiones y genera un RESUMEN EJECUTIVO DE CAMBIOS. \
             ¿Qué se añadió? ¿Qué se eliminó? ¿Qué cambió de sentido? \
             ¿Cuál es el impacto de los cambios?",
            format!("VERSIONES (separadas por ---V2---):\n\n{text}"),
        ),

        // ── Módulo 9: Motor de Preguntas Inversas (INV-001..INV-005) ─────────
        "inverse_questions" => (
            "Actúa como EDITOR JEFE. Analiza el documento y genera: \
             1. Información FALTANTE que debería incluir \
             2. Checklist de validación humana pendiente \
             3. Alertas de tono inadecuado \
             4. Sugerencias de mejora proactivas antes de publicar.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "press_release_check" => (
            "Verifica si la nota de prensa tiene toda la información necesaria. \
             Checkea: titular, entradilla (5W), cuerpo, cita directa, boilerplate, \
             datos de contacto. Indica qué falta y qué mejorar.",
            format!("NOTA DE PRENSA:\n\n{text}"),
        ),
        "validation_questions" => (
            "Genera un CHECKLIST DE VALIDACIÓN HUMANA para este documento. \
             Preguntas concretas que el responsable debe confirmar antes de publicar: \
             ¿Aforo confirmado? ¿Cargo oficial correcto? ¿Cifra publicable? ¿Fecha definitiva?",
            format!("DOCUMENTO:\n\n{text}"),
        ),

        // ── Módulo 10: Extracción Estructurada y Metadatos (EXT-001..EXT-007)
        "ner_extraction" => (
            "Extrae todas las ENTIDADES del texto en formato tabla markdown: \
             | Tipo | Entidad | Contexto | \
             Tipos: PERSONA, ORGANIZACIÓN, FECHA, LUGAR, IMPORTE, NORMATIVA. \
             También incluye: eventos clave, decisiones y acciones pendientes.",
            format!("TEXTO:\n\n{text}"),
        ),
        "keywords_extraction" => (
            "Extrae 5-10 PALABRAS CLAVE Y CATEGORÍAS TEMÁTICAS del documento. \
             Formato: lista con importancia (alta/media) y breve justificación. \
             Sugiere también el nivel de confidencialidad recomendado.",
            format!("TEXTO:\n\n{text}"),
        ),
        "event_timeline" => (
            "Genera una LÍNEA TEMPORAL de todos los eventos mencionados. \
             Ordena cronológicamente. Formato: FECHA — Evento — Relevancia. \
             Si no hay fecha exacta, usa el período indicado.",
            format!("TEXTO:\n\n{text}"),
        ),
        "impact_analysis" => (
            "Analiza el IMPACTO del documento: \
             1. ¿Por qué importa? \
             2. ¿A quién afecta directamente? \
             3. ¿Consecuencias si no se actúa? \
             4. ¿Oportunidades que plantea? \
             Respuesta en 4 párrafos cortos.",
            format!("DOCUMENTO:\n\n{text}"),
        ),

        // ── Módulo 13: Verificabilidad y Soporte a Fuente (VER-001..VER-005) ─
        "verifiability_check" => (
            "Analiza cada afirmación y clasifícala: \
             ✅ SUSTENTADA — respaldada por datos del propio texto \
             ⚠️ INFERIDA — conclusión razonable pero no explícita \
             ❌ NO SOPORTADA — afirmación sin base en el texto. \
             Genera informe con porcentajes y lista por categoría.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "evidence_gaps" => (
            "Detecta HUECOS DE EVIDENCIA: \
             - Afirmaciones de impacto sin dato de respaldo \
             - Beneficios declarados sin fuente \
             - Medidas sin órgano competente \
             - Conclusiones sin base explícita. \
             Lista cada hueco con fragmento y pregunta sin respuesta.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "traceability_map" => (
            "Genera un MAPA DE TRAZABILIDAD del documento. \
             Para cada párrafo de conclusión, indica qué parte del texto fuente lo sustenta. \
             Formato: Afirmación → Origen en el texto (cita literal o párrafo N).",
            format!("DOCUMENTO:\n\n{text}"),
        ),

        // ── Módulo 14: Publicación Segura (PUB-001..PUB-006) ─────────────────
        "anonymize" => (
            "Detecta y PROPONE ANONIMIZACIÓN de datos personales: \
             nombres, DNI/NIF, direcciones, teléfonos, emails, cuentas bancarias. \
             Para cada dato: a) [REDACTADO] b) Generalización c) Seudónimo. \
             Presenta el texto con sustituciones aplicadas.",
            format!("TEXTO:\n\n{text}"),
        ),
        "preflight_check" => (
            "PREFLIGHT DOCUMENTAL antes de publicar/enviar: \
             1. Siglas sin desarrollar \
             2. Fechas ambiguas \
             3. Nombres inconsistentes \
             4. Cifras sin unidad \
             5. Tono incorrecto para canal institucional \
             6. Referencias a adjuntos inexistentes. \
             Puntúa la publicabilidad del 1 al 10 con justificación.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "public_version" => (
            "Genera una VERSIÓN PÚBLICA del documento: \
             elimina datos personales, información sensible interna \
             y cualquier contenido no apto para difusión externa. \
             Señala cada cambio con [EXPURGADO].",
            format!("DOCUMENTO ORIGINAL:\n\n{text}"),
        ),
        "rgpd_check" => (
            "Verifica el cumplimiento RGPD/LOPDGDD del documento. \
             1. Datos personales encontrados y base jurídica de tratamiento \
             2. Riesgos identificados \
             3. Recomendaciones de corrección.",
            format!("DOCUMENTO:\n\n{text}"),
        ),

        // ── Módulo 15: Inteligencia Editorial (INT-001..INT-005) ─────────────
        "style_linting" => (
            "Actúa como VALIDADOR DE ESTILO INSTITUCIONAL. \
             Detecta: tono demasiado comercial, jerga inadecuada para resolución, \
             lenguaje de marketing en comunicación institucional. \
             Lista cada caso con alternativa correcta.",
            format!("DOCUMENTO:\n\n{text}"),
        ),
        "reader_simulation" => (
            "SIMULA cómo diferentes lectores interpretarán este texto. \
             Perspectivas: directivo, técnico, ciudadanía, prensa, oposición, jurídico. \
             Para cada uno: qué entendería, qué malinterpretaría, qué preguntas surgirían.",
            format!("TEXTO:\n\n{text}"),
        ),

        // ── Módulo 16: Constructor Guiado desde Formulario (GUI-001..GUI-004) ─
        "generate_from_form" => (
            "A partir de los datos estructurados proporcionados, \
             genera un PAQUETE COMPLETO de comunicación: \
             nota de prensa, FAQ, email, minuta y resumen ejecutivo. \
             Separa cada sección con un encabezado claro.",
            format!("DATOS DEL FORMULARIO:\n\n{text}"),
        ),

        // ── Módulo 17: Paquete de Expediente (EXP-001..EXP-005) ──────────────
        "generate_file_package" => (
            "Genera la estructura de un EXPEDIENTE COMPLETO: \
             1. Índice de contenidos \
             2. Portada (título, fecha, organismo, clasificación) \
             3. Documento principal (reformateado institucionalmente) \
             4. Registro de cambios \
             5. Notas de auditoría.",
            format!("DOCUMENTO FUENTE:\n\n{text}"),
        ),

        // ── Módulo 18: Modo Crisis y Comparecencia (CRI-001..CRI-005) ────────
        "crisis_press_questions" => (
            "SIMULACRO DE COMPARECENCIA DE PRENSA. Genera: \
             1. 10 preguntas previsibles de periodistas \
             2. 5 preguntas hostiles o incómodas \
             3. Respuestas prudentes para las 3 más difíciles \
             4. Líneas rojas de comunicación (qué NO decir).",
            format!("DOCUMENTO / INCIDENTE:\n\n{text}"),
        ),
        "crisis_communication" => (
            "Modo CRISIS REPUTACIONAL. Kit completo: \
             1. Comunicado inicial (2-3 párrafos) \
             2. Q&A de contención (5 preguntas y respuestas) \
             3. Versión interna para empleados \
             4. Mensaje para redes sociales (máx. 280 caracteres) \
             5. Riesgos reputacionales identificados.",
            format!("INCIDENTE / CONTEXTO:\n\n{text}"),
        ),
        "argumentario" => (
            "Genera un ARGUMENTARIO completo: \
             1. Puntos clave — mensajes fuerza (máx. 5) \
             2. Rebatimientos para las 5 objeciones más probables \
             3. Datos de soporte para cada argumento \
             4. Mensaje central en una frase.",
            format!("CONTENIDO BASE:\n\n{text}"),
        ),
        "difficult_questions_simulator" => (
            "SIMULADOR DE PREGUNTAS DIFÍCILES. \
             Genera 10 preguntas complicadas con respuestas sugeridas \
             graduadas por nivel de riesgo: BAJO / MEDIO / ALTO. \
             Para cada una: respuesta recomendada, argumentos de soporte, riesgos.",
            format!("CONTEXTO / DOCUMENTO:\n\n{text}"),
        ),

        // ── Fallback ──────────────────────────────────────────────────────────
        other => {
            tracing::warn!("acción desconocida: {other}, usando fallback");
            (
                "Procesa el siguiente texto según las instrucciones del sistema.",
                format!("TEXTO:\n\n{text}"),
            )
        }
    };

    let system = format!("{base_system}\n\nINSTRUCCIÓN ESPECÍFICA: {action_instr}");
    // /no_think desactiva el chain-of-thought interno de Qwen 3,
    // evitando que los tokens <channel|> / <think>…</think> lleguen al cliente.
    let user_msg_final = format!("{user_msg}\n\n/no_think");
    (system, user_msg_final)
}
