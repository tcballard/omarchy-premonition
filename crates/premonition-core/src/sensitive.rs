//! Content-bearing wrappers whose debug output is always redacted.

use std::fmt;

use sha2::{Digest, Sha256};

/// Sensitive UTF-8 text with deliberately redacted `Debug` output.
#[derive(Clone, Eq, PartialEq)]
pub struct SensitiveText(String);

impl SensitiveText {
    /// Creates sensitive text without logging or formatting its body.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Exposes the body only to an explicit content-bearing operation.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Returns the UTF-8 byte length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true when the body is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns a SHA-256 digest suitable for equality/provenance checks.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

impl fmt::Debug for SensitiveText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SensitiveText")
            .field("bytes", &self.len())
            .field("sha256", &hex_prefix(&self.digest()))
            .finish_non_exhaustive()
    }
}

fn hex_prefix(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(16);
    for byte in &digest[..8] {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_contains_body() {
        let secret = SensitiveText::new("UNIQUE_SECRET_SENTINEL".into());
        let debug = format!("{secret:?}");
        assert!(!debug.contains("UNIQUE_SECRET_SENTINEL"));
        assert!(debug.contains("bytes"));
    }
}
