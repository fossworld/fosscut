//! Local endpoints (ADR-0022, item 4): a Unix domain socket in the runtime directory on
//! macOS and Linux, a named pipe on Windows. Nothing listens on the network.
//!
//! Each side of a connection reads and writes in turn, never both at once: Windows
//! serialises synchronous I/O on one pipe handle, so a reader blocked on one thread would
//! stall a writer on another.

use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use fosscut_core::ApiError;

use super::auth::{random, to_hex};
use super::{UNAVAILABLE, error};

/// The longest Unix socket path, leaving room for the terminating zero; macOS allows 104
/// bytes and Linux 108.
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 103;

/// A Windows pipe access list: full access for the system and the owner, nobody else.
#[cfg(windows)]
const OWNER_ONLY: &str = "D:P(A;;GA;;;SY)(A;;GA;;;OW)";

/// Accepts connections on a fresh, randomly named endpoint.
pub(crate) struct Listener {
    #[cfg(unix)]
    inner: std::os::unix::net::UnixListener,
    #[cfg(windows)]
    inner: interprocess::os::windows::named_pipe::PipeListener<
        interprocess::os::windows::named_pipe::pipe_mode::Bytes,
        interprocess::os::windows::named_pipe::pipe_mode::Bytes,
    >,
    endpoint: String,
}

impl Listener {
    /// Listens on a new endpoint: a socket in `runtime` on Unix, a pipe on Windows.
    pub(crate) fn bind(runtime: &Path) -> Result<Listener, ApiError> {
        let failed = |e: io::Error| error(UNAVAILABLE, format!("cannot listen for clients: {e}"));
        #[cfg(unix)]
        {
            let path = runtime.join(format!("{}.sock", to_hex(&random()?[..5])));
            let endpoint = path.to_str().map(str::to_string).ok_or_else(|| {
                error(
                    UNAVAILABLE,
                    "the runtime directory's path is not valid UTF-8",
                )
            })?;
            if endpoint.len() > MAX_SOCKET_PATH {
                return Err(error(
                    UNAVAILABLE,
                    format!("the socket path {endpoint} is longer than {MAX_SOCKET_PATH} bytes"),
                )
                .with_hint("Set FOSSCUT_RUNTIME_DIR to a shorter path.")
                .with_details(
                    serde_json::json!({ "runtime_dir": runtime, "reason": "socket_path_too_long" }),
                ));
            }
            let inner = std::os::unix::net::UnixListener::bind(&path).map_err(failed)?;
            Ok(Listener { inner, endpoint })
        }
        #[cfg(windows)]
        {
            use interprocess::os::windows::named_pipe::{PipeListenerOptions, PipeMode, pipe_mode};
            use interprocess::os::windows::security_descriptor::SecurityDescriptor;
            let _ = runtime;
            let endpoint = format!(r"\\.\pipe\fosscut-{}", to_hex(&random()?[..8]));
            let sddl = widestring::U16CString::from_str(OWNER_ONLY)
                .map_err(|e| error(UNAVAILABLE, format!("bad access list: {e}")))?;
            let descriptor = SecurityDescriptor::deserialize(&sddl).map_err(failed)?;
            let inner = PipeListenerOptions::new()
                .path(endpoint.as_str())
                .mode(PipeMode::Bytes)
                .accept_remote(false)
                .input_buffer_size_hint(65_536)
                .output_buffer_size_hint(65_536)
                .security_descriptor(Some(descriptor))
                .create_duplex::<pipe_mode::Bytes>()
                .map_err(failed)?;
            Ok(Listener { inner, endpoint })
        }
    }

    /// The socket path or pipe name clients connect to.
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Waits for the next client.
    pub(crate) fn accept(&self) -> io::Result<Stream> {
        #[cfg(unix)]
        {
            let (stream, _) = self.inner.accept()?;
            Ok(Stream {
                inner: Inner::Unix(stream),
            })
        }
        #[cfg(windows)]
        {
            Ok(Stream {
                inner: Inner::Server(self.inner.accept()?),
            })
        }
    }
}

/// One end of a connection.
pub(crate) struct Stream {
    inner: Inner,
}

enum Inner {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    #[cfg(windows)]
    Server(
        interprocess::os::windows::named_pipe::DuplexPipeStream<
            interprocess::os::windows::named_pipe::pipe_mode::Bytes,
        >,
    ),
    #[cfg(windows)]
    Client(std::fs::File),
}

