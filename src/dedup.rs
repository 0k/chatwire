//! Remembering which events were already handed to the caller.
//!
//! A sync loop that reconnects can present the same event twice. The
//! connector keeps a bounded window of the event identifiers it has
//! already emitted, and drops repeats.

use std::collections::{HashSet, VecDeque};

/// A fixed-size set of recently seen event identifiers.
///
/// The window is bounded so that a long running connector does not
/// grow without limit. Events older than the window can be emitted a
/// second time, which is why the window must be comfortably larger
/// than anything a reconnect can replay.
#[derive(Debug)]
pub struct SeenEvents {
    capacity: usize,
    order: VecDeque<String>,
    ids: HashSet<String>,
}

impl SeenEvents {
    /// Create a window holding at most `capacity` identifiers.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero: a window that remembers nothing
    /// would silently disable deduplication.
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "a deduplication window must hold at least one event"
        );
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            ids: HashSet::with_capacity(capacity),
        }
    }

    /// Record an identifier, and say whether it is new.
    ///
    /// Returns `true` the first time an identifier is offered, and
    /// `false` for as long as it stays inside the window.
    pub fn insert(&mut self, event_id: &str) -> bool {
        if self.ids.contains(event_id) {
            return false;
        }
        if self.order.len() == self.capacity {
            let evicted = self
                .order
                .pop_front()
                .expect("a full window has a front element");
            self.ids.remove(&evicted);
        }
        self.order.push_back(event_id.to_owned());
        self.ids.insert(event_id.to_owned());
        true
    }

    /// How many identifiers the window currently holds.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether the window holds no identifier at all.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_event_is_accepted() {
        let mut seen = SeenEvents::new(4);
        assert!(seen.insert("$one"));
    }

    #[test]
    fn a_repeated_event_is_refused() {
        let mut seen = SeenEvents::new(4);
        seen.insert("$one");
        assert!(!seen.insert("$one"));
    }

    #[test]
    fn distinct_events_are_all_accepted() {
        let mut seen = SeenEvents::new(4);
        assert!(seen.insert("$one"));
        assert!(seen.insert("$two"));
        assert!(seen.insert("$three"));
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn the_window_never_grows_past_its_capacity() {
        let mut seen = SeenEvents::new(2);
        seen.insert("$one");
        seen.insert("$two");
        seen.insert("$three");
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn an_event_pushed_out_of_the_window_is_accepted_again() {
        let mut seen = SeenEvents::new(2);
        seen.insert("$one");
        seen.insert("$two");
        seen.insert("$three");
        assert!(seen.insert("$one"));
    }

    #[test]
    fn an_event_still_inside_the_window_stays_refused() {
        let mut seen = SeenEvents::new(3);
        seen.insert("$one");
        seen.insert("$two");
        seen.insert("$three");
        assert!(!seen.insert("$two"));
    }

    #[test]
    fn a_fresh_window_is_empty() {
        let seen = SeenEvents::new(8);
        assert!(seen.is_empty());
    }

    #[test]
    #[should_panic(expected = "at least one event")]
    fn a_zero_capacity_window_is_refused() {
        SeenEvents::new(0);
    }
}
