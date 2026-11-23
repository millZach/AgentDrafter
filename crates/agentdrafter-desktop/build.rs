const DESKTOP_COMMANDS: &[&str] = &[
    "pick_drawing_set_folder",
    "pick_drawing_file",
    "inspect_path",
    "list_recents",
    "record_recent",
    "remove_recent",
    "open_in_new_window",
    "set_window_title",
    "resolve_sibling_path",
];

fn main() {
    let attrs = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(DESKTOP_COMMANDS));
    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}
