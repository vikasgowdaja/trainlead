mod service;

use std::path::PathBuf;

use tauri::{Manager, State};
use trainlead_core::{
    extraction::GroqClient,
    models::{Cluster, IngestReport},
};

struct AppState {
    database: PathBuf,
    groq: Option<GroqClient>,
    groq_error: Option<String>,
}

#[derive(serde::Serialize)]
struct ExtractionStatus {
    enabled: bool,
    provider: &'static str,
    model: Option<String>,
    detail: Option<String>,
}

#[tauri::command]
fn extraction_status(state: State<'_, AppState>) -> ExtractionStatus {
    ExtractionStatus {
        enabled: state.groq.is_some(),
        provider: "Groq",
        model: state
            .groq
            .as_ref()
            .map(|client| client.text_model().to_string()),
        detail: state.groq_error.clone(),
    }
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
) -> Result<IngestReport, String> {
    let path = state.database.clone();
    let groq = state.groq.clone();
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(service::capture(&path, &text, &group, export, groq))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())
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
            let (groq, groq_error) = match std::env::var("GROQ_API_KEY") {
                Ok(key) if !key.trim().is_empty() => match GroqClient::from_env() {
                    Ok(client) => (Some(client), None),
                    Err(error) => (None, Some(error.to_string())),
                },
                _ => (None, Some("GROQ_API_KEY is not set".to_string())),
            };
            app.manage(AppState {
                database: directory.join("trainlead.db"),
                groq,
                groq_error,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_leads,
            capture_text,
            load_demo,
            review_count,
            list_reviews,
            extraction_status
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("TrainLead could not start: {error}");
    }
}
