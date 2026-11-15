//! CCS/1 M:input interpolation | RULE:one frame maximum; barriers flush preceding totals; no extrapolation.
use super::InputEvent;

#[derive(Default)]
pub struct Interpolator {
    pointer: (i64, i64),
    scroll: (i64, i64),
}
impl Interpolator {
    pub fn pending(&self) -> bool {
        self.pointer != (0, 0) || self.scroll != (0, 0)
    }
    pub fn push(&mut self, event: InputEvent) -> Vec<InputEvent> {
        match event {
            InputEvent::Pointer { dx, dy, .. } => {
                self.pointer.0 = self.pointer.0.saturating_add(dx);
                self.pointer.1 = self.pointer.1.saturating_add(dy);
                Vec::new()
            }
            InputEvent::Scroll { dx, dy, .. } => {
                self.scroll.0 = self.scroll.0.saturating_add(dx);
                self.scroll.1 = self.scroll.1.saturating_add(dy);
                Vec::new()
            }
            InputEvent::Begin(_) | InputEvent::End(_) | InputEvent::Withdrawn => {
                self.clear();
                vec![event]
            }
            _ => {
                let mut events = self.flush();
                events.push(event);
                events
            }
        }
    }
    pub fn clear(&mut self) {
        self.pointer = (0, 0);
        self.scroll = (0, 0);
    }
    pub fn flush(&mut self) -> Vec<InputEvent> {
        let mut events = Vec::with_capacity(2);
        if self.pointer != (0, 0) {
            events.push(InputEvent::Pointer {
                dx: self.pointer.0,
                dy: self.pointer.1,
                t: None,
            });
        }
        if self.scroll != (0, 0) {
            events.push(InputEvent::Scroll {
                dx: self.scroll.0,
                dy: self.scroll.1,
                t: None,
            });
        }
        self.clear();
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Button, Gesture};
    #[test]
    fn totals_barriers_and_cancel_at_120hz() {
        let mut smooth = Interpolator::default();
        let mut sum = (0, 0);
        for frame in 0..120 {
            for _ in 0..4 {
                assert!(smooth
                    .push(InputEvent::Pointer {
                        dx: 17,
                        dy: -9,
                        t: None
                    })
                    .is_empty());
            }
            let barrier = if frame % 2 == 0 {
                InputEvent::Gesture(Gesture::TaskView)
            } else {
                InputEvent::Button {
                    button: Button::Left,
                    down: false,
                }
            };
            let events = smooth.push(barrier.clone());
            assert_eq!(events.last(), Some(&barrier));
            for e in events {
                if let InputEvent::Pointer { dx, dy, .. } = e {
                    sum.0 += dx;
                    sum.1 += dy;
                }
            }
            assert!(!smooth.pending());
        }
        assert_eq!(sum, (120 * 4 * 17, -120 * 4 * 9));
        smooth.push(InputEvent::Scroll {
            dx: 50,
            dy: 90,
            t: None,
        });
        assert_eq!(smooth.push(InputEvent::End(0)), [InputEvent::End(0)]);
        assert!(smooth.flush().is_empty());
    }
}
