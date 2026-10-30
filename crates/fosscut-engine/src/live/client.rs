//! The client side of a live session (ADR-0022, items 5 and 8): connecting, proving the
//! session key both ways, and calls.

use std::collections::VecDeque;
use std::io;
use std::path::Path;

use fosscut_core::ApiError;
use serde_json::{Value, json};

use super::auth::{self, Side};
use super::record::{self, Record};
use super::transport::{self, Stream};
use super::wire::{Call, Hello, LineReader, Reply, Welcome, write_line};
use super::{
    Change, MAX_MESSAGE, PROTOCOL, PROTOCOL_MISMATCH, REQUEST_TIMEOUT, UNAUTHORIZED, UNAVAILABLE,
    error,
};

/// Why a connection could not be opened.
#[derive(Debug)]
pub(crate) enum OpenError {
    /// The endpoint did not answer or broke off; the host may be busy, starting or gone.
    Unreachable(String),
    /// The host answered and refused, or could not prove the key.
    Refused(ApiError),
}

/// Why a call failed.
#[derive(Debug)]
pub(crate) enum CallError {
    /// The request never reached the host; sending it elsewhere is safe.
    NotSent(io::Error),
    /// The request was sent and no answer came; it may or may not have run.
    Lost(io::Error),
    /// The host answered with an error.
    Api(ApiError),
}

/// An authenticated connection to a host.
pub(crate) struct Connection {
    stream: Stream,
    lines: LineReader,
    next: u64,
    welcome: Welcome,
    notifications: VecDeque<Value>,
}

impl Connection {
    /// Connects to the host a record names and runs the handshake: the host proves it
    /// knows the session key before the client proves it in turn.
    pub(crate) fn open(
        record: &Record,
        key_name: &str,
        bundle: &Path,
        client: &str,
    ) -> Result<Connection, OpenError> {
        if record.protocol != PROTOCOL {
            return Err(OpenError::Refused(protocol_mismatch(record.protocol)));
        }
        let secret = auth::from_hex(&record.key).ok_or_else(|| {
            OpenError::Refused(error(UNAUTHORIZED, "the session record's key is malformed"))
        })?;
        let stream = transport::connect(&record.endpoint)
            .map_err(|e| OpenError::Unreachable(format!("cannot connect to the host: {e}")))?;
        stream
            .set_timeout(Some(REQUEST_TIMEOUT))
            .map_err(|e| OpenError::Unreachable(e.to_string()))?;
        let client_nonce = auth::random().map_err(OpenError::Refused)?;
        let mut connection = Connection {
            stream,
            lines: LineReader::default(),
            next: 1,
            welcome: Welcome {
                protocol: PROTOCOL,
                host: record.host.clone(),
                project: record.project.clone(),
                revision: 0,
                nonce: String::new(),
                proof: String::new(),
            },
            notifications: VecDeque::new(),
        };
        // The host checks the bundle from its own working directory, so the path goes
        // over the wire absolute.
        let bundle = std::fs::canonicalize(bundle)
            .or_else(|_| std::path::absolute(bundle))
            .unwrap_or_else(|_| bundle.to_path_buf());
        let hello = Hello {
            protocol: PROTOCOL,
            client: client.to_string(),
            bundle,
            nonce: auth::to_hex(&client_nonce),
        };
        let welcome: Welcome = connection
            .call("hello", json!(hello))
            .map_err(handshake_error)
            .and_then(|value| {
                serde_json::from_value(value).map_err(|e| {
                    OpenError::Refused(error(PROTOCOL_MISMATCH, format!("welcome: {e}")))
                })
            })?;
        if welcome.protocol != PROTOCOL {
            return Err(OpenError::Refused(protocol_mismatch(welcome.protocol)));
        }
        let host_nonce = auth::from_hex(&welcome.nonce).ok_or_else(|| {
            OpenError::Refused(error(PROTOCOL_MISMATCH, "welcome: the nonce is malformed"))
        })?;
        let expected = auth::proof(&secret, Side::Host, &client_nonce, &host_nonce, key_name);
        if !auth::verify(&expected, &welcome.proof) {
            return Err(OpenError::Refused(error(
                UNAUTHORIZED,
                "the host could not prove it knows the session key",
            )));
        }
        let proof = auth::proof(&secret, Side::Client, &client_nonce, &host_nonce, key_name);
        connection
            .call("auth", json!({ "proof": proof.to_hex().to_string() }))
            .map_err(handshake_error)?;
        connection.welcome = welcome;
        Ok(connection)
    }

