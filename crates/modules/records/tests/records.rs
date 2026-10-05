//! The records module through its only entry points, `init` and `handle`, against the in-memory
//! `Ctx` (CLAUDE.md §12 rule 13): files and events are inspected on the fake backend.

use std::time::Duration;

use serde_json::{json, Value};
use shimmer_core::testing::TestEnv;
use shimmer_core::{ErrorCode, Module, Result, StoreBackend};
use shimmer_records::Records;

async fn setup() -> (Records, TestEnv) {
    let env = TestEnv::new("records");
    let records = Records::default();
    records.init(&env.ctx).await.unwrap();
    (records, env)
}

async fn call(r: &Records, env: &TestEnv, op: &str, params: Value) -> Result<Value> {
    r.handle(op, params, &env.ctx).await
}

fn file(env: &TestEnv, path: &str) -> Option<String> {
    env.backend.read("records", path).unwrap().map(|b| String::from_utf8(b).unwrap())
}

fn topics(env: &TestEnv) -> Vec<String> {
    env.backend.events().into_iter().map(|e| e.topic).collect()
}

async fn add_two_sum(r: &Records, env: &TestEnv) -> Value {
    call(
        r,
        env,
        "records.add",
        json!({"collection": "leetcode", "id": "two-sum",
        "fields": {"title": "Two Sum", "difficulty": "easy"}}),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn init_seeds_leetcode_once() {
    let (r, env) = setup().await;
    assert!(file(&env, "collections/leetcode.toml").unwrap().contains("id = \"leetcode\""));
    r.init(&env.ctx).await.unwrap();
    assert_eq!(topics(&env), ["records.collection.created"], "a second start must not seed again");

    let data = call(&r, &env, "records.collections", Value::Null).await.unwrap();
    assert_eq!(data["collections"][0]["id"], "leetcode");
    assert_eq!(data["collections"][0]["stamp_on_complete"], "last_solved");
}

#[tokio::test]
async fn add_writes_one_file_and_one_event() {
    let (r, env) = setup().await;
    let item = add_two_sum(&r, &env).await;
    assert_eq!(
        item,
        json!({"id": "two-sum", "status": "todo", "title": "Two Sum", "difficulty": "easy",
               "url": null, "last_solved": null})
    );
    let text = file(&env, "items/leetcode/two-sum.toml").unwrap();
    assert!(text.contains("status = \"todo\"") && text.contains("title = \"Two Sum\""), "{text}");

    let ev = env.backend.events().pop().unwrap();
    assert_eq!(ev.topic, "records.item.created");
    assert_eq!(
        ev.payload,
        json!({"collection": "leetcode", "id": "two-sum", "title": "Two Sum", "deadlines": {}, "item": item})
    );
}

#[tokio::test]
async fn add_rejects_bad_input_and_changes_nothing() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    let before = topics(&env).len();

    let cases = [
        (json!({"collection": "leetcode", "id": "two-sum", "fields": {"title": "Again"}}), ErrorCode::Conflict),
        (json!({"collection": "nope", "id": "x", "fields": {}}), ErrorCode::NotFound),
        (json!({"collection": "leetcode", "id": "Bad Id", "fields": {"title": "x"}}), ErrorCode::InvalidParams),
        (json!({"collection": "leetcode", "id": "x", "fields": {}}), ErrorCode::InvalidParams),
        (
            json!({"collection": "leetcode", "id": "x", "fields": {"title": "x", "difficulty": "trivial"}}),
            ErrorCode::InvalidParams,
        ),
        (
            json!({"collection": "leetcode", "id": "x", "fields": {"title": "x", "colour": "red"}}),
            ErrorCode::InvalidParams,
        ),
        (json!({"collection": "leetcode", "id": "x", "fields": {"title": "x", "url": null}}), ErrorCode::InvalidParams),
        (json!({"collection": "leetcode"}), ErrorCode::InvalidParams),
    ];
    for (params, code) in cases {
        let e = call(&r, &env, "records.add", params.clone()).await.unwrap_err();
        assert_eq!(e.code, code, "{params}: {}", e.message);
    }
    assert_eq!(topics(&env).len(), before, "a failed request emits nothing (all or nothing)");
    assert!(file(&env, "items/leetcode/x.toml").is_none());
}

#[tokio::test]
async fn unknown_collection_matches_the_mock_fixture() {
    let (r, env) = setup().await;
    let e = call(&r, &env, "records.list", json!({"collection": "nope"})).await.unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::NotFound, "no collection 'nope'"));
}

