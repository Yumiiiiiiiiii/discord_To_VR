//! Opt-in, bounded metadata only. Payloads, IDs and credentials never enter this API.
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_EVENTS: usize = 100;

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Live,
    Simulation,
    OverlayTest,
    System,
}
#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    ReceivedNotification,
    ReceivedMessage,
    FilteredMode,
    FilteredBlocked,
    FilteredDuplicate,
    FilteredSelf,
    FilteredEmpty,
    Queued,
    Validated,
    Sent,
    DropNoOverlay,
    DropFull,
    DropExpired,
    DropPolicy,
    DropDisconnected,
    DropSendFailed,
    DropInvalid,
    DropStopped,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub received: u64,
    pub filtered: u64,
    pub queued: u64,
    pub validated: u64,
    pub sent: u64,
    pub dropped: u64,
    pub last_received_at_ms: Option<u64>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: u64,
    pub at_ms: u64,
    pub source: Source,
    pub outcome: Outcome,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub started_at_ms: u64,
    pub live: Counts,
    pub tests: Counts,
    pub events: Vec<Event>,
}
struct State {
    started_at_ms: u64,
    next: u64,
    live: Counts,
    tests: Counts,
    events: VecDeque<Event>,
}
#[derive(Clone)]
pub struct Diagnostics(Arc<Mutex<State>>);
#[derive(Clone)]
pub struct Trace {
    log: Diagnostics,
    source: Source,
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
impl Default for Diagnostics {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            started_at_ms: now_ms(),
            next: 0,
            live: Counts::default(),
            tests: Counts::default(),
            events: VecDeque::new(),
        })))
    }
}
impl Diagnostics {
    pub fn trace(&self, source: Source) -> Trace {
        Trace {
            log: self.clone(),
            source,
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            started_at_ms: state.started_at_ms,
            live: state.live.clone(),
            tests: state.tests.clone(),
            events: state.events.iter().rev().cloned().collect(),
        }
    }
    pub fn clear(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.started_at_ms = now_ms();
        state.live = Counts::default();
        state.tests = Counts::default();
        state.events.clear();
    }
}
impl Trace {
    pub fn record(&self, outcome: Outcome) {
        let mut state = self.log.0.lock().unwrap_or_else(|e| e.into_inner());
        let at_ms = now_ms();
        if self.source != Source::System {
            let counts = if self.source == Source::Live {
                &mut state.live
            } else {
                &mut state.tests
            };
            if matches!(
                outcome,
                Outcome::ReceivedNotification | Outcome::ReceivedMessage
            ) {
                counts.last_received_at_ms = Some(at_ms);
            }
            let count = match outcome {
                Outcome::ReceivedNotification | Outcome::ReceivedMessage => &mut counts.received,
                Outcome::FilteredMode
                | Outcome::FilteredBlocked
                | Outcome::FilteredDuplicate
                | Outcome::FilteredSelf
                | Outcome::FilteredEmpty => &mut counts.filtered,
                Outcome::Queued => &mut counts.queued,
                Outcome::Validated => &mut counts.validated,
                Outcome::Sent => &mut counts.sent,
                _ => &mut counts.dropped,
            };
            *count = count.saturating_add(1);
        }
        state.next = state.next.saturating_add(1);
        let id = state.next;
        state.events.push_back(Event {
            id,
            at_ms,
            source: self.source,
            outcome,
        });
        if state.events.len() > MAX_EVENTS {
            state.events.pop_front();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_bounded_and_separates_real_receives_from_local_tests() {
        let log = Diagnostics::default();
        let live = log.trace(Source::Live);
        let test = log.trace(Source::Simulation);
        live.record(Outcome::ReceivedNotification);
        for _ in 0..150 {
            test.record(Outcome::ReceivedMessage);
            test.record(Outcome::FilteredMode);
        }
        let snapshot = log.snapshot();
        assert_eq!(snapshot.live.received, 1);
        assert_eq!(snapshot.tests.received, 150);
        assert_eq!(snapshot.tests.sent, 0);
        assert_eq!(snapshot.events.len(), MAX_EVENTS);
        assert!(snapshot
            .events
            .windows(2)
            .all(|pair| pair[0].id > pair[1].id));
        log.clear();
        assert_eq!(log.snapshot().live.received, 0);
        assert!(log.snapshot().events.is_empty());
    }
}
