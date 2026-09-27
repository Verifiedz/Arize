//! In-memory `Ctx` for module tests (§12 rule 13). Enable with the `testing` feature.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::bus::{Emitter, EventSink};
use crate::clock::Clock;
use crate::ctx::{Ctx, ModuleConfig};
use crate::error::{Error, Result};
use crate::event::Event;
use crate::http::{HttpBackend, HttpGateway, HttpResponse};
use crate::ids::{ModuleId, TaskId};
use crate::queue::{EnqueueRequest, QueueHandle, TaskHandle, TaskSubmitter};
use crate::store::{validate_path, StoreBackend, TxPlan, Write};

/// In-memory store that is also the event sink. Committed events are inspectable.
#[derive(Default)]
pub struct MemBackend {
    files: Mutex<BTreeMap<(String, String), Vec<u8>>>,
    events: Mutex<Vec<Event>>,
}

impl MemBackend {
    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }
}

impl StoreBackend for MemBackend {
    fn read(&self, ns: &str, path: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.files.lock().unwrap().get(&(ns.into(), path.into())).cloned())
    }

    fn list(&self, ns: &str, prefix: &str) -> Result<Vec<String>> {
        let dir = format!("{prefix}/");
        Ok(self
            .files
            .lock()
            .unwrap()
            .keys()
            .filter(|(n, p)| n == ns && (prefix.is_empty() || p.starts_with(&dir)))
            .map(|(_, p)| p.clone())
            .collect())
    }

    fn commit(&self, ns: &str, plan: TxPlan) -> Result<()> {
        for w in &plan.writes {
            let (Write::Put { path, .. } | Write::Delete { path }) = w;
            validate_path(path)?;
        }
        let mut files = self.files.lock().unwrap();
        for w in plan.writes {
            match w {
                Write::Put { path, bytes } => {
                    files.insert((ns.into(), path), bytes);
                }
                Write::Delete { path } => {
                    files.remove(&(ns.into(), path));
                }
            }
        }
        self.events.lock().unwrap().extend(plan.events);
        Ok(())
    }
}

impl EventSink for MemBackend {
    fn emit(&self, event: Event) -> Result<()> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

/// Records what a module enqueued; hands back sequential positions.
#[derive(Default)]
pub struct StubQueue {
    pub requests: Mutex<Vec<EnqueueRequest>>,
}

#[async_trait]
impl TaskSubmitter for StubQueue {
    async fn submit(&self, req: EnqueueRequest) -> Result<TaskHandle> {
        let mut r = self.requests.lock().unwrap();
        let lane = req.lane.clone().unwrap_or_else(crate::LaneId::default_lane);
        r.push(req);
        Ok(TaskHandle { task_id: TaskId::new(), lane, position: r.len() - 1 })
    }
}

/// Serves canned bodies by exact URL; anything else is `unavailable`.
#[derive(Default)]
pub struct StubHttp {
    pub responses: Mutex<BTreeMap<String, HttpResponse>>,
}

#[async_trait]
impl HttpBackend for StubHttp {
    async fn get(&self, _module: &ModuleId, url: &str) -> Result<HttpResponse> {
        self.responses.lock().unwrap().get(url).cloned().ok_or_else(|| Error::unavailable(format!("no stub for {url}")))
    }
}

/// A `Ctx` wired to in-memory fakes, with the fakes exposed for assertions.
pub struct TestEnv {
    pub ctx: Ctx,
    pub backend: Arc<MemBackend>,
    pub queue: Arc<StubQueue>,
    pub http: Arc<StubHttp>,
    pub clock: Clock,
}

impl TestEnv {
    /// Namespace equals the module id.
    pub fn new(module: &str) -> Self {
        let backend = Arc::new(MemBackend::default());
        let queue = Arc::new(StubQueue::default());
        let http = Arc::new(StubHttp::default());
        let clock = Clock::fake(chrono::DateTime::UNIX_EPOCH + chrono::Duration::days(20_000));
        let id = ModuleId::new(module);
        let ctx = Ctx::new(
            crate::NamespacedStore::new(module, id.clone(), backend.clone(), clock.clone()),
            HttpGateway::new(id.clone(), http.clone()),
            Emitter::new(id.clone(), backend.clone(), clock.clone()),
            QueueHandle::new(id.clone(), queue.clone()),
            clock.clone(),
            crate::time::LocalTimezone::UTC,
            CancellationToken::new(),
            ModuleConfig::default(),
            id,
        );
        Self { ctx, backend, queue, http, clock }
    }
}