#[tokio::test]
async fn list_filters_orders_and_pages() {
    let (r, env) = setup().await;
    for (id, difficulty) in [("c-hard", "hard"), ("a-easy", "easy"), ("b-easy", "easy")] {
        call(
            &r,
            &env,
            "records.add",
            json!({"collection": "leetcode", "id": id,
            "fields": {"title": id, "difficulty": difficulty}}),
        )
        .await
        .unwrap();
    }
    let ids = |v: &Value| {
        v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>()
    };

    let all = call(&r, &env, "records.list", json!({"collection": "leetcode"})).await.unwrap();
    assert_eq!((ids(&all), all["total"].clone()), (vec!["a-easy".into(), "b-easy".into(), "c-hard".into()], json!(3)));

    let easy = call(&r, &env, "records.list", json!({"collection": "leetcode", "filter": {"difficulty": "easy"}}))
        .await
        .unwrap();
    assert_eq!(ids(&easy), ["a-easy", "b-easy"]);

    let page =
        call(&r, &env, "records.list", json!({"collection": "leetcode", "limit": 1, "offset": 1})).await.unwrap();
    assert_eq!((ids(&page), page["total"].clone()), (vec!["b-easy".into()], json!(3)), "total counts before paging");

    let unset =
        call(&r, &env, "records.list", json!({"collection": "leetcode", "filter": {"url": null}})).await.unwrap();
    assert_eq!(unset["total"], 3);

    for bad in [json!({"filter": {"dificulty": "easy"}}), json!({"limit": 0}), json!({"limit": 501})] {
        let mut params = bad.clone();
        params["collection"] = json!("leetcode");
        let e = call(&r, &env, "records.list", params).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidParams, "{bad}");
    }
}

