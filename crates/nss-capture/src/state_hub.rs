//! Fan-out of the recording state and of capture failures.
//!
//! Port of upstream `NSSCapture/StateHub.swift`. Upstream handed out `AsyncStream`s; the core traits
//! are synchronous by ADR 0009, so subscribers here are plain callbacks and the hub keeps the last
//! value so a new subscriber does not have to race the first transition.
//!
//! Two channels, because they mean different things to the shell: the *state* drives the tray icon
//! and the HUD, and a *capture event* is something that went wrong and needs a sentence in the UI.

use std::sync::{Arc, Mutex};

use nss_core::{NssError, RecordingState};

/// Which part of a session reported the problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Screen,
    Microphone,
    Camera,
    Writer,
}

/// Something that went wrong while recording was running.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureEvent {
    pub component: Component,
    pub error: NssError,
    /// True if the screen recording itself ended. Mic and camera losses are non-fatal: their files are
    /// finalised and the recording continues.
    pub is_fatal: bool,
}

impl CaptureEvent {
    #[must_use]
    pub fn new(component: Component, error: NssError, is_fatal: bool) -> Self {
        Self {
            component,
            error,
            is_fatal,
        }
    }

    /// A fatal screen-capture failure.
    #[must_use]
    pub fn screen(error: NssError) -> Self {
        Self::new(Component::Screen, error, true)
    }

    /// A microphone or camera failure that does not end the recording.
    #[must_use]
    pub fn device(component: Component, error: NssError) -> Self {
        Self::new(component, error, false)
    }
}

/// Callback registered with [`StateHub::subscribe`] or [`EventHub::subscribe`].
pub type Listener<T> = Box<dyn Fn(&T) + Send + 'static>;

/// Lock-protected fan-out of `RecordingState`. Subscribers register synchronously and immediately get
/// the current value.
#[derive(Default)]
pub struct StateHub {
    inner: Mutex<HubState>,
}

type SharedListener<T> = Arc<Mutex<Listener<T>>>;

#[derive(Default)]
struct HubState {
    state: RecordingState,
    next_id: u64,
    listeners: Vec<(u64, SharedListener<RecordingState>)>,
}

impl StateHub {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn current(&self) -> RecordingState {
        self.lock().state.clone()
    }

    /// Registers a listener and calls it once with the current state. Keep the returned id to
    /// unsubscribe.
    pub fn subscribe(&self, listener: Listener<RecordingState>) -> u64 {
        let shared: SharedListener<RecordingState> = Arc::new(Mutex::new(listener));
        let (id, current) = {
            let mut guard = self.lock();
            let id = guard.next_id;
            guard.next_id += 1;
            let current = guard.state.clone();
            guard.listeners.push((id, Arc::clone(&shared)));
            (id, current)
        };
        // Run outside the hub lock so the listener may query `current()` or unsubscribe safely.
        let cb = shared.lock().unwrap_or_else(|e| e.into_inner());
        cb(&current);
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.lock().listeners.retain(|(i, _)| *i != id);
    }

    pub fn set(&self, state: RecordingState) {
        // Listeners run with the hub lock released: a listener that calls back into the hub (the
        // shell updates its tray icon, which reads the state) must not deadlock, and any subscribe
        // or unsubscribe triggered inside a callback is preserved.
        let snapshot: Vec<SharedListener<RecordingState>> = {
            let mut guard = self.lock();
            guard.state = state.clone();
            guard.listeners.iter().map(|(_, l)| Arc::clone(l)).collect()
        };
        for listener in snapshot {
            let cb = listener.lock().unwrap_or_else(|e| e.into_inner());
            cb(&state);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HubState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Default)]
struct EventState {
    next_id: u64,
    listeners: Vec<(u64, SharedListener<CaptureEvent>)>,
}

/// Fan-out of [`CaptureEvent`]s.
#[derive(Default)]
pub struct EventHub {
    inner: Mutex<EventState>,
}

impl EventHub {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.lock().listeners.len()
    }

