//! The FossCut desktop app. Everything but this entry point is in the library.

#![forbid(unsafe_code)]
// No console window behind the app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    fosscut_desktop::run();
}
