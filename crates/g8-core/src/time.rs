//! Time utilities: `EpochMillis` newtype wrapping i64 epoch milliseconds.

use serde::{Deserialize, Serialize};

/// Epoch milliseconds since Unix epoch. All timestamps in g8 use this type.
///
/// # Examples
///
/// ```
/// use g8_core::EpochMillis;
///
/// let t = EpochMillis::from_i64(1_700_000_000_000);
/// assert_eq!(t.as_i64(), 1_700_000_000_000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochMillis(i64);

impl EpochMillis {
    /// Construct from a raw i64 epoch-millisecond value.
    pub fn from_i64(ms: i64) -> Self {
        Self(ms)
    }

    /// Return the underlying i64.
    pub fn as_i64(self) -> i64 {
        self.0
    }

    /// Return the value as a u64 (saturating at 0 for negative values).
    pub fn as_u64(self) -> u64 {
        self.0.max(0) as u64
    }
}

impl std::fmt::Display for EpochMillis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Returns the current wall-clock time as [`EpochMillis`].
pub fn now_millis() -> EpochMillis {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    EpochMillis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_serde() {
        let t = EpochMillis::from_i64(1_700_000_000_000);
        let json = serde_json::to_string(&t).unwrap();
        let back: EpochMillis = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn now_millis_is_reasonable() {
        let ms = now_millis();
        // Any time after 2020-01-01
        assert!(ms.as_i64() > 1_577_836_800_000);
    }

    #[test]
    fn as_u64_saturates() {
        let neg = EpochMillis::from_i64(-5);
        assert_eq!(neg.as_u64(), 0);
    }
}
