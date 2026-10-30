//! Mutual proof of the session key (ADR-0022, item 5). Each side sends a BLAKE3 keyed
//! hash over both nonces, so the key never crosses the wire and a process that took over
//! a dead host's endpoint can neither learn the key nor pass as the host.

use fosscut_core::{ApiError, codes};

use super::PROTOCOL;

/// A session key or a nonce: 32 bytes from the operating system's random source.
pub(crate) type Secret = [u8; 32];

/// Which side a proof comes from, so a proof cannot be reflected back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Host,
    Client,
}

/// 32 random bytes.
pub(crate) fn random() -> Result<Secret, ApiError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| {
        ApiError::new(
            codes::INTERNAL,
            format!("the operating system has no random source: {e}"),
        )
    })?;
    Ok(bytes)
}

/// The proof `side` sends: a keyed hash over a label, the protocol version, both nonces
/// and the bundle's record key.
pub(crate) fn proof(
    key: &Secret,
    side: Side,
    client_nonce: &Secret,
    host_nonce: &Secret,
    bundle_key: &str,
) -> blake3::Hash {
    let label: &[u8] = match side {
        Side::Host => b"fosscut live session: host proof\0",
        Side::Client => b"fosscut live session: client proof\0",
    };
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(label);
    hasher.update(&PROTOCOL.to_le_bytes());
    hasher.update(client_nonce);
    hasher.update(host_nonce);
    hasher.update(bundle_key.as_bytes());
    hasher.finalize()
}

/// Checks a proof received as hex. `blake3::Hash` compares in constant time.
pub(crate) fn verify(expected: &blake3::Hash, received: &str) -> bool {
    blake3::Hash::from_hex(received).is_ok_and(|hash| hash == *expected)
}

/// Lowercase hex.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 32 bytes from 64 hex digits.
pub(crate) fn from_hex(text: &str) -> Option<Secret> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (byte, pair) in bytes.iter_mut().zip(text.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofs_depend_on_every_input() {
        let key = random().unwrap();
        let (a, b) = (random().unwrap(), random().unwrap());
        assert_ne!(key, a, "two draws differ");
        let host = proof(&key, Side::Host, &a, &b, "k");
        assert!(verify(&host, &host.to_hex()));
        assert_ne!(host, proof(&key, Side::Client, &a, &b, "k"), "sides differ");
        assert_ne!(
            host,
            proof(&key, Side::Host, &b, &a, "k"),
            "nonce order counts"
        );
        assert_ne!(
            host,
            proof(&key, Side::Host, &a, &b, "other"),
            "the bundle counts"
        );
        assert_ne!(host, proof(&a, Side::Host, &a, &b, "k"), "the key counts");
        assert!(!verify(&host, "zz"));
        assert!(!verify(&host, &"0".repeat(64)));
    }

    #[test]
    fn hex_round_trips_and_rejects_bad_input() {
        let bytes = random().unwrap();
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("ab"), None);
        assert_eq!(from_hex(&"g".repeat(64)), None);
        assert_eq!(from_hex(&"é".repeat(32)), None);
    }
}
