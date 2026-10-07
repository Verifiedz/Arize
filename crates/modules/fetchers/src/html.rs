//! A shared HTML-parsing helper for `Method::Scrape` sources (ADR 0028 §7), built on
//! `scraper` (pure Rust, ISC, no system dependency -- verified to build identically on both
//! CI legs, `.github/workflows/ci.yml:10`).

use scraper::{ElementRef, Html, Selector};
use shimmer_core::{Error, Result};

pub fn parse(body: &[u8]) -> Html {
    Html::parse_document(&String::from_utf8_lossy(body))
}

/// Every element matching `selector`, or a distinct, reported failure if none match -- a 200
/// page that doesn't contain what a source expects is a structure change (or a bug), not a
/// quietly empty result (ADR 0028 §7). `what` names what was being looked for, for the error
/// message; the caller (the orchestration that drives a `Source`, landing in a later change)
/// is responsible for turning this into `fetchers.fetch.failed`'s `reason: "parse_empty"`.
pub fn select_or_fail<'a>(doc: &'a Html, selector: &str, what: &str) -> Result<Vec<ElementRef<'a>>> {
    let parsed =
        Selector::parse(selector).map_err(|e| Error::internal(format!("invalid selector '{selector}': {e:?}")))?;
    let found: Vec<ElementRef<'a>> = doc.select(&parsed).collect();
    if found.is_empty() {
        return Err(Error::module_error(format!(
            "no elements matched '{selector}' looking for {what}; the page's structure may have changed"
        )));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use shimmer_core::ErrorCode;

    use super::*;

    const PAGE: &str = r#"<html><body>
        <div class="job"><a class="title" href="/jobs/1">Senior Rustacean</a></div>
        <div class="job"><a class="title" href="/jobs/2">Junior Rustacean</a></div>
    </body></html>"#;

    #[test]
    fn finds_every_matching_element() {
        let doc = parse(PAGE.as_bytes());
        let titles = select_or_fail(&doc, "a.title", "job titles").unwrap();
        assert_eq!(titles.len(), 2);
        assert_eq!(titles[0].text().collect::<String>(), "Senior Rustacean");
        assert_eq!(titles[1].value().attr("href"), Some("/jobs/2"));
    }

    #[test]
    fn no_matches_on_an_otherwise_fine_page_is_a_reported_failure_not_an_empty_success() {
        let doc = parse(PAGE.as_bytes());
        let err = select_or_fail(&doc, "div.listing-that-no-longer-exists", "job listings").unwrap_err();
        assert_eq!(err.code, ErrorCode::ModuleError);
        assert!(err.message.contains("job listings"), "{}", err.message);
    }

    #[test]
    fn an_invalid_selector_is_its_own_distinct_error() {
        let doc = parse(PAGE.as_bytes());
        let err = select_or_fail(&doc, ":::not-css", "anything").unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal, "a bad selector is a bug in the source, not the page");
    }

    #[test]
    fn non_utf8_bytes_are_handled_rather_than_panicking() {
        let doc = parse(&[0xff, 0xfe, b'<', b'p', b'>', b'h', b'i', b'<', b'/', b'p', b'>']);
        let found = select_or_fail(&doc, "p", "paragraphs").unwrap();
        assert_eq!(found.len(), 1);
    }
}
