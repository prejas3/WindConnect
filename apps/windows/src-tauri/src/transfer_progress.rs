//! CCS/1 PERF:monotonic one-second samples + EMA, no timer and no speed before a full window.
#[derive(Default)]
pub struct Rate {
    sample: Option<(u64, u64)>,
    previous: u64,
    value: Option<f64>,
}
impl Rate {
    pub fn update(&mut self, now_ms: u64, bytes: u64, running: bool) -> Option<u64> {
        if !running || bytes < self.previous {
            *self = Self::default();
        }
        self.previous = bytes;
        if !running {
            return None;
        }
        let (at, start) = *self.sample.get_or_insert((now_ms, bytes));
        let elapsed = now_ms.saturating_sub(at);
        if elapsed >= 1000 {
            let speed = bytes.saturating_sub(start) as f64 * 1000.0 / elapsed as f64;
            self.value = Some(self.value.map_or(speed, |old| old * 0.5 + speed * 0.5));
            self.sample = Some((now_ms, bytes));
        }
        self.value.map(|v| v as u64)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn steady_pause_and_retry_reset() {
        let mut r = Rate::default();
        assert_eq!(r.update(0, 0, true), None);
        assert_eq!(r.update(500, 500, true), None);
        assert_eq!(r.update(1000, 1000, true), Some(1000));
        assert_eq!(r.update(1100, 1100, true), Some(1000));
        assert_eq!(r.update(2000, 1100, false), None);
        assert_eq!(r.update(3000, 1100, true), None);
        assert_eq!(r.update(4000, 2100, true), Some(1000));
        assert_eq!(r.update(4100, 0, true), None);
    }
}
