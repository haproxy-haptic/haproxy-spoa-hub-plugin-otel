//! Opaque 128-bit correlation handle (task T010).
//!
//! Generated on the request-side SPOE message and round-tripped through
//! HAProxy as a transaction-scoped variable; on the response-side it keys
//! the in-flight span lookup. 128 bits give a negligible collision rate
//! within a hub instance's request window.

use rand::RngCore;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CorrelationHandle([u8; 16]);

impl CorrelationHandle {
    /// Generate a fresh handle from a cryptographically-strong RNG.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Render as a 32-character lowercase hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parse a 32-character hex string back into a handle.
    pub fn from_hex(s: &str) -> Result<Self, ParseError> {
        if s.len() != 32 {
            return Err(ParseError::WrongLength(s.len()));
        }
        let mut out = [0u8; 16];
        hex::decode_to_slice(s, &mut out).map_err(|_| ParseError::InvalidHex)?;
        Ok(Self(out))
    }
}

impl fmt::Display for CorrelationHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for CorrelationHandle {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("expected 32 hex characters, got {0}")]
    WrongLength(usize),
    #[error("invalid hex string")]
    InvalidHex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let h = CorrelationHandle::generate();
        let s = h.to_hex();
        assert_eq!(s.len(), 32);
        assert_eq!(CorrelationHandle::from_hex(&s).unwrap(), h);
    }

    #[test]
    fn handles_are_unique_in_a_small_sample() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1024 {
            assert!(seen.insert(CorrelationHandle::generate()));
        }
    }

    #[test]
    fn rejects_short_hex() {
        assert!(matches!(
            CorrelationHandle::from_hex("0123"),
            Err(ParseError::WrongLength(4))
        ));
    }

    #[test]
    fn rejects_invalid_hex() {
        assert!(matches!(
            CorrelationHandle::from_hex("zz123456789012345678901234567890"),
            Err(ParseError::InvalidHex)
        ));
    }
}