    /// The host's answer to `hello`: its name, the project and its revision then.
    pub(crate) fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// True when the host has closed the connection: a kept connection to a host that
    /// closed the project is not reused.
    pub(crate) fn hung_up(&self) -> bool {
        self.notifications.is_empty() && self.stream.hung_up()
    }

    /// Sends one call and waits for its answer, keeping notifications that arrive first.
    pub(crate) fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.next;
        self.next += 1;
        write_line(&mut self.stream, &Call::new(id, method, params)).map_err(CallError::NotSent)?;
        loop {
            let value = self.next_message().map_err(CallError::Lost)?;
            if value.get("method").is_some() {
                self.notifications.push_back(value);
                continue;
            }
            let reply: Reply =
                serde_json::from_value(value).map_err(|e| CallError::Lost(io::Error::other(e)))?;
            if let Some(failure) = reply.error {
                return Err(CallError::Api(
                    failure
                        .data
                        .unwrap_or_else(|| error(PROTOCOL_MISMATCH, failure.message)),
                ));
            }
            if reply.id != Some(id) {
                return Err(CallError::Lost(io::Error::other(
                    "the host answered another request",
                )));
            }
            return Ok(reply.result.unwrap_or(Value::Null));
        }
    }

    fn next_message(&mut self) -> io::Result<Value> {
        if let Some(value) = self.notifications.pop_front() {
            return Ok(value);
        }
        let line = self
            .lines
            .read_line(&mut self.stream, MAX_MESSAGE)?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the host closed the connection",
                )
            })?;
        serde_json::from_slice(&line).map_err(io::Error::other)
    }
}

fn handshake_error(error: CallError) -> OpenError {
    match error {
        CallError::Api(api) => OpenError::Refused(api),
        CallError::NotSent(e) | CallError::Lost(e) => {
            OpenError::Unreachable(format!("the host broke off the handshake: {e}"))
        }
    }
}

fn protocol_mismatch(host: u32) -> ApiError {
    error(
        PROTOCOL_MISMATCH,
        format!("the host speaks protocol {host}, this FossCut {PROTOCOL}"),
    )
    .with_details(json!({ "host_protocol": host, "client_protocol": PROTOCOL }))
}

/// A stream of a host's commits, from [`watch`].
pub struct Subscription {
    connection: Connection,
}

impl Subscription {
    /// The next commit, waiting for it. `None` when the host closed the project.
    pub fn next_change(&mut self) -> Result<Option<Change>, ApiError> {
        match self.connection.next_message() {
            Ok(value) => {
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                serde_json::from_value(params)
                    .map(Some)
                    .map_err(|e| error(PROTOCOL_MISMATCH, format!("changed: {e}")))
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(error(
                UNAVAILABLE,
                format!("the host connection broke: {e}"),
            )),
        }
    }
}

/// Follows the commits of the host serving `bundle`, as `client`; records are looked up
/// in `runtime`. Fails with `fosscut.session.unavailable` when no host serves it.
pub fn watch(bundle: &Path, runtime: &Path, client: &str) -> Result<Subscription, ApiError> {
    let key_name = record::bundle_key(bundle)?;
    let found = record::read(runtime, &key_name)?
        .ok_or_else(|| error(UNAVAILABLE, format!("no host serves {}", bundle.display())))?;
    let mut connection =
        Connection::open(&found, &key_name, bundle, client).map_err(|e| match e {
            OpenError::Refused(api) => api,
            OpenError::Unreachable(message) => error(UNAVAILABLE, message),
        })?;
    match connection.call("subscribe", Value::Null) {
        Ok(_) => {}
        Err(CallError::Api(api)) => return Err(api),
        Err(CallError::NotSent(e) | CallError::Lost(e)) => {
            return Err(error(
                UNAVAILABLE,
                format!("the host connection broke: {e}"),
            ));
        }
    }
    connection
        .stream
        .set_timeout(None)
        .map_err(|e| error(UNAVAILABLE, e.to_string()))?;
    Ok(Subscription { connection })
}