#[tokio::test]
async fn update_sets_and_unsets_but_keeps_required_fields() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    let item = call(
        &r,
        &env,
        "records.update",
        json!({"collection": "leetcode", "id": "two-sum",
        "fields": {"url": "https://leetcode.com/problems/two-sum", "difficulty": null}}),
    )
    .await
    .unwrap();
    assert_eq!(item["url"], "https://leetcode.com/problems/two-sum");
    assert_eq!(item["difficulty"], Value::Null);
    assert!(!file(&env, "items/leetcode/two-sum.toml").unwrap().contains("difficulty"));
    let ev = env.backend.events().pop().unwrap();
    assert_eq!(
        (ev.topic.as_str(), ev.payload["changed"].clone()),
        ("records.item.updated", json!(["difficulty", "url"]))
    );

    let e =
        call(&r, &env, "records.update", json!({"collection": "leetcode", "id": "two-sum", "fields": {"title": null}}))
            .await
            .unwrap_err();
    assert!(e.message.contains("'title' is required"));
    let e = call(&r, &env, "records.update", json!({"collection": "leetcode", "id": "ghost", "fields": {}}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn complete_marks_done_and_stamps_today_each_time() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    let target = json!({"collection": "leetcode", "id": "two-sum"});

    let today = env.clock.now().date_naive().to_string();
    let item = call(&r, &env, "records.complete", target.clone()).await.unwrap();
    assert_eq!((item["status"].clone(), item["last_solved"].clone()), (json!("done"), json!(today)));

    // Solving it again on another day is another completion.
    env.clock.advance(Duration::from_secs(86_400));
    let tomorrow = env.clock.now().date_naive().to_string();
    let item = call(&r, &env, "records.complete", target.clone()).await.unwrap();
    assert_eq!(item["last_solved"], json!(tomorrow));
    let completed = topics(&env).iter().filter(|t| *t == "records.item.completed").count();
    assert_eq!(completed, 2);

    let got = call(&r, &env, "records.get", target).await.unwrap();
    assert_eq!(got, item);
}

#[tokio::test]
async fn complete_stamps_the_local_date_not_the_utc_date() {
    let (r, mut env) = setup().await;
    add_two_sum(&r, &env).await;
    // TestEnv's fake clock lands on an exact UTC midnight, so a timezone west of UTC is
    // still "yesterday" locally — exactly the ADR 0009 bug: stamping the UTC date instead
    // of the user's local date.
    env.ctx.local_tz = shimmer_core::LocalTimezone::parse("America/Los_Angeles").unwrap();
    let utc_today = env.clock.now().date_naive();
    let local_yesterday = utc_today.pred_opt().unwrap();

    let target = json!({"collection": "leetcode", "id": "two-sum"});
    let item = call(&r, &env, "records.complete", target).await.unwrap();
    assert_eq!(item["last_solved"], json!(local_yesterday.to_string()));
    assert_ne!(item["last_solved"], json!(utc_today.to_string()));
}

#[tokio::test]
async fn remove_deletes_the_file() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    let target = json!({"collection": "leetcode", "id": "two-sum"});
    assert_eq!(call(&r, &env, "records.remove", target.clone()).await.unwrap(), json!({"removed": true}));
    assert!(file(&env, "items/leetcode/two-sum.toml").is_none());
    assert_eq!(topics(&env).last().unwrap(), "records.item.removed");
    assert_eq!(call(&r, &env, "records.get", target).await.unwrap_err().code, ErrorCode::NotFound);
}

#[tokio::test]
async fn hand_edits_are_read_and_a_broken_file_does_not_hide_the_rest() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    env.ctx.store.write("items/leetcode/edited.toml", "title = \"Edited\"\nlast_solved = 2026-09-18\n").unwrap();
    env.ctx.store.write("items/leetcode/broken.toml", "status = \"maybe\"").unwrap();

    let all = call(&r, &env, "records.list", json!({"collection": "leetcode"})).await.unwrap();
    assert_eq!(all["total"], 2);
    assert_eq!(all["items"][0]["last_solved"], "2026-09-18");
    assert!(all["skipped"][0].as_str().unwrap().contains("'broken.toml'"));
}

