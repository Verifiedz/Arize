//! The event bus: a bounded broadcast. Each subscriber has its own 1024-slot queue, and a
//! slow one loses its oldest events and is told so (`core.stream.lagged`) instead of
//! stalling everyone else.

use shimmer_core::Event;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Event>,
}

impl Bus {
    pub fn new() -> Self {
        Self { tx: broadcast::channel(shimmer_proto::EVENT_QUEUE_CAPACITY).0 }
    }

    /// No subscribers is fine.
    pub fn publish(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
