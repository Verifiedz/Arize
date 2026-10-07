use async_trait::async_trait;
use serde_json::Value;
use shimmer_core::{Ctx, Error, Manifest, Module, Result};

#[derive(Default)]
pub struct Fetchers;

#[async_trait]
impl Module for Fetchers {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fetchers".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            namespace: "fetchers".into(),
            // Nothing emitted yet; filled in alongside the code that actually emits it
            // (ADR 0028 §3, §9), same pattern `workspaces` and `records` already follow.
            topics: Vec::new(),
            // Gets the real `ctx.http` once a `Source` actually needs it (ADR 0028 §10).
            capabilities: Vec::new(),
        }
    }

    /// Nothing to set up yet: no `Source` is registered, so there is nothing to recover or
    /// seed.
    async fn init(&self, _ctx: &Ctx) -> Result<()> {
        Ok(())
    }

    async fn handle(&self, op: &str, _params: Value, _ctx: &Ctx) -> Result<Value> {
        Err(Error::unknown_op(format!("fetchers has no op '{op}'")))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shimmer_core::testing::TestEnv;
    use shimmer_core::ErrorCode;

    use super::*;

    #[test]
    fn manifest_identifies_the_module_with_nothing_declared_yet() {
        let m = Fetchers.manifest();
        assert_eq!(m.id.as_str(), "fetchers");
        assert_eq!(m.namespace, "fetchers");
        assert!(m.topics.is_empty());
        assert!(m.capabilities.is_empty());
    }

    #[tokio::test]
    async fn init_succeeds_with_nothing_registered() {
        let env = TestEnv::new("fetchers");
        Fetchers.init(&env.ctx).await.unwrap();
    }

    #[tokio::test]
    async fn an_unknown_op_is_rejected_rather_than_silently_accepted() {
        let env = TestEnv::new("fetchers");
        let err = Fetchers.handle("fetchers.fetch", json!({}), &env.ctx).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownOp);
    }
}