#[tokio::test]
async fn a_broken_collection_file_fails_only_the_requests_that_use_it() {
    // ADR 0008. Issue #29: one bad file used to fail `records.collections` as a whole, and the
    // CLI asks for it before every command, so every collection became unusable.
    let (_, env) = setup().await;
    env.ctx.store.write("collections/broken.toml", "not [valid toml").unwrap();
    env.ctx.store.write("collections/Bad Name.toml", "[collection]\nid = \"x\"\nlabel = \"x\"\n").unwrap();

    let restarted = Records::default();
    restarted.init(&env.ctx).await.expect("a malformed collection must not fail init");

    let data = call(&restarted, &env, "records.collections", json!({})).await.unwrap();
    let ids: Vec<&str> = data["collections"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["leetcode"]);
    let skipped: Vec<&str> = data["skipped"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert!(skipped.iter().any(|s| s.starts_with("collection file 'broken.toml': ")), "{skipped:?}");
    assert!(skipped.iter().any(|s| s.starts_with("collection file 'Bad Name.toml': the file name")), "{skipped:?}");

    // The healthy collection keeps working.
    add_two_sum(&restarted, &env).await;
    let list = call(&restarted, &env, "records.list", json!({"collection": "leetcode"})).await.unwrap();
    assert_eq!(list["total"], 1);

    // A request that uses the broken one still fails, naming the file.
    let e = call(&restarted, &env, "records.list", json!({"collection": "broken"})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidParams);
    assert!(e.message.contains("'broken.toml'"), "{}", e.message);
}

#[tokio::test]
async fn collections_has_no_skipped_key_when_every_file_is_fine() {
    let (r, env) = setup().await;
    let data = call(&r, &env, "records.collections", json!({})).await.unwrap();
    assert!(data.get("skipped").is_none(), "{data}");
}

#[tokio::test]
async fn a_new_module_instance_sees_everything() {
    // What a daemon restart looks like to the module: fresh state, same files.
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    call(&r, &env, "records.complete", json!({"collection": "leetcode", "id": "two-sum"})).await.unwrap();

    let restarted = Records::default();
    restarted.init(&env.ctx).await.unwrap();
    let got = call(&restarted, &env, "records.get", json!({"collection": "leetcode", "id": "two-sum"})).await.unwrap();
    assert_eq!(got["status"], "done");
}

#[tokio::test]
async fn a_user_defined_collection_needs_no_code() {
    let (r, env) = setup().await;
    env.ctx
        .store
        .write(
            "collections/jobs.toml",
            r#"
            [collection]
            id = "jobs"
            label = "Job applications"
            stamp_on_complete = "applied_on"

            [[field]]
            name = "company"
            type = "string"
            required = true

            [[field]]
            name = "applied_on"
            type = "date"
            "#,
        )
        .unwrap();
    call(&r, &env, "records.add", json!({"collection": "jobs", "id": "acme", "fields": {"company": "Acme"}}))
        .await
        .unwrap();
    let item = call(&r, &env, "records.complete", json!({"collection": "jobs", "id": "acme"})).await.unwrap();
    assert_eq!(item["applied_on"], json!(env.clock.now().date_naive().to_string()));
}

// ---------------------------------------------------------------- ADR 0016: completion lifecycle

/// A job tracker that refuses a second completion, as `job-applications` will.
fn add_jobs_collection(env: &TestEnv) {
    env.ctx
        .store
        .write(
            "collections/jobs.toml",
            r#"
            [collection]
            id = "jobs"
            label = "Job applications"
            stamp_on_complete = "applied_on"
            repeat_complete = "refuse"

            [[field]]
            name = "company"
            type = "string"
            required = true

            [[field]]
            name = "stage"
            type = "enum"
            values = ["oa", "interview"]

            [[field]]
            name = "applied_on"
            type = "date"
            "#,
        )
        .unwrap();
}

#[tokio::test]
async fn reopen_writes_todo_and_its_own_event() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    let target = json!({"collection": "leetcode", "id": "two-sum"});
    let solved = call(&r, &env, "records.complete", target.clone()).await.unwrap();

    let item = call(&r, &env, "records.reopen", target.clone()).await.unwrap();
    assert_eq!(item["status"], "todo");
    assert_eq!(item["last_solved"], solved["last_solved"], "the stamp is kept by default");
    assert!(file(&env, "items/leetcode/two-sum.toml").unwrap().contains("status = \"todo\""));
    let ev = env.backend.events().pop().unwrap();
    assert_eq!(ev.topic, "records.item.reopened");
    assert_eq!(
        ev.payload,
        json!({"collection": "leetcode", "id": "two-sum", "title": "Two Sum", "deadlines": {}, "item": item})
    );

    // Reopening again: not done, so conflict, and nothing written.
    let before = topics(&env).len();
    let e = call(&r, &env, "records.reopen", target).await.unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::Conflict, "'two-sum' in 'leetcode' is not done"));
    assert_eq!(topics(&env).len(), before);
}

#[tokio::test]
async fn reopen_can_clear_the_stamp() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    call(&r, &env, "records.complete", json!({"collection": "leetcode", "id": "two-sum"})).await.unwrap();
    let item =
        call(&r, &env, "records.reopen", json!({"collection": "leetcode", "id": "two-sum", "clear_stamp": true}))
            .await
            .unwrap();
    assert_eq!(item["last_solved"], Value::Null);
    assert!(!file(&env, "items/leetcode/two-sum.toml").unwrap().contains("last_solved"));
}

