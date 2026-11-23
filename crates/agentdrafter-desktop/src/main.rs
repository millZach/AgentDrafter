//! AgentDrafter Desktop: a thin Tauri v2 shell over the in-process local
//! AgentDrafter HTTP server (`agentdrafter_cli::http::start_local`).
//!
//! No drawing logic lives here (ADR-0007, ADR-0011): this binary starts the
//! core server on `127.0.0.1:0` with a per-session random bearer token, opens a
//! native window showing the embedded viewer page, and exposes native OS
//! folder/file pickers, persisted recents, multi-window launching, and window
//! title updates to the page.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod state;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use agentdrafter_cli::http::{start_local, LocalServer};
use agentdrafter_core::Settings;
use state::{InspectedPath, RecentEntry, SourceKind};
use tauri::{
    AppHandle, Manager, RunEvent, State, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;

struct DesktopState {
    base_url: String,
    token: String,
    server: Mutex<Option<LocalServer>>,
    recents_file: PathBuf,
    recents_lock: Mutex<()>,
    next_window_id: AtomicUsize,
}

impl DesktopState {
    fn shutdown_server(&self) {
        if let Ok(mut guard) = self.server.lock() {
            if let Some(mut server) = guard.take() {
                server.shutdown();
            }
        }
    }
}

#[tauri::command]
async fn pick_drawing_set_folder(app: AppHandle) -> Result<Option<InspectedPath>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Open drawing set folder (containing set.yaml)")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| format!("folder picker failed: {e}"))?;

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("selected folder is not a local path: {e}"))?;
    state::inspect_path(&path).map(Some)
}

#[tauri::command]
async fn pick_drawing_file(
    app: AppHandle,
    filter: Option<String>,
) -> Result<Option<InspectedPath>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file();
        let dialog = match filter.as_deref() {
            Some("dwg") => dialog
                .set_title("Open plant DWG original")
                .add_filter("AutoCAD DWG", &["dwg", "DWG"]),
            Some("yaml") => dialog
                .set_title("Open single sheet description")
                .add_filter("Sheet description YAML", &["yaml", "yml"]),
            _ => dialog
                .set_title("Open plant DWG or single sheet description")
                .add_filter("Drawings & Sheets", &["dwg", "DWG", "yaml", "yml"])
                .add_filter("AutoCAD DWG", &["dwg", "DWG"])
                .add_filter("Sheet description YAML", &["yaml", "yml"]),
        };
        dialog.blocking_pick_file()
    })
    .await
    .map_err(|e| format!("file picker failed: {e}"))?;

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|e| format!("selected file is not a local path: {e}"))?;
    state::inspect_path(&path).map(Some)
}

#[tauri::command]
fn inspect_path(path: String) -> Result<InspectedPath, String> {
    state::inspect_path(std::path::Path::new(&path))
}

#[tauri::command]
fn list_recents(desktop: State<'_, DesktopState>) -> Result<Vec<RecentEntry>, String> {
    let _guard = desktop
        .recents_lock
        .lock()
        .map_err(|_| "recents lock poisoned".to_owned())?;
    Ok(state::load_recents(&desktop.recents_file))
}

#[tauri::command]
fn record_recent(
    desktop: State<'_, DesktopState>,
    path: String,
    kind: SourceKind,
    name: String,
    subtitle: Option<String>,
) -> Result<Vec<RecentEntry>, String> {
    let _guard = desktop
        .recents_lock
        .lock()
        .map_err(|_| "recents lock poisoned".to_owned())?;
    let mut entries = state::load_recents(&desktop.recents_file);
    state::record_recent(
        &mut entries,
        RecentEntry {
            path,
            kind,
            name,
            opened_at_ms: state::now_ms(),
            subtitle,
        },
    );
    state::save_recents(&desktop.recents_file, &entries)?;
    Ok(entries)
}

#[tauri::command]
fn remove_recent(
    desktop: State<'_, DesktopState>,
    path: String,
) -> Result<Vec<RecentEntry>, String> {
    let _guard = desktop
        .recents_lock
        .lock()
        .map_err(|_| "recents lock poisoned".to_owned())?;
    let mut entries = state::load_recents(&desktop.recents_file);
    state::remove_recent(&mut entries, &path);
    state::save_recents(&desktop.recents_file, &entries)?;
    Ok(entries)
}

#[tauri::command]
fn open_in_new_window(
    app: AppHandle,
    desktop: State<'_, DesktopState>,
    source: Option<String>,
    title: Option<String>,
) -> Result<String, String> {
    let id = desktop.next_window_id.fetch_add(1, Ordering::SeqCst);
    let label = format!("win-{id}");
    let url_str = state::window_url(
        &desktop.base_url,
        &desktop.token,
        source.as_deref(),
        source.is_none(),
    );
    let url = Url::parse(&url_str).map_err(|e| format!("invalid local window URL: {e}"))?;
    let win_title = state::window_title(title.as_deref());
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title(&win_title)
        .inner_size(1280.0, 820.0)
        .min_inner_size(390.0, 600.0)
        .build()
        .map_err(|e| format!("could not open new window: {e}"))?;
    Ok(label)
}

#[tauri::command]
fn set_window_title(window: WebviewWindow, title: String) -> Result<(), String> {
    let formatted = state::window_title(Some(&title));
    window
        .set_title(&formatted)
        .map_err(|e| format!("could not update window title: {e}"))
}

fn main() {
    let launch_arg = state::parse_launch_path(&std::env::args_os().collect::<Vec<_>>());
    let server = start_local(Settings::from_env()).unwrap_or_else(|err| {
        eprintln!(
            "AgentDrafter desktop could not start its local engine: {}",
            err.message
        );
        std::process::exit(1);
    });
    let base_url = server.base_url();
    let token = server.token().to_owned();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let recents_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("agentdrafter-desktop"));
            let recents_file = recents_dir.join("recents.json");

            let (initial_source, initial_title) = match launch_arg {
                Some(ref raw) => match state::inspect_path(raw) {
                    Ok(info) => (Some(info.path), Some(info.name)),
                    Err(_) => (Some(raw.display().to_string()), None),
                },
                None => (None, None),
            };
            let url_str = state::window_url(
                &base_url,
                &token,
                initial_source.as_deref(),
                initial_source.is_none(),
            );
            let url = Url::parse(&url_str)?;

            app.manage(DesktopState {
                base_url,
                token,
                server: Mutex::new(Some(server)),
                recents_file,
                recents_lock: Mutex::new(()),
                next_window_id: AtomicUsize::new(2),
            });

            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title(state::window_title(initial_title.as_deref()))
                .inner_size(1280.0, 820.0)
                .min_inner_size(390.0, 600.0)
                .build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pick_drawing_set_folder,
            pick_drawing_file,
            inspect_path,
            list_recents,
            record_recent,
            remove_recent,
            open_in_new_window,
            set_window_title,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build AgentDrafter desktop application");

    app.run(|app_handle, event| {
        if matches!(event, RunEvent::Exit) {
            app_handle.state::<DesktopState>().shutdown_server();
        }
    });
}
