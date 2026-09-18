mod core;

// This command is intentionally small: it proves the UI-to-core boundary
// without pretending that persistence or connector integrations exist yet.
#[tauri::command]
fn core_status() -> core::CoreStatus {
    core::status()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![core_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
