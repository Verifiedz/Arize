//! Keeps the derived SQLite index current. It only ever *reads* the bus and the event log;
//! nothing else writes to the index (§1.4).

use std::sync::{Arc, Mutex};

use swe_store::{Index, Store};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

pub type SharedIndex = Arc<Mutex<Index>>;

/// Open the index and bring it fully up to date with the log before returning.
pub async fn open(store: Arc<Store>) -> swe_core::Result<SharedIndex> {
    tokio::task::spawn_blocking(move || {
        let mut index = Index::open(&store.index_path())?;
        index.catch_up(&store)?;
        Ok(Arc::new(Mutex::new(index)))
    })
    .await
    .map_err(|e| swe_core::Error::internal(format!("index open: {e}")))?
}

pub async fn run(
    index: SharedIndex,
    store: Arc<Store>,
    mut events: broadcast::Receiver<swe_core::Event>,
    shutdown: CancellationToken,
) {
    loop {
        let event = tokio::select! {
            _ = shutdown.cancelled() => return,
            e = events.recv() => e,
        };
        let (index, store) = (index.clone(), store.clone());
        let work = match event {
            Ok(e) => tokio::task::spawn_blocking(move || lock(&index).apply(&e)),
            // We missed events; the log has them all.
            Err(RecvError::Lagged(_)) => tokio::task::spawn_blocking(move || lock(&index).catch_up(&store)),
            Err(RecvError::Closed) => return,
        };
        match work.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!(error = %e, "index update failed; will heal on next catch-up"),
            Err(e) => tracing::error!(error = %e, "index task panicked"),
        }
    }
}

fn lock(index: &SharedIndex) -> std::sync::MutexGuard<'_, Index> {
    index.lock().unwrap_or_else(|e| e.into_inner())
}
