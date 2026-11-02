//! The FossCut desktop app. Everything but this entry point is in the library.

#![forbid(unsafe_code)]
// No console window behind the app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `fosscut-desktop mcp [-p <project>]` serves FossCut's MCP server on stdio, without a
    // window, for the agents the app runs (ADR-0043).
    if let Some(code) = fosscut_desktop::agent::mcp_main(std::env::args_os().skip(1)) {
        std::process::exit(code);
    }
    fosscut_desktop::run();
}