#[tokio::test]
async fn complete_with_fields_is_one_write_and_one_event() {
    let (r, env) = setup().await;
    add_jobs_collection(&env);
    call(&r, &env, "records.add", json!({"collection": "jobs", "id": "acme", "fields": {"company": "Acme"}}))
        .await
        .unwrap();
    let before = topics(&env).len();

    let item = call(
        &r,
        &env,
        "records.complete",
        json!({"collection": "jobs", "id": "acme", "fields": {"stage": "oa", "applied_on": "2026-10-03"}}),
    )
    .await
    .unwrap();
    assert_eq!(
        (item["status"].clone(), item["stage"].clone(), item["applied_on"].clone()),
        (json!("done"), json!("oa"), json!("2026-10-03")),
        "fields set and the stamp back-dated"
    );
    assert_eq!(topics(&env)[before..], ["records.item.completed"], "one event, no separate update");

    // Bad fields: nothing written.
    call(&r, &env, "records.reopen", json!({"collection": "jobs", "id": "acme"})).await.unwrap();
    let before = topics(&env).len();
    let e =
        call(&r, &env, "records.complete", json!({"collection": "jobs", "id": "acme", "fields": {"stage": "offer"}}))
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidParams);
    assert_eq!(topics(&env).len(), before);
}

#[tokio::test]
async fn refuse_makes_a_second_completion_a_conflict_until_reopened() {
    let (r, env) = setup().await;
    add_jobs_collection(&env);
    let target = json!({"collection": "jobs", "id": "acme"});
    call(&r, &env, "records.add", json!({"collection": "jobs", "id": "acme", "fields": {"company": "Acme"}}))
        .await
        .unwrap();
    let first = call(&r, &env, "records.complete", target.clone()).await.unwrap();
    let file_after_first = file(&env, "items/jobs/acme.toml");
    let before = topics(&env).len();

    env.clock.advance(Duration::from_secs(86_400));
    let e = call(&r, &env, "records.complete", target.clone()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert_eq!(
        e.message,
        format!("'acme' in 'jobs' was already completed on {}", first["applied_on"].as_str().unwrap())
    );
    assert_eq!((topics(&env).len(), file(&env, "items/jobs/acme.toml")), (before, file_after_first));

    call(&r, &env, "records.reopen", target.clone()).await.unwrap();
    let again = call(&r, &env, "records.complete", target).await.unwrap();
    assert_ne!(again["applied_on"], first["applied_on"], "reopened on purpose, so it stamps the new day");
}

#[tokio::test]
async fn collections_always_say_what_a_repeat_completion_does() {
    let (r, env) = setup().await;
    add_jobs_collection(&env);
    let data = call(&r, &env, "records.collections", json!({})).await.unwrap();
    let repeat = |id: &str| {
        data["collections"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap()["repeat_complete"].clone()
    };
    assert_eq!(repeat("leetcode"), "restamp", "the default is filled in");
    assert_eq!(repeat("jobs"), "refuse");
}

// ---------------------------------------------------------------- ADR 0017: integration metadata

/// What a job-applications template will look like: a title, a unique URL, deadlines.
fn add_postings_collection(env: &TestEnv) {
    env.ctx
        .store
        .write(
            "collections/postings.toml",
            r#"
            [collection]
            id = "postings"
            label = "Postings"
            title = "{company}: {position}"
            stamp_on_complete = "applied_on"

            [[field]]
            name = "company"
            type = "string"
            required = true

            [[field]]
            name = "position"
            type = "string"

            [[field]]
            name = "url"
            type = "string"
            role = "url"
            unique = true

            [[field]]
            name = "stage"
            type = "enum"
            values = ["oa", "interview"]

            [[field]]
            name = "oa_deadline"
            type = "date"
            role = "deadline"

            [[field]]
            name = "applied_on"
            type = "date"

            [extra.calendar]
            lead_days = [3, 1]
            "#,
        )
        .unwrap();
}

async fn add_posting(r: &Records, env: &TestEnv, id: &str, fields: Value) -> Result<Value> {
    call(r, env, "records.add", json!({"collection": "postings", "id": id, "fields": fields})).await
}

#[tokio::test]
async fn events_carry_the_title_and_the_deadlines() {
    let (r, env) = setup().await;
    add_postings_collection(&env);
    add_posting(
        &r,
        &env,
        "amazon",
        json!({"company": "Amazon", "position": "SDE Intern", "oa_deadline": "2026-10-20", "applied_on": "2026-10-04"}),
    )
    .await
    .unwrap();
    let ev = env.backend.events().pop().unwrap();
    assert_eq!(ev.payload["title"], "Amazon: SDE Intern");
    assert_eq!(ev.payload["deadlines"], json!({"oa_deadline": "2026-10-20"}), "applied_on has no role: history");

    // Every item event says so, not just created.
    for (op, extra) in [("records.update", json!({"fields": {"stage": "oa"}})), ("records.complete", json!({}))] {
        let mut params = json!({"collection": "postings", "id": "amazon"});
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        call(&r, &env, op, params).await.unwrap();
        let ev = env.backend.events().pop().unwrap();
        assert_eq!(
            (ev.payload["title"].clone(), ev.payload["deadlines"]["oa_deadline"].clone()),
            (json!("Amazon: SDE Intern"), json!("2026-10-20")),
            "{op}"
        );
    }
}

#[tokio::test]
async fn a_unique_value_can_be_taken_only_once() {
    let (r, env) = setup().await;
    add_postings_collection(&env);
    let url = "https://amazon.jobs/123";
    add_posting(&r, &env, "amazon", json!({"company": "Amazon", "url": url})).await.unwrap();
    let before = topics(&env).len();

    let want = format!("url '{url}' is already used by 'amazon' in 'postings'");
    let e = add_posting(&r, &env, "dup", json!({"company": "Amazon", "url": url})).await.unwrap_err();
    assert_eq!((e.code, e.message.as_str()), (ErrorCode::Conflict, want.as_str()));
    assert!(file(&env, "items/postings/dup.toml").is_none());

    add_posting(&r, &env, "other", json!({"company": "Other"})).await.unwrap();
    let e = call(&r, &env, "records.update", json!({"collection": "postings", "id": "other", "fields": {"url": url}}))
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict, "update");
    let e =
        call(&r, &env, "records.complete", json!({"collection": "postings", "id": "other", "fields": {"url": url}}))
            .await
            .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict, "complete with fields");
    assert_eq!(topics(&env).len(), before + 1, "only 'other' was added; the refused writes emitted nothing");

    // A record keeps its own value: updating it with the same URL is fine.
    call(&r, &env, "records.update", json!({"collection": "postings", "id": "amazon", "fields": {"url": url}}))
        .await
        .unwrap();
    // Unset and "" never count: many records may have no URL.
    add_posting(&r, &env, "no-url-1", json!({"company": "A", "url": ""})).await.unwrap();
    add_posting(&r, &env, "no-url-2", json!({"company": "B", "url": ""})).await.unwrap();
}

