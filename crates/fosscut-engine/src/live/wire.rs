//! The wire format (ADR-0022, item 6): JSON-RPC 2.0, one UTF-8 JSON object per line.

use std::io::{self, Read, Write};

use fosscut_core::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC: the line is not JSON.
pub(crate) const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: the JSON is not a request.
pub(crate) const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: no such method.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: the parameters do not fit the method.
pub(crate) const INVALID_PARAMS: i64 = -32602;
/// A FossCut error; `data` holds the `ApiError`.
pub(crate) const FOSSCUT_ERROR: i64 = -32000;

/// A request from a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Call {
    /// Always `2.0`.
    pub jsonrpc: String,
    /// Chosen by the client; the reply repeats it.
    pub id: u64,
    /// `hello`, `auth`, `execute`, `query`, `snapshot` or `subscribe`.
    pub method: String,
    /// The method's parameters.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Call {
    pub(crate) fn new(id: u64, method: &str, params: Value) -> Call {
        Call {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// The host's answer to one call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Reply {
    /// Always `2.0`.
    pub jsonrpc: String,
    /// The call's id, or `null` when the call could not be read.
    pub id: Option<u64>,
    /// The result, when the call succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The error, when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

impl Reply {
    pub(crate) fn ok(id: u64, result: Value) -> Reply {
        Reply {
            jsonrpc: "2.0".into(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub(crate) fn failed(id: Option<u64>, code: i64, error: ApiError) -> Reply {
        Reply {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(WireError {
                code,
                message: error.message.clone(),
                data: Some(error),
            }),
        }
    }
}

/// A JSON-RPC error carrying the FossCut error, so clients return what a local session
/// would have.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WireError {
    /// The JSON-RPC error code.
    pub code: i64,
    /// For people.
    pub message: String,
    /// The FossCut error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ApiError>,
}

/// `hello`: the first message of a connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Hello {
    /// The client's protocol version.
    pub protocol: u32,
    /// The client's name for the host's list: `cli`, `app`, `agent:<client>`.
    pub client: String,
    /// The bundle the client means, which the host checks is its own.
    pub bundle: std::path::PathBuf,
    /// The client's nonce, in hex.
    pub nonce: String,
}

/// The host's answer to `hello`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Welcome {
    /// The host's protocol version.
    pub protocol: u32,
    /// The host program.
    pub host: super::HostName,
    /// The project's id.
    pub project: String,
    /// The project's revision.
    pub revision: u64,
    /// The host's nonce, in hex.
    pub nonce: String,
    /// The host's proof of the session key, in hex.
    pub proof: String,
}

/// A message from the host that answers no call: `changed` after a commit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Notification {
    /// Always `2.0`.
    pub jsonrpc: String,
    /// `changed`.
    pub method: String,
    /// What changed.
    pub params: Value,
}

/// Reads newline-terminated messages from a byte stream, refusing any longer than a cap.
#[derive(Default)]
pub(crate) struct LineReader {
    buffer: Vec<u8>,
}

impl LineReader {
    /// The next message without its newline, or `None` at a clean end of stream.
    pub(crate) fn read_line(
        &mut self,
        stream: &mut impl Read,
        cap: usize,
    ) -> io::Result<Option<Vec<u8>>> {
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
                if end > cap {
                    return Err(too_large(cap));
                }
                let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
                line.pop();
                return Ok(Some(line));
            }
            if self.buffer.len() > cap {
                return Err(too_large(cap));
            }
            let read = stream.read(&mut chunk)?;
            if read == 0 {
                return if self.buffer.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the stream ended inside a message",
                    ))
                };
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }
}

fn too_large(cap: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("a message is longer than {cap} bytes"),
    )
}

/// Writes one message and its newline. JSON escapes newlines inside strings, so the
/// newline only ever ends a message.
pub(crate) fn write_line(stream: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_at_newlines_and_respect_the_cap() {
        let mut input: &[u8] = b"{\"a\":1}\n{\"b\":2}\n12345678\n";
        let mut reader = LineReader::default();
        assert_eq!(
            reader.read_line(&mut input, 16).unwrap().unwrap(),
            b"{\"a\":1}"
        );
        assert_eq!(
            reader.read_line(&mut input, 16).unwrap().unwrap(),
            b"{\"b\":2}"
        );
        let error = reader.read_line(&mut input, 4).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let mut endless = std::io::repeat(b'x');
        let error = LineReader::default()
            .read_line(&mut endless, 1000)
            .unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::InvalidData,
            "a message with no end is refused at the cap"
        );

        let mut cut: &[u8] = b"{\"a\":";
        let error = LineReader::default().read_line(&mut cut, 64).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        let mut empty: &[u8] = b"";
        assert_eq!(
            LineReader::default().read_line(&mut empty, 64).unwrap(),
            None
        );
    }

    /// Every message kind, as clients and hosts see it. A change here is a protocol change:
    /// bump `PROTOCOL` (ADR-0022, item 6).
    #[test]
    fn the_wire_format_is_stable() {
        use fosscut_core::{EntryKind, Request, codes};

        use super::super::{Change, HostName};

        let hello = Hello {
            protocol: 1,
            client: "agent:claude-code".into(),
            bundle: "/work/Demo.fosscut".into(),
            nonce: "00".repeat(32),
        };
        let welcome = Welcome {
            protocol: 1,
            host: HostName {
                name: "FossCut".into(),
                version: "0.1.0".into(),
            },
            project: "project_0001".into(),
            revision: 4,
            nonce: "11".repeat(32),
            proof: "22".repeat(32),
        };
        let mut request = Request::single(
            "clip.split",
            serde_json::json!({ "clip": "clip_0003", "at": "105f" }),
        );
        request.expected_revision = Some(4);
        request.request_id = Some("split-1".into());
        request.actor = "agent:claude-code".into();
        let conflict = ApiError::new(
            codes::REVISION_CONFLICT,
            "the project is at revision 5, not 4",
        )
        .with_details(serde_json::json!({ "expected_revision": 4, "current_revision": 5 }));
        let change = Change {
            revision: 5,
            seq: 5,
            kind: EntryKind::Commit,
            actor: "human".into(),
            commands: vec!["clip.move".into()],
        };
        let messages = serde_json::json!([
            Call::new(1, "hello", serde_json::json!(hello)),
            Reply::ok(1, serde_json::json!(welcome)),
            Call::new(2, "auth", serde_json::json!({ "proof": "33".repeat(32) })),
            Reply::ok(2, serde_json::json!({})),
            Call::new(3, "execute", serde_json::json!(request)),
            Reply::failed(Some(3), FOSSCUT_ERROR, conflict),
            Call::new(
                4,
                "query",
                serde_json::json!({ "name": "timeline.read", "args": { "detail": "full" } })
            ),
            Call::new(5, "snapshot", Value::Null),
            Call::new(6, "subscribe", Value::Null),
            Notification {
                jsonrpc: "2.0".into(),
                method: "changed".into(),
                params: serde_json::json!(change),
            },
            Reply::failed(
                None,
                PARSE_ERROR,
                ApiError::new("fosscut.session.protocol", "the message is not JSON")
            ),
        ]);
        insta::assert_json_snapshot!("wire_format", messages);
    }

    #[test]
    fn messages_round_trip_through_lines() {
        let mut out = Vec::new();
        let call = Call::new(3, "query", serde_json::json!({ "name": "a\nb" }));
        write_line(&mut out, &call).unwrap();
        assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 1);
        let mut input = out.as_slice();
        let line = LineReader::default()
            .read_line(&mut input, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(serde_json::from_slice::<Call>(&line).unwrap(), call);
    }
}
