// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = aidebook_lib::agents::bridge::run_if_requested() {
        std::process::exit(code);
    }
    aidebook_lib::run()
}
