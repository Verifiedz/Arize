//! The `rss` kind (ADR 0028 §2a): any RSS 0.x/1.0/2.0 or Atom feed, configured by url rather
//! than compiled in. One instance per `[modules.fetchers.sources.<id>]` entry with
//! `kind = "rss"`, built by [`crate::source::resolve`] -- never part of the fixed
//! [`crate::source::registry`].

use async_trait::async_trait;
use serde_json::json;
use shimmer_core::{Ctx, Error, Result};

use crate::http;
use crate::source::{Method, Page, RawItem, Source};

pub struct Rss {
    id: String,
    url: String,
}

impl Rss {
    pub fn new(id: String, url: String) -> Self {
        Self { id, url }
    }
}

#[async_trait]
impl Source for Rss {
    fn id(&self) -> &str {
        &self.id
    }

    fn method(&self) -> Method {
        // A feed url is a known, fetcher-maintained endpoint -- the same reasoning as
        // hn-whoishiring's Firebase API (ADR 0028 §10) -- not a crawled site, so no
        // robots.txt check applies (ADR 0028 §2a).
        Method::Api
    }

    async fn fetch_page(&self, ctx: &Ctx, cursor: Option<&str>) -> Result<Page> {
        // A feed has no pagination concept (ADR 0028 §2a): everything comes back in one
        // page. `fetch_page` is still only ever called with `cursor: None` by the host
        // (it never follows a `next_cursor` this impl never returns), but guard it anyway
        // rather than silently re-fetching if that assumption ever changes elsewhere.
        if cursor.is_some() {
            return Ok(Page { items: Vec::new(), next_cursor: None });
        }

        let resp = http::require_ok(http::get(ctx, &self.url).await?, &self.url)?;
        let feed = feed_rs::parser::parse(resp.body.as_slice())
            .map_err(|e| Error::module_error(format!("rss: could not parse feed at {}: {e}", self.url)))?;

        // Zero entries is not treated as a failure the way an HTML selector matching
        // nothing is (`html::select_or_fail`) -- a feed can legitimately have no items yet
        // (a brand-new blog), so there is no "structure changed" signal to read into it.
        let items = feed
            .entries
            .into_iter()
            .map(|entry| {
                let url = entry.links.first().map(|l| l.href.clone()).unwrap_or_default();
                // Dedup key (ADR 0028 §2a): the entry's own id/guid, falling back to its
                // link -- feed-rs normally synthesizes a non-empty `id` even when the
                // underlying feed entry had none, but the fallback covers the edge case
                // where that synthesis still lands on an empty string.
                let source_id = if entry.id.trim().is_empty() { url.clone() } else { entry.id };
                let title = entry.title.map(|t| t.content).unwrap_or_else(|| url.clone());
                RawItem {
                    source_id,
                    title,
                    url,
                    raw_data: json!({
                        "summary": entry.summary.map(|t| t.content),
                        "published": entry.published,
                    }),
                }
            })
            .collect();

        Ok(Page { items, next_cursor: None })
    }
}

#[cfg(test)]
mod tests {
    use shimmer_core::testing::TestEnv;
    use shimmer_core::{ErrorCode, HttpResponse};

    use super::*;

    const ATOM_FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Example Blog</title>
  <id>urn:uuid:feed-1</id>
  <updated>2026-10-01T00:00:00Z</updated>
  <entry>
    <id>urn:uuid:entry-1</id>
    <title>First post</title>
    <link href="https://example.test/posts/1"/>
    <summary>The first post's summary.</summary>
    <updated>2026-10-01T00:00:00Z</updated>
  </entry>
  <entry>
    <id>urn:uuid:entry-2</id>
    <title>Second post</title>
    <link href="https://example.test/posts/2"/>
    <updated>2026-10-02T00:00:00Z</updated>
  </entry>
</feed>"#;

    const EMPTY_ATOM_FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Empty Blog</title>
  <id>urn:uuid:feed-empty</id>
  <updated>2026-10-01T00:00:00Z</updated>
</feed>"#;

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
            from_cache: false,
            stale: false,
            retry_after_secs: None,
        }
    }

    fn stub(env: &TestEnv, url: &str, resp: HttpResponse) {
        env.http.responses.lock().unwrap().insert(url.to_owned(), resp);
    }

    #[test]
    fn id_is_whatever_it_was_constructed_with_and_method_is_api() {
        let rss = Rss::new("my-team-blog".into(), "https://example.test/feed.xml".into());
        assert_eq!(rss.id(), "my-team-blog");
        assert_eq!(rss.method(), Method::Api);
    }

    #[tokio::test]
    async fn both_entries_parse_with_their_dedup_key_title_and_url() {
        let env = TestEnv::new("fetchers");
        stub(&env, "https://example.test/feed.xml", ok(ATOM_FEED));
        let rss = Rss::new("my-team-blog".into(), "https://example.test/feed.xml".into());

        let page = rss.fetch_page(&env.ctx, None).await.unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].source_id, "urn:uuid:entry-1");
        assert_eq!(page.items[0].title, "First post");
        assert_eq!(page.items[0].url, "https://example.test/posts/1");
        assert_eq!(page.items[0].raw_data["summary"], "The first post's summary.");
        assert_eq!(page.items[1].source_id, "urn:uuid:entry-2");
        assert!(page.next_cursor.is_none(), "a feed is always a single page");
    }

    #[tokio::test]
    async fn zero_entries_is_an_empty_page_not_a_failure() {
        let env = TestEnv::new("fetchers");
        stub(&env, "https://example.test/feed.xml", ok(EMPTY_ATOM_FEED));
        let rss = Rss::new("my-team-blog".into(), "https://example.test/feed.xml".into());

        let page = rss.fetch_page(&env.ctx, None).await.unwrap();
        assert!(page.items.is_empty());
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn a_second_page_is_never_fetched() {
        let env = TestEnv::new("fetchers");
        // Deliberately no stub for the url -- if this impl ever tried to re-fetch for a
        // cursor it never hands out, the test would fail with "no stub for...".
        let rss = Rss::new("my-team-blog".into(), "https://example.test/feed.xml".into());
        let page = rss.fetch_page(&env.ctx, Some("anything")).await.unwrap();
        assert!(page.items.is_empty());
    }

    #[tokio::test]
    async fn malformed_xml_is_a_classifiable_module_error() {
        let env = TestEnv::new("fetchers");
        stub(&env, "https://example.test/feed.xml", ok("not a feed at all"));
        let rss = Rss::new("my-team-blog".into(), "https://example.test/feed.xml".into());

        let err = rss.fetch_page(&env.ctx, None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ModuleError);
        assert!(err.message.contains("could not parse feed"), "{}", err.message);
    }
}
