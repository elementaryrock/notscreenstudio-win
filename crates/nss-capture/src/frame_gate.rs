//! The countdown gate every screen frame passes through.
//!
//! Port of upstream `NSSCapture/FrameGate.swift`. Capture warms up before the countdown ends — the
//! session has to be running to know its size and to have a frame ready — but nothing may be *written*
//! until the countdown finishes. Frames that arrive early are held (only the newest, so the recorder
//! does not buffer a countdown's worth of 4K frames) and the held one is written at the moment the
//! gate opens, stamped with that instant. So t = 0 is the end of the countdown, not the moment capture
//! warmed up, and a stale frame can never be written ahead of it.

/// Holds the last frame seen while the gate is closed, then only lets time stamps through that
/// actually advance.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameGate<T> {
    is_armed: bool,
    held: Option<T>,
    minimum: f64,
    last: f64,
}

impl<T> FrameGate<T> {
    #[must_use]
    pub fn new(armed: bool) -> Self {
        Self {
            is_armed: armed,
            held: None,
            minimum: f64::NEG_INFINITY,
            last: f64::NEG_INFINITY,
        }
    }

    /// True once frames may be written.
    #[must_use]
    pub fn is_armed(&self) -> bool {
        self.is_armed
    }

    /// Keeps the newest frame seen while not armed.
    pub fn hold(&mut self, payload: T) {
        self.held = Some(payload);
    }

    /// True while a frame is being held.
    #[must_use]
    pub fn has_held(&self) -> bool {
        self.held.is_some()
    }

    /// Opens the gate at `now` (host seconds) and returns the held frame, if any, to be written at
    /// `now`.
    pub fn arm(&mut self, now: f64) -> Option<T> {
        self.is_armed = true;
        self.minimum = now;
        self.held.take()
    }

    /// True if a frame with this time stamp may be written next. Frames stamped before the gate opened
    /// (queued during the countdown) and frames that do not advance the clock are rejected.
    pub fn accept(&mut self, pts: f64) -> bool {
        if !self.is_armed || pts < self.minimum || pts <= self.last {
            return false;
        }
        self.last = pts;
        true
    }

    /// The last accepted time stamp, for diagnostics.
    #[must_use]
    pub fn last(&self) -> f64 {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_before_arm_are_never_written() {
        let mut g: FrameGate<u32> = FrameGate::new(false);
        assert!(!g.accept(1.0));
        g.hold(1);
        g.hold(2);
        assert!(g.has_held(), "only the newest is kept");
    }

    #[test]
    fn arm_returns_the_held_frame_and_makes_it_the_origin() {
        let mut g: FrameGate<u32> = FrameGate::new(false);
        g.hold(7);
        assert_eq!(g.arm(10.0), Some(7));
        assert!(!g.has_held());
        assert!(
            g.accept(10.0),
            "the held frame is written at the end of the countdown"
        );
    }

    #[test]
    fn stale_frames_queued_before_arm_are_dropped() {
        let mut g: FrameGate<u32> = FrameGate::new(false);
        let _ = g.arm(10.0);
        assert!(!g.accept(9.99));
        assert!(g.accept(10.016));
    }

    #[test]
    fn timestamps_strictly_increase() {
        let mut g: FrameGate<u32> = FrameGate::new(false);
        g.hold(1);
        let _ = g.arm(10.0);
        assert!(g.accept(10.0));
        assert!(!g.accept(10.0), "a duplicate time stamp is not a new frame");
        assert!(g.accept(10.02));
        assert!(
            !g.accept(10.01),
            "an older frame cannot overtake the newest one"
        );
        assert_eq!(g.last(), 10.02);
    }

    #[test]
    fn an_already_armed_gate_accepts_increasing_frames() {
        let mut g: FrameGate<u32> = FrameGate::new(true);
        assert!(g.accept(0.5));
        assert!(g.accept(0.6));
    }

    #[test]
    fn arm_without_a_held_frame_waits_for_live_frames() {
        let mut g: FrameGate<u32> = FrameGate::new(false);
        assert_eq!(g.arm(5.0), None);
        assert!(!g.accept(4.0));
        assert!(g.accept(5.01));
    }
}
