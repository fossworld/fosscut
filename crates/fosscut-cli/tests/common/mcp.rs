//! An MCP client for end-to-end tests: it starts the built `fosscut mcp` as a child process
//! and calls its tools over stdio, as an agent does.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
};
use rmcp::service::RunningService;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::Value;

/// A client that introduces itself with `name`, which the project history records as
/// `agent:<name>`.
pub struct NamedClient(pub &'static str);

impl ClientHandler for NamedClient {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new(self.0, "1.0"),
        )
    }
}

/// A connected client.
pub type Client = RunningService<RoleClient, NamedClient>;

/// Starts `fosscut mcp` in `dir` and connects to it as `name`.
pub async fn start(dir: &Path, name: &'static str) -> Client {
    let command = tokio::process::Command::new(env!("CARGO_BIN_EXE_fosscut")).configure(|cmd| {
        cmd.arg("mcp")
            .current_dir(dir)
            .env_remove("FOSSCUT_PROJECT")
            .env("FOSSCUT_CACHE_DIR", dir.join("cache"));
    });
    NamedClient(name)
        .serve(TokioChildProcess::new(command).unwrap())
        .await
        .unwrap()
}

/// Calls `tool` with `args`.
pub async fn call(client: &Client, tool: &str, args: Value) -> CallToolResult {
    let arguments = args.as_object().cloned().unwrap_or_default();
    client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments))
        .await
        .unwrap()
}

/// The JSON a tool answered with: its last text block.
pub fn json_of(result: &CallToolResult) -> Value {
    let text = result
        .content
        .iter()
        .rev()
        .find_map(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The width and height of the PNG image a tool returned first.
pub fn png_size(result: &CallToolResult) -> (u32, u32) {
    use base64::Engine;
    let image = result.content[0].as_image().unwrap();
    assert_eq!(image.mime_type, "image/png");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let number = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (number(16), number(20))
}
