//! The only way out to the network. Rate limiting, caching and backoff live behind the
//! backend, in the daemon; a module never holds a raw HTTP client.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::ids::ModuleId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait HttpBackend: Send + Sync {
    /// `module` lets the gateway attribute and rate-limit per caller.
    async fn get(&self, module: &ModuleId, url: &str) -> Result<HttpResponse>;
}

#[derive(Clone)]
pub struct HttpGateway {
    module: ModuleId,
    backend: Arc<dyn HttpBackend>,
}

impl HttpGateway {
    pub fn new(module: ModuleId, backend: Arc<dyn HttpBackend>) -> Self {
        Self { module, backend }
    }

    pub async fn get(&self, url: &str) -> Result<HttpResponse> {
        self.backend.get(&self.module, url).await
    }
}