#[tokio::test]
async fn hand_edited_duplicates_stay_readable_and_editable() {
    let (r, env) = setup().await;
    add_postings_collection(&env);
    for id in ["a", "b"] {
        env.ctx.store.write(&format!("items/postings/{id}.toml"), "company = \"X\"\nurl = \"https://same\"\n").unwrap();
    }
    let list = call(&r, &env, "records.list", json!({"collection": "postings"})).await.unwrap();
    assert_eq!(list["total"], 2);
    // Changing another field of a duplicate still works; only taking a used value fails.
    call(&r, &env, "records.update", json!({"collection": "postings", "id": "a", "fields": {"stage": "oa"}}))
        .await
        .unwrap();
}

#[tokio::test]
async fn changed_lists_only_fields_whose_value_changed() {
    let (r, env) = setup().await;
    add_postings_collection(&env);
    add_posting(&r, &env, "amazon", json!({"company": "Amazon", "stage": "oa"})).await.unwrap();
    let changed = |env: &TestEnv| env.backend.events().pop().unwrap().payload["changed"].clone();

    call(
        &r,
        &env,
        "records.update",
        json!({"collection": "postings", "id": "amazon", "fields": {"stage": "interview", "company": "Amazon"}}),
    )
    .await
    .unwrap();
    assert_eq!(changed(&env), json!(["stage"]), "company was sent but didn't change");

    call(&r, &env, "records.update", json!({"collection": "postings", "id": "amazon", "fields": {"stage": null}}))
        .await
        .unwrap();
    assert_eq!(changed(&env), json!(["stage"]), "unsetting is a change");

    call(&r, &env, "records.update", json!({"collection": "postings", "id": "amazon", "fields": {"stage": null}}))
        .await
        .unwrap();
    assert_eq!(changed(&env), json!([]), "nothing changed, but the request is still answered with an event");
}

