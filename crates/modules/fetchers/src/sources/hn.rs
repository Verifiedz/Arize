//! Hacker News "Who is hiring?" via its official, public, unauthenticated Firebase API
//! (ADR 0028 §10) -- no robots.txt applies; this is an API host, not a crawled site, and
//! the API is explicitly meant for third-party clients.
//!
//! Flow: `user/whoishiring.json` for the submitted story ids (newest first) -> the latest
//! one's `item/<id>.json` for its `kids` (top-level comments, each a job posting) -> one
//! `item/<id>.json` fetch per comment. Each `fetch_page` call re-fetches the thread to get
//! `kids` again rather than caching it itself (the trait is stateless across calls, see
//! [`crate::source::Source`]) -- cheap after the first call, since `ctx.http`'s own gateway
//! cache (ADR 0027) serves the repeat within its TTL.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use shimmer_core::{Ctx, Error, Result};

use crate::http;
use crate::source::{Method, Page, RawItem, Source};

const BASE: &str = "https://hacker-news.firebaseio.com/v0";
/// Comment items fetched per `fetch_page` call. Deliberately modest -- the per-run new-item
/// cap (`dedup::MAX_NEW_PER_RUN`, enforced by the orchestration that drives this trait) is
/// what actually bounds a whole run; this just keeps one page's own fan-out small.
const PAGE_SIZE: usize = 20;

pub struct HnWhoIsHiring;

#[derive(Deserialize)]
struct UserItem {
    #[serde(default)]
    submitted: Vec<u64>,
}

#[derive(Deserialize)]
struct HnItem {
    id: u64,
    #[serde(default)]
    kids: Vec<u64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    by: Option<String>,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    dead: bool,
}

#[async_trait]
impl Source for HnWhoIsHiring {
    fn id(&self) -> &'static str {
        "hn-whoishiring"
    }

    fn method(&self) -> Method {
        Method::Api
    }

    async fn fetch_page(&self, ctx: &Ctx, cursor: Option<&str>) -> Result<Page> {
        let (thread_id, offset) = match cursor {
            Some(c) => parse_cursor(c)?,
            None => (latest_thread(ctx).await?, 0),
        };
        let thread = item(ctx, thread_id).await?;
        let kids = thread.kids;
        let start = offset.min(kids.len());
        let end = (offset + PAGE_SIZE).min(kids.len());

        let mut items = Vec::new();
        for &kid in &kids[start..end] {
            let comment = item(ctx, kid).await?;
            if comment.deleted || comment.dead {
                continue;
            }
            let Some(text) = comment.text else { continue };
            items.push(RawItem {
                source_id: comment.id.to_string(),
                title: summarize(&text),
                url: format!("https://news.ycombinator.com/item?id={}", comment.id),
                raw_data: json!({"by": comment.by, "text": text}),
            });
        }
        let next_cursor = (end < kids.len()).then(|| format!("{thread_id}:{end}"));
        Ok(Page { items, next_cursor })
    }
}

async fn latest_thread(ctx: &Ctx) -> Result<u64> {
    let url = format!("{BASE}/user/whoishiring.json");
    let resp = http::require_ok(http::get(ctx, &url).await?, &url)?;
    let user: UserItem = parse_json(&resp.body, &url)?;
    user.submitted
        .first()
        .copied()
        .ok_or_else(|| Error::module_error(format!("parse_empty: {url}: no submitted threads")))
}

async fn item(ctx: &Ctx, id: u64) -> Result<HnItem> {
    let url = format!("{BASE}/item/{id}.json");
    let resp = http::require_ok(http::get(ctx, &url).await?, &url)?;
    parse_json(&resp.body, &url)
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8], url: &str) -> Result<T> {
    serde_json::from_slice(body).map_err(|e| Error::module_error(format!("parse_empty: {url}: {e}")))
}

fn parse_cursor(c: &str) -> Result<(u64, usize)> {
    let bad = || Error::invalid_params(format!("bad cursor '{c}'"));
    let (thread, offset) = c.split_once(':').ok_or_else(bad)?;
    Ok((thread.parse().map_err(|_| bad())?, offset.parse().map_err(|_| bad())?))
}

