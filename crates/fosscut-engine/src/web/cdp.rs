//! A small Chrome DevTools Protocol client over a WebSocket on the loopback interface
//! (ADR-0039): enough of RFC 6455 for one client connection (masked text frames out,
//! fragmented frames in, ping and close), and request and response matching by id with
//! the events in between kept for whoever waits for them.
//!
//! Requests the page makes are answered here too, from the composition's folder, through
//! the `Fetch` domain: the page lives at a made-up origin, so relative addresses, modules
//! and `fetch` work, and nothing outside the folder or on the network can be read.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The origin a composition's page is served from. `.invalid` never resolves (RFC 2606),
/// so nothing could reach the network even if interception failed.
pub(crate) const ORIGIN: &str = "https://composition.fosscut.invalid";

/// Longest wait for one answer from the browser.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// What went wrong talking to the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CdpError(pub(crate) String);

impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn error(message: impl Into<String>) -> CdpError {
    CdpError(message.into())
}

/// One WebSocket connection, as a client.
struct Socket {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Socket {
    /// Connects to `url` (`ws://127.0.0.1:PORT/PATH`) and completes the opening handshake.
    fn connect(url: &str) -> Result<Socket, CdpError> {
        let rest = url
            .strip_prefix("ws://")
            .ok_or_else(|| error(format!("not a ws:// address: {url}")))?;
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let stream = TcpStream::connect(host)
            .map_err(|e| error(format!("cannot connect to the browser at {host}: {e}")))?;
        stream
            .set_nodelay(true)
            .map_err(|e| error(format!("cannot configure the connection: {e}")))?;
        let writer = stream
            .try_clone()
            .map_err(|e| error(format!("cannot configure the connection: {e}")))?;
        let mut socket = Socket {
            reader: BufReader::new(stream),
            writer,
        };
        let mut key = [0u8; 16];
        getrandom::fill(&mut key).map_err(|e| error(format!("no randomness: {e}")))?;
        let request = format!(
            "GET /{path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            encode_base64(&key)
        );
        socket
            .writer
            .write_all(request.as_bytes())
            .map_err(|e| error(format!("cannot send the handshake: {e}")))?;
        socket.set_timeout(Some(ANSWER_TIMEOUT))?;
        let mut status = String::new();
        socket
            .reader
            .read_line(&mut status)
            .map_err(|e| error(format!("no handshake answer: {e}")))?;
        if !status.starts_with("HTTP/1.1 101") {
            return Err(error(format!(
                "the browser refused the connection: {}",
                status.trim()
            )));
        }
        loop {
            let mut line = String::new();
            let read = socket
                .reader
                .read_line(&mut line)
                .map_err(|e| error(format!("broken handshake: {e}")))?;
            if read == 0 || line == "\r\n" || line == "\n" {
                break;
            }
        }
        Ok(socket)
    }

    fn set_timeout(&mut self, timeout: Option<Duration>) -> Result<(), CdpError> {
        self.reader
            .get_ref()
            .set_read_timeout(timeout)
            .map_err(|e| error(format!("cannot set a timeout: {e}")))
    }

    /// Sends `text` as one masked text frame.
    fn send(&mut self, text: &str) -> Result<(), CdpError> {
        self.send_frame(0x1, text.as_bytes())
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), CdpError> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode);
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len <= usize::from(u16::MAX) {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mut mask = [0u8; 4];
        getrandom::fill(&mut mask).map_err(|e| error(format!("no randomness: {e}")))?;
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.writer
            .write_all(&frame)
            .map_err(|e| error(format!("the browser connection broke: {e}")))
    }

