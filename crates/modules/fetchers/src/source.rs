//! The extension point for a job board, feed, or any other place `fetchers` reaches
//! outward to (ADR 0028 §2). One `impl Source` per site, compiled in under [`crate::sources`]
//! -- `.dev/check-deps.py` bans a module depending on another module, so there is nowhere
//! else for one to live (CLAUDE.md §8: "about forty lines," Rust, not data, unlike a
//! `records` collection).

use async_trait::async_trait;
use serde_json::Value;
use shimmer_core::{Ctx, Result};

/// The host loops [`Source::fetch_page`] at most this many times per run, following
/// `next_cursor` -- a `Source` never loops itself.
pub const MAX_PAGES: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// A known, source-maintained endpoint. No robots.txt check (ADR 0028 §7) -- there is
    /// nothing to crawl.
    Api,
    /// A crawled page. The `Source` itself must check the target host's robots.txt (via
    /// [`crate::robots::allowed`]) before fetching -- the host has no way to know a scrape
    /// target's URL ahead of time to check it centrally.
    Scrape,
}

/// One item a page of a `Source` returned, before dedup.
#[derive(Clone, Debug, PartialEq)]
pub struct RawItem {
    /// The dedup key (ADR 0028 §4) -- stable and unique within this source, never a content
    /// hash.
    pub source_id: String,
    pub title: String,
    pub url: String,
    /// Becomes `fetchers.item.found`'s `raw_data`, capped by the caller (ADR 0028 §3) --
    /// a `Source` returns it uncapped.
    pub raw_data: Value,
}

#[derive(Debug)]
pub struct Page {
    pub items: Vec<RawItem>,
    /// `None` means there is nothing more to page through.
    pub next_cursor: Option<String>,
}

#[async_trait]
pub trait Source: Send + Sync {
    /// Also `fetchers.item.found`'s `source_name` and `data/fetchers/<id>/...`'s directory
    /// name.
    fn id(&self) -> &'static str;
    fn method(&self) -> Method;
    /// One page, given the previous page's `next_cursor` (`None` for the first page).
    async fn fetch_page(&self, ctx: &Ctx, cursor: Option<&str>) -> Result<Page>;
}

/// Every `Source` this build knows about (ADR 0028 §10). A compiled-in list, not a plugin
/// registry -- adding a source is adding a line here.
pub fn registry() -> Vec<std::sync::Arc<dyn Source>> {
    vec![std::sync::Arc::new(crate::sources::hn::HnWhoIsHiring)]
}

pub fn find(id: &str) -> Option<std::sync::Arc<dyn Source>> {
    registry().into_iter().find(|s| s.id() == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_has_hn_whoishiring_and_only_known_ids_resolve() {
        let all = registry();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id(), "hn-whoishiring");
        assert!(find("hn-whoishiring").is_some());
        assert!(find("not-a-real-source").is_none());
    }
}
