//! CCS/1 M:windctl | PURPOSE:diagnostic parsing and nearest-rank RTT statistics
use std::time::Duration;
use wind_core::identity::PublicKey;

pub fn decode_hex(text: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    if text.len() > max_bytes.saturating_mul(2)
        || !text.len().is_multiple_of(2)
        || !text.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(format!(
            "expected even-length hexadecimal, at most {max_bytes} bytes"
        ));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn parse_key(text: &str) -> Result<PublicKey, String> {
    let bytes: [u8; 32] = decode_hex(text, 32)?
        .try_into()
        .map_err(|_| "Ed25519 public key must be exactly 64 hex characters")?;
    PublicKey::from_bytes(&bytes).ok_or_else(|| "invalid Ed25519 public key".into())
}

#[derive(Debug, PartialEq, Eq)]
pub struct LatencyStats {
    pub samples: usize,
    pub min: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub max: Duration,
}

impl LatencyStats {
    /// CCS/1 RULE:nearest rank = ceil(percentile * N / 100), 1-based; successes only.
    pub fn from_samples(mut samples: Vec<Duration>) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        samples.sort_unstable();
        let n = samples.len();
        Some(Self {
            samples: n,
            min: samples[0],
            p50: samples[n.div_ceil(2) - 1],
            p95: samples[(n * 95).div_ceil(100) - 1],
            max: samples[n - 1],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank_including_small_samples() {
        assert!(LatencyStats::from_samples(vec![]).is_none());
        let stats = LatencyStats::from_samples(vec![Duration::from_micros(7)]).unwrap();
        assert_eq!(stats.min, stats.max);
        assert_eq!(stats.p50, stats.p95);
        let stats =
            LatencyStats::from_samples((1..=100).rev().map(Duration::from_micros).collect())
                .unwrap();
        assert_eq!(stats.p50.as_micros(), 50);
        assert_eq!(stats.p95.as_micros(), 95);
        assert_eq!(stats.samples, 100);
        let stats =
            LatencyStats::from_samples((1..=3).map(Duration::from_micros).collect()).unwrap();
        assert_eq!(stats.p50.as_micros(), 2);
        assert_eq!(stats.p95.as_micros(), 3);
    }

    #[test]
    fn hex_rejects_invalid_and_oversized_input_without_slicing_unicode() {
        for input in ["a", "zz", "é", "abcd"] {
            assert!(decode_hex(input, 1).is_err());
        }
        assert_eq!(decode_hex("aB00", 2).unwrap(), [0xab, 0]);
        assert!(parse_key("").is_err());
        assert!(parse_key(&"ff".repeat(33)).is_err());
    }
}