    /// The next complete text message, answering pings on the way.
    fn receive(&mut self) -> Result<String, CdpError> {
        let mut message = Vec::new();
        loop {
            let mut head = [0u8; 2];
            self.read_exact(&mut head)?;
            let fin = head[0] & 0x80 != 0;
            let opcode = head[0] & 0x0F;
            let masked = head[1] & 0x80 != 0;
            let len = match head[1] & 0x7F {
                126 => {
                    let mut bytes = [0u8; 2];
                    self.read_exact(&mut bytes)?;
                    u64::from(u16::from_be_bytes(bytes))
                }
                127 => {
                    let mut bytes = [0u8; 8];
                    self.read_exact(&mut bytes)?;
                    u64::from_be_bytes(bytes)
                }
                short => u64::from(short),
            };
            let mut mask = [0u8; 4];
            if masked {
                self.read_exact(&mut mask)?;
            }
            let len = usize::try_from(len).map_err(|_| error("a message is too large"))?;
            let mut payload = vec![0u8; len];
            self.read_exact(&mut payload)?;
            if masked {
                for (i, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mask[i % 4];
                }
            }
            match opcode {
                0x0..=0x2 => {
                    message.extend_from_slice(&payload);
                    if fin {
                        return String::from_utf8(message)
                            .map_err(|_| error("the browser sent text that is not UTF-8"));
                    }
                }
                0x8 => return Err(error("the browser closed the connection")),
                0x9 => self.send_frame(0xA, &payload)?,
                _ => {}
            }
        }
    }

    fn read_exact(&mut self, buffer: &mut [u8]) -> Result<(), CdpError> {
        self.reader.read_exact(buffer).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                error("the browser did not answer in time")
            }
            _ => error(format!("the browser connection broke: {e}")),
        })
    }
}

/// A connection to a browser, with at most one page session in use.
pub(crate) struct Cdp {
    socket: Socket,
    next_id: u64,
    events: VecDeque<Value>,
    /// The composition folder requests are answered from.
    root: Option<PathBuf>,
}

impl Cdp {
    /// Connects to the browser's DevTools endpoint `url`.
    pub(crate) fn connect(url: &str) -> Result<Cdp, CdpError> {
        Ok(Cdp {
            socket: Socket::connect(url)?,
            next_id: 0,
            events: VecDeque::new(),
            root: None,
        })
    }

    /// Answers the page's requests from `root` from now on (with `Fetch.enable` sent for the
    /// session by the caller).
    pub(crate) fn serve(&mut self, root: PathBuf) {
        self.root = Some(root);
    }