/// A short, human-readable label for a job posting comment, since HN comments carry no
/// title of their own -- the first non-empty line of its text, HTML-unescaped and stripped
/// of the `<p>` paragraph breaks the API's text field uses in place of real markup,
/// truncated to a sane length.
fn summarize(text: &str) -> String {
    let unescaped = text.replace("<p>", "\n").replace("&amp;", "&").replace("&gt;", ">").replace("&lt;", "<");
    let first_line = unescaped.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let truncated: String = first_line.chars().take(120).collect();
    if truncated.is_empty() {
        "(untitled)".to_owned()
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use shimmer_core::testing::TestEnv;
    use shimmer_core::{ErrorCode, HttpResponse};

    use super::*;

    const USER: &str = include_str!("../../tests/fixtures/hn/user_whoishiring.json");
    const USER_EMPTY: &str = include_str!("../../tests/fixtures/hn/user_whoishiring_empty.json");
    const THREAD: &str = include_str!("../../tests/fixtures/hn/thread_45000001.json");
    const COMMENT_10: &str = include_str!("../../tests/fixtures/hn/comment_45000010.json");
    const COMMENT_11_DELETED: &str = include_str!("../../tests/fixtures/hn/comment_45000011.json");
    const COMMENT_12: &str = include_str!("../../tests/fixtures/hn/comment_45000012.json");
    const MALFORMED: &str = include_str!("../../tests/fixtures/hn/malformed.json");

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
            from_cache: false,
            stale: false,
            retry_after_secs: None,
        }
    }

    fn stub(env: &TestEnv, path: &str, resp: HttpResponse) {
        env.http.responses.lock().unwrap().insert(format!("{BASE}{path}"), resp);
    }

    fn stub_full_thread(env: &TestEnv) {
        stub(env, "/user/whoishiring.json", ok(USER));
        stub(env, "/item/45000001.json", ok(THREAD));
        stub(env, "/item/45000010.json", ok(COMMENT_10));
        stub(env, "/item/45000011.json", ok(COMMENT_11_DELETED));
        stub(env, "/item/45000012.json", ok(COMMENT_12));
    }

    #[tokio::test]
    async fn the_first_page_skips_deleted_comments_and_covers_the_whole_short_thread() {
        let env = TestEnv::new("fetchers");
        stub_full_thread(&env);

        let page = HnWhoIsHiring.fetch_page(&env.ctx, None).await.unwrap();
        // 3 kids, well under PAGE_SIZE, so this is the whole thread in one page: 10 and 12
        // are valid, 11 is deleted and must not become an item.
        assert_eq!(page.items.len(), 2, "45000011 is deleted and must not become an item");
        assert_eq!(page.items[0].source_id, "45000010");
        assert_eq!(page.items[0].url, "https://news.ycombinator.com/item?id=45000010");
        assert!(page.items[0].title.contains("Acme Corp"), "{}", page.items[0].title);
        assert_eq!(page.items[1].source_id, "45000012");
        assert!(page.next_cursor.is_none(), "the whole thread fit in one page");
    }

    #[test]
    fn id_is_hn_whoishiring_and_method_is_api() {
        assert_eq!(HnWhoIsHiring.id(), "hn-whoishiring");
        assert_eq!(HnWhoIsHiring.method(), Method::Api);
    }

    #[tokio::test]
    async fn a_resumed_cursor_does_not_re_fetch_the_latest_thread_lookup() {
        let env = TestEnv::new("fetchers");
        // Deliberately NOT stubbing /user/whoishiring.json: if the resumed-cursor path tried
        // to look the latest thread up again, this call would fail with "no stub for...".
        stub(&env, "/item/45000001.json", ok(THREAD));
        stub(&env, "/item/45000012.json", ok(COMMENT_12));

        let page = HnWhoIsHiring.fetch_page(&env.ctx, Some("45000001:2")).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].source_id, "45000012");
        assert!(page.next_cursor.is_none(), "every kid has now been paged through");
    }

    #[tokio::test]
    async fn no_stub_at_all_is_a_transport_failure_not_a_module_error() {
        let env = TestEnv::new("fetchers"); // nothing stubbed
        let err = HnWhoIsHiring.fetch_page(&env.ctx, None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Unavailable);
    }

    #[tokio::test]
    async fn a_404_response_is_a_classifiable_http_error() {
        let env = TestEnv::new("fetchers");
        stub(
            &env,
            "/user/whoishiring.json",
            HttpResponse { status: 404, body: Vec::new(), from_cache: false, stale: false, retry_after_secs: None },
        );
        let err = HnWhoIsHiring.fetch_page(&env.ctx, None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ModuleError);
        assert!(err.message.starts_with("http_error:"), "{}", err.message);
    }

    #[tokio::test]
    async fn malformed_json_is_a_classifiable_parse_failure() {
        let env = TestEnv::new("fetchers");
        stub(&env, "/user/whoishiring.json", ok(MALFORMED));
        let err = HnWhoIsHiring.fetch_page(&env.ctx, None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ModuleError);
        assert!(err.message.starts_with("parse_empty:"), "{}", err.message);
    }

    #[tokio::test]
    async fn no_submitted_threads_at_all_is_also_a_classifiable_parse_failure() {
        let env = TestEnv::new("fetchers");
        stub(&env, "/user/whoishiring.json", ok(USER_EMPTY));
        let err = HnWhoIsHiring.fetch_page(&env.ctx, None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ModuleError);
        assert!(err.message.starts_with("parse_empty:"), "{}", err.message);
    }

    #[test]
    fn summarize_strips_markup_and_truncates() {
        assert_eq!(summarize("<p>Acme Corp | Remote<p>We build things."), "Acme Corp | Remote");
        assert_eq!(summarize(""), "(untitled)");
        let long = "x".repeat(500);
        assert_eq!(summarize(&long).chars().count(), 120);
    }
}
