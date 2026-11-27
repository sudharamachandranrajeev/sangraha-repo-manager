// Keeps a console window from opening next to Sangraha on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sangraha_lib::run()
}