#[tokio::test]
async fn collections_return_the_integration_keys() {
    let (r, env) = setup().await;
    add_postings_collection(&env);
    let data = call(&r, &env, "records.collections", json!({})).await.unwrap();
    let c = data["collections"].as_array().unwrap().iter().find(|c| c["id"] == "postings").unwrap().clone();
    assert_eq!(c["title"], "{company}: {position}");
    assert_eq!(c["extra"], json!({"calendar": {"lead_days": [3, 1]}}));
    let url = c["fields"].as_array().unwrap().iter().find(|f| f["name"] == "url").unwrap().clone();
    assert_eq!((url["role"].clone(), url["unique"].clone()), (json!("url"), json!(true)));
}

// ---------------------------------------------------------------- ADR 0018: ids

#[tokio::test]
async fn add_without_an_id_names_the_record_from_its_title() {
    let (r, env) = setup().await;
    let add =
        |title: &str| call(&r, &env, "records.add", json!({"collection": "leetcode", "fields": {"title": title}}));
    assert_eq!(add("Two Sum").await.unwrap()["id"], "two-sum");
    assert_eq!(add("Two Sum").await.unwrap()["id"], "two-sum-2", "never clashes");
    assert_eq!(add("Two Sum").await.unwrap()["id"], "two-sum-3");
    assert!(file(&env, "items/leetcode/two-sum-2.toml").is_some());
    let ev = env.backend.events().pop().unwrap();
    assert_eq!((ev.topic.as_str(), ev.payload["id"].clone()), ("records.item.created", json!("two-sum-3")));
}

#[tokio::test]
async fn add_without_a_usable_title_uses_todays_date() {
    let (r, env) = setup().await;
    let today = env.clock.now().date_naive().to_string();
    // A title with nothing Latin in it leaves no id: fall back to the date.
    let item =
        call(&r, &env, "records.add", json!({"collection": "leetcode", "fields": {"title": "شركة"}})).await.unwrap();
    assert_eq!(item["id"], format!("{today}-1"));

    // A collection with no title at all: the date too, counting up.
    env.ctx
        .store
        .write(
            "collections/notes.toml",
            "[collection]\nid = \"notes\"\nlabel = \"Notes\"\n[[field]]\nname = \"text\"\ntype = \"string\"\n",
        )
        .unwrap();
    for n in 1..=2 {
        let item =
            call(&r, &env, "records.add", json!({"collection": "notes", "fields": {"text": "hi"}})).await.unwrap();
        assert_eq!(item["id"], format!("{today}-{n}"));
    }
}

#[tokio::test]
async fn an_explicit_id_is_used_as_given() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    let e = call(
        &r,
        &env,
        "records.add",
        json!({"collection": "leetcode", "id": "two-sum", "fields": {"title": "Two Sum"}}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict, "an explicit id is never suffixed");
}