    /// Sends `method` with `params` to `session` (the browser itself when `None`) and waits
    /// for its result, answering the page's requests and keeping other events meanwhile.
    pub(crate) fn call(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, CdpError> {
        let id = self.send(session, method, params)?;
        let deadline = Instant::now() + ANSWER_TIMEOUT;
        loop {
            let message = self.next_message(deadline)?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(problem) = message.get("error") {
                    let text = problem
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error");
                    return Err(error(format!("{method}: {text}")));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            self.keep(message)?;
        }
    }

    /// Waits up to `timeout` for the event `method` from `session`, answering the page's
    /// requests meanwhile; events seen before are looked at first.
    pub(crate) fn wait_event(
        &mut self,
        session: &str,
        method: &str,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        let matches = |event: &Value| {
            event.get("method").and_then(Value::as_str) == Some(method)
                && event.get("sessionId").and_then(Value::as_str) == Some(session)
        };
        if let Some(index) = self.events.iter().position(matches) {
            return Ok(self.events.remove(index).unwrap_or(Value::Null));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let message = self.next_message(deadline)?;
            if matches(&message) {
                return Ok(message);
            }
            self.keep(message)?;
        }
    }

    /// Forgets the events seen so far.
    pub(crate) fn clear_events(&mut self) {
        self.events.clear();
    }

    fn send(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<u64, CdpError> {
        self.next_id += 1;
        let mut message = json!({ "id": self.next_id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.socket.send(&message.to_string())?;
        Ok(self.next_id)
    }

    fn next_message(&mut self, deadline: Instant) -> Result<Value, CdpError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(error("the browser did not answer in time"));
        }
        self.socket.set_timeout(Some(left))?;
        let text = self.socket.receive()?;
        serde_json::from_str(&text).map_err(|e| error(format!("unreadable message: {e}")))
    }

    /// Answers a paused request, or keeps an event for later. Answers to requests nobody
    /// waits for (those this client sends on its own) are dropped.
    fn keep(&mut self, message: Value) -> Result<(), CdpError> {
        match message.get("method").and_then(Value::as_str) {
            Some("Fetch.requestPaused") => self.answer(&message),
            Some(_) => {
                // Keep the latest events only: a page that logs a lot must not grow this.
                if self.events.len() >= 256 {
                    self.events.pop_front();
                }
                self.events.push_back(message);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Fulfils a request the page made from the composition folder, or fails it.
    fn answer(&mut self, event: &Value) -> Result<(), CdpError> {
        let session = event
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let params = &event["params"];
        let request = params["requestId"].as_str().unwrap_or_default().to_string();
        let url = params["request"]["url"].as_str().unwrap_or_default();
        let file = self.root.as_deref().and_then(|root| local_file(root, url));
        let reply = match file.and_then(|path| std::fs::read(&path).ok().map(|b| (path, b))) {
            Some((path, body)) => (
                "Fetch.fulfillRequest",
                json!({
                    "requestId": request,
                    "responseCode": 200,
                    "responseHeaders": [
                        { "name": "Content-Type", "value": content_type(&path) },
                        { "name": "Access-Control-Allow-Origin", "value": "*" },
                        { "name": "Cache-Control", "value": "no-store" },
                    ],
                    "body": encode_base64(&body),
                }),
            ),
            None => (
                "Fetch.failRequest",
                json!({ "requestId": request, "errorReason": "BlockedByClient" }),
            ),
        };
        self.send(session.as_deref(), reply.0, reply.1).map(|_| ())
    }
}

/// The file in `root` that `url` names on the composition's origin; `None` for any other
/// address or a path that leaves the folder.
pub(crate) fn local_file(root: &Path, url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix(ORIGIN)?;
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode(path)?;
    let mut file = root.to_path_buf();
    for part in Path::new(decoded.trim_start_matches('/')).components() {
        match part {
            Component::Normal(name) => file.push(name),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if file.is_dir() {
        file.push("index.html");
    }
    file.is_file().then_some(file)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().filter(|s| !s.contains('\0'))
}

/// The media type a file is served with, by its extension.
pub(crate) fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Decodes standard base64, ignoring padding and whitespace.
pub(crate) fn decode_base64(text: &str) -> Result<Vec<u8>, CdpError> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return Err(error("the browser sent invalid base64")),
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for text in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let encoded = encode_base64(text.as_bytes());
            assert_eq!(decode_base64(&encoded).unwrap(), text.as_bytes());
        }
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert!(decode_base64("Zm9v!").is_err());
    }

    #[test]
    fn requests_stay_inside_the_composition() {
        let root = std::env::temp_dir().join(format!("fosscut-cdp-{}", std::process::id()));
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("index.html"), "<p>hi</p>").unwrap();
        std::fs::write(root.join("lib/a b.js"), "1").unwrap();
        let page = |path: &str| format!("{ORIGIN}{path}");
        assert_eq!(
            local_file(&root, &page("/index.html?x=1#top")),
            Some(root.join("index.html"))
        );
        assert_eq!(local_file(&root, &page("/")), Some(root.join("index.html")));
        assert_eq!(
            local_file(&root, &page("/lib/a%20b.js")),
            Some(root.join("lib").join("a b.js"))
        );
        assert_eq!(local_file(&root, &page("/../secret")), None);
        assert_eq!(local_file(&root, &page("/lib/%2e%2e/%2e%2e/x")), None);
        assert_eq!(local_file(&root, &page("/missing.js")), None);
        assert_eq!(local_file(&root, "https://example.com/index.html"), None);
        assert_eq!(
            local_file(&root, &format!("{ORIGIN}.evil/index.html")),
            None
        );
        assert_eq!(
            content_type(Path::new("a.MJS")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("a.woff2")), "font/woff2");
        let _ = std::fs::remove_dir_all(&root);
    }
}
