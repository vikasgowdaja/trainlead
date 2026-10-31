mod service;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tauri::{Manager, State};
use trainlead_core::{
    Error,
    extraction::GroqClient,
    models::{Cluster, IngestReport},
    rules::{FallbackExtractor, RuleExtractor},
};

struct AppState {
    directory: PathBuf,
    database: PathBuf,
    groq: Mutex<GroqState>,
    rules: Arc<RuleExtractor>,
}

#[derive(serde::Serialize)]
struct ExtractionResult {
    #[serde(flatten)]
    report: IngestReport,
    engine: String,
}

#[derive(Default)]
struct GroqState {
    client: Option<GroqClient>,
    source: Option<&'static str>,
    error: Option<String>,
}

#[derive(serde::Serialize)]
struct ExtractionStatus {
    enabled: bool,
    provider: &'static str,
    model: Option<String>,
    source: Option<&'static str>,
    detail: Option<String>,
}

fn message(error: Error) -> String {
    match error {
        Error::Configuration(detail) => detail,
        Error::Http(_) => "Could not reach Groq. Check the internet connection and try again.".into(),
        other => other.to_string(),
    }
}

fn client_from(key: &str) -> Result<GroqClient, Error> {
    GroqClient::new(key, std::env::var("GROQ_MODEL").ok(), std::env::var("GROQ_VISION_MODEL").ok())
}

/// A key saved in the app wins over GROQ_API_KEY from .env.
fn load_groq(directory: &Path) -> GroqState {
    let (key, source) = match service::saved_key(directory) {
        Some(key) => (key, "app"),
        None => match std::env::var("GROQ_API_KEY") {
            Ok(key) if !key.trim().is_empty() => (key, ".env"),
            _ => return GroqState { error: Some("No Groq API key".into()), ..Default::default() },
        },
    };
    match client_from(&key) {
        Ok(client) => GroqState { client: Some(client), source: Some(source), error: None },
        Err(error) => GroqState { error: Some(message(error)), ..Default::default() },
    }
}

fn groq(state: &AppState) -> Result<std::sync::MutexGuard<'_, GroqState>, String> {
    state.groq.lock().map_err(|_| "extraction settings are unavailable".to_string())
}

fn status_of(groq: &GroqState) -> ExtractionStatus {
    ExtractionStatus {
        enabled: groq.client.is_some(),
        provider: "Groq",
        model: groq.client.as_ref().map(GroqClient::text_model),
        source: groq.source,
        detail: groq.error.clone(),
    }
}

/// Groq when a key is configured, with on-device rules as the fallback (or the only engine).
fn extractor(state: &AppState) -> Result<Arc<FallbackExtractor<GroqClient>>, String> {
    let client = groq(state)?.client.clone();
    Ok(Arc::new(FallbackExtractor::new(client, state.rules.clone())))
}

#[tauri::command]
fn extraction_status(state: State<'_, AppState>) -> Result<ExtractionStatus, String> {
    Ok(status_of(&*groq(&state)?))
}

#[tauri::command]
async fn save_groq_key(state: State<'_, AppState>, key: String) -> Result<ExtractionStatus, String> {
    let key = key.trim().to_string();
    if !(20..=200).contains(&key.len()) || !key.chars().all(|character| character.is_ascii_graphic()) {
        return Err("That does not look like a Groq API key (it usually starts with gsk_)".into());
    }
    let client = client_from(&key).map_err(message)?;
    client.verify().await.map_err(message)?;
    service::save_key(&state.directory, &key).map_err(message)?;
    let mut groq = groq(&state)?;
    *groq = GroqState { client: Some(client), source: Some("app"), error: None };
    Ok(status_of(&groq))
}

#[tauri::command]
fn clear_groq_key(state: State<'_, AppState>) -> Result<ExtractionStatus, String> {
    service::remove_key(&state.directory).map_err(message)?;
    let mut groq = groq(&state)?;
    *groq = load_groq(&state.directory);
    Ok(status_of(&groq))
}

#[tauri::command]
async fn retry_review(state: State<'_, AppState>, ids: Option<Vec<i64>>) -> Result<ExtractionResult, String> {
    let extractor = extractor(&state)?;
    let path = state.database.clone();
    let engine = extractor.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(service::retry(&path, ids, extractor))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(message)?;
    Ok(ExtractionResult { report, engine: engine.engine() })
}

#[tauri::command]
async fn list_leads(state: State<'_, AppState>) -> Result<Vec<Cluster>, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || service::open(&path)?.clusters())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn capture_text(
    state: State<'_, AppState>,
    text: String,
    group: String,
    export: bool,
) -> Result<ExtractionResult, String> {
    let path = state.database.clone();
    let extractor = extractor(&state)?;
    let engine = extractor.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(service::capture(&path, &text, &group, export, extractor))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(message)?;
    Ok(ExtractionResult { report, engine: engine.engine() })
}

#[tauri::command]
async fn load_demo(state: State<'_, AppState>) -> Result<IngestReport, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(service::load_demo(&path))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn review_count(state: State<'_, AppState>) -> Result<usize, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service::open(&path)?
            .messages_by_status("needs_review")
            .map(|messages| messages.len())
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_reviews(
    state: State<'_, AppState>,
) -> Result<Vec<trainlead_core::store::StoredMessage>, String> {
    let path = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service::open(&path)?.messages_by_status("needs_review")
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            // Existing environment variables win; then the app-data .env, then a workspace .env (dev).
            let _ = dotenvy::from_path(directory.join(".env"));
            let _ = dotenvy::dotenv();
            app.manage(AppState {
                database: directory.join("trainlead.db"),
                groq: Mutex::new(load_groq(&directory)),
                rules: Arc::new(RuleExtractor::new()?),
                directory,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_leads,
            capture_text,
            load_demo,
            review_count,
            list_reviews,
            extraction_status,
            save_groq_key,
            clear_groq_key,
            retry_review
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("TrainLead could not start: {error}");
    }
}