#[tokio::test]
async fn rename_moves_the_record_in_one_step() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    call(&r, &env, "records.complete", json!({"collection": "leetcode", "id": "two-sum"})).await.unwrap();

    let item =
        call(&r, &env, "records.rename", json!({"collection": "leetcode", "id": "two-sum", "new_id": "1-two-sum"}))
            .await
            .unwrap();
    assert_eq!((item["id"].clone(), item["status"].clone()), (json!("1-two-sum"), json!("done")), "values untouched");
    assert!(file(&env, "items/leetcode/two-sum.toml").is_none());
    assert!(file(&env, "items/leetcode/1-two-sum.toml").is_some());
    let ev = env.backend.events().pop().unwrap();
    assert_eq!(ev.topic, "records.item.renamed");
    assert_eq!(
        (ev.payload["id"].clone(), ev.payload["new_id"].clone(), ev.payload["title"].clone()),
        (json!("two-sum"), json!("1-two-sum"), json!("Two Sum"))
    );
    assert_eq!(ev.payload["item"], item);
}

#[tokio::test]
async fn rename_refuses_bad_or_taken_ids_and_changes_nothing() {
    let (r, env) = setup().await;
    add_two_sum(&r, &env).await;
    call(&r, &env, "records.add", json!({"collection": "leetcode", "id": "lru", "fields": {"title": "LRU"}}))
        .await
        .unwrap();
    let before = topics(&env).len();
    let rename = |id: &str, new_id: &str| {
        call(&r, &env, "records.rename", json!({"collection": "leetcode", "id": id, "new_id": new_id}))
    };
    for (id, new_id, code) in [
        ("two-sum", "lru", ErrorCode::Conflict),
        ("two-sum", "Bad Id", ErrorCode::InvalidParams),
        ("two-sum", "two-sum", ErrorCode::InvalidParams),
        ("ghost", "anything", ErrorCode::NotFound),
    ] {
        assert_eq!(rename(id, new_id).await.unwrap_err().code, code, "{id} -> {new_id}");
    }
    assert_eq!(topics(&env).len(), before);
    assert!(file(&env, "items/leetcode/two-sum.toml").is_some());
}

// ---------------------------------------------------------------- ADR 0019: queries

#[tokio::test]
async fn list_filters_searches_and_sorts_through_the_op() {
    let (r, env) = setup().await;
    for (id, title, difficulty) in [
        ("two-sum", "Two Sum", "easy"),
        ("two-sum-again", "Two Sum Again", "hard"),
        ("lru-cache", "LRU Cache", "medium"),
    ] {
        call(
            &r,
            &env,
            "records.add",
            json!({"collection": "leetcode", "id": id, "fields": {"title": title, "difficulty": difficulty}}),
        )
        .await
        .unwrap();
    }
    let list = |params: Value| {
        let mut p = json!({"collection": "leetcode"});
        p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
        call(&r, &env, "records.list", p)
    };
    let ids = |v: &Value| {
        v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>()
    };

    // By id, truly: not by file path ('-' < '.').
    assert_eq!(ids(&list(json!({})).await.unwrap()), ["lru-cache", "two-sum", "two-sum-again"]);
    // Enum order easy < medium < hard, descending.
    assert_eq!(ids(&list(json!({"sort": ["-difficulty"]})).await.unwrap()), ["two-sum-again", "lru-cache", "two-sum"]);
    assert_eq!(ids(&list(json!({"filter": {"difficulty": {"ne": "hard"}}})).await.unwrap()), ["lru-cache", "two-sum"]);
    let found = list(json!({"search": "two", "limit": 1})).await.unwrap();
    assert_eq!(
        (ids(&found), found["total"].clone()),
        (vec!["two-sum".to_owned()], json!(2)),
        "total counts before paging"
    );

    let e = list(json!({"filter": {"difficulty": {"lt": "hard"}}, "sort": ["nope"]})).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidParams);
}