/// Connects to a host's endpoint.
pub(crate) fn connect(endpoint: &str) -> io::Result<Stream> {
    #[cfg(unix)]
    {
        Ok(Stream {
            inner: Inner::Unix(std::os::unix::net::UnixStream::connect(endpoint)?),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // SECURITY_IDENTIFICATION: the server may learn who connects but cannot act as
        // this user, should another user's process own the pipe name.
        const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
        // ERROR_PIPE_BUSY: every instance is taken for a moment, between the host accepting
        // one client and offering the next instance.
        const ERROR_PIPE_BUSY: i32 = 231;
        let started = std::time::Instant::now();
        loop {
            let opened = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .security_qos_flags(SECURITY_IDENTIFICATION)
                .open(endpoint);
            match opened {
                Ok(file) => {
                    return Ok(Stream {
                        inner: Inner::Client(file),
                    });
                }
                Err(e)
                    if e.raw_os_error() == Some(ERROR_PIPE_BUSY)
                        && started.elapsed() < Duration::from_secs(2) =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Stream {
    /// Limits how long one read or write may block. Windows pipe handles have no timeout;
    /// there a read ends when the other side answers or the pipe breaks.
    pub(crate) fn set_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match &self.inner {
            #[cfg(unix)]
            Inner::Unix(stream) => {
                stream.set_read_timeout(timeout)?;
                stream.set_write_timeout(timeout)
            }
            #[cfg(windows)]
            _ => {
                let _ = timeout;
                Ok(())
            }
        }
    }

    /// True when the other side has closed the connection (or sent something unasked),
    /// looking without blocking. On Windows this is not known before the next read.
    pub(crate) fn hung_up(&self) -> bool {
        match &self.inner {
            #[cfg(unix)]
            Inner::Unix(stream) => {
                if stream.set_nonblocking(true).is_err() {
                    return true;
                }
                // A kept connection carries nothing unasked: a byte waiting on it breaks
                // the protocol as much as the end of the stream does.
                let mut byte = [0u8; 1];
                let mut reader: &std::os::unix::net::UnixStream = stream;
                let read = reader.read(&mut byte);
                let restored = stream.set_nonblocking(false).is_ok();
                match read {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => !restored,
                    _ => true,
                }
            }
            #[cfg(windows)]
            _ => false,
        }
    }

    /// A handle another thread can use to end the connection, waking a blocked read.
    pub(crate) fn closer(&self) -> Closer {
        match &self.inner {
            #[cfg(unix)]
            Inner::Unix(stream) => Closer(stream.try_clone().ok()),
            #[cfg(windows)]
            _ => Closer(None),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match &mut self.inner {
            #[cfg(unix)]
            Inner::Unix(stream) => stream.read(buf),
            #[cfg(windows)]
            Inner::Server(stream) => stream.read(buf),
            #[cfg(windows)]
            Inner::Client(file) => file.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.inner {
            #[cfg(unix)]
            Inner::Unix(stream) => stream.write(buf),
            #[cfg(windows)]
            Inner::Server(stream) => stream.write(buf),
            #[cfg(windows)]
            Inner::Client(file) => file.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.inner {
            #[cfg(unix)]
            Inner::Unix(stream) => stream.flush(),
            // Flushing a pipe waits until the other side has read everything; written
            // data is already in the pipe, so the host never waits on a slow reader.
            #[cfg(windows)]
            Inner::Server(_) => Ok(()),
            #[cfg(windows)]
            Inner::Client(file) => file.flush(),
        }
    }
}

/// Ends a connection from another thread (Unix); on Windows the connection ends when its
/// client next writes or disconnects.
pub(crate) struct Closer(Option<CloseHandle>);

#[cfg(unix)]
type CloseHandle = std::os::unix::net::UnixStream;
#[cfg(not(unix))]
type CloseHandle = ();

impl Closer {
    /// Shuts the connection down in both directions.
    pub(crate) fn close(&self) {
        #[cfg(unix)]
        if let Some(stream) = &self.0 {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        #[cfg(not(unix))]
        let _ = &self.0;
    }
}

#[cfg(all(test, windows))]
mod tests {
    use interprocess::os::windows::security_descriptor::{
        AsSecurityDescriptorExt, SecurityDescriptor,
    };

    use super::OWNER_ONLY;

    #[test]
    fn the_pipe_access_list_names_only_the_system_and_the_owner() {
        let sddl = widestring::U16CString::from_str(OWNER_ONLY).unwrap();
        let descriptor = SecurityDescriptor::deserialize(&sddl).unwrap();
        // DACL_SECURITY_INFORMATION
        let text = descriptor.serialize(0x4, |s| s.to_string_lossy()).unwrap();
        assert!(
            text.starts_with("D:P"),
            "inherited entries are blocked: {text}"
        );
        for anyone in ["WD", "AN", "NU", "BU", "AU", "IU"] {
            assert!(
                !text.contains(&format!(";;;{anyone})")),
                "{anyone} in {text}"
            );
        }
        assert!(text.contains(";;;SY)") && text.contains(";;;OW)"), "{text}");
    }
}