    pub fn subscribe(&self, listener: Listener<CaptureEvent>) -> u64 {
        let mut guard = self.lock();
        let id = guard.next_id;
        guard.next_id += 1;
        guard.listeners.push((id, Arc::new(Mutex::new(listener))));
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.lock().listeners.retain(|(i, _)| *i != id);
    }

    pub fn send(&self, event: CaptureEvent) {
        let snapshot: Vec<SharedListener<CaptureEvent>> = {
            let guard = self.lock();
            guard.listeners.iter().map(|(_, l)| Arc::clone(l)).collect()
        };
        for listener in snapshot {
            let cb = listener.lock().unwrap_or_else(|e| e.into_inner());
            cb(&event);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EventState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::Timestamp;
    use std::sync::Arc;

    fn recording() -> RecordingState {
        RecordingState::Recording {
            started_at: Timestamp(0.0),
        }
    }

    #[test]
    fn a_new_subscriber_sees_the_current_state_immediately() {
        let hub = StateHub::new();
        hub.set(recording());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        hub.subscribe(Box::new(move |s| {
            sink.lock().unwrap().push(s.clone());
        }));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(seen.lock().unwrap()[0], recording());
    }

    #[test]
    fn every_subscriber_sees_every_change() {
        let hub = StateHub::new();
        let a = Arc::new(Mutex::new(Vec::new()));
        let b = Arc::new(Mutex::new(Vec::new()));
        let (sa, sb) = (Arc::clone(&a), Arc::clone(&b));
        hub.subscribe(Box::new(move |s| sa.lock().unwrap().push(s.clone())));
        hub.subscribe(Box::new(move |s| sb.lock().unwrap().push(s.clone())));
        hub.set(RecordingState::Stopping {});
        hub.set(RecordingState::Idle {});
        assert_eq!(a.lock().unwrap().len(), 3);
        assert_eq!(b.lock().unwrap().len(), 3);
        assert_eq!(hub.current(), RecordingState::Idle {});
    }

    #[test]
    fn unsubscribing_stops_delivery() {
        let hub = StateHub::new();
        let seen = Arc::new(Mutex::new(0));
        let sink = Arc::clone(&seen);
        let id = hub.subscribe(Box::new(move |_| *sink.lock().unwrap() += 1));
        hub.unsubscribe(id);
        hub.set(recording());
        assert_eq!(*seen.lock().unwrap(), 1, "only the initial call");
    }

    #[test]
    fn a_listener_may_read_the_hub_without_deadlocking() {
        let hub = Arc::new(StateHub::new());
        let inner = Arc::clone(&hub);
        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        // The shell's tray icon does exactly this: it is told about the change and then reads the state.
        hub.subscribe(Box::new(move |s| {
            let again = inner.current();
            assert_eq!(&again, s);
            *sink.lock().unwrap() = Some(again);
        }));
        hub.set(recording());
        assert_eq!(*seen.lock().unwrap(), Some(recording()));
    }

    #[test]
    fn events_reach_every_subscriber_in_order() {
        let hub = EventHub::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        hub.subscribe(Box::new(move |e| {
            sink.lock().unwrap().push(e.component);
        }));
        hub.send(CaptureEvent::screen(NssError::Io("stopped".into())));
        hub.send(CaptureEvent::device(
            Component::Microphone,
            NssError::Io("unplugged".into()),
        ));
        assert_eq!(
            *seen.lock().unwrap(),
            [Component::Screen, Component::Microphone]
        );
        assert_eq!(hub.subscriber_count(), 1);
    }

    #[test]
    fn screen_failures_are_fatal_and_device_failures_are_not() {
        let screen = CaptureEvent::screen(NssError::Io("gone".into()));
        assert!(screen.is_fatal);
        let mic = CaptureEvent::device(Component::Microphone, NssError::Io("gone".into()));
        assert!(!mic.is_fatal);
    }
}
