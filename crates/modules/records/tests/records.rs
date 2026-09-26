//! The records module through its only entry points, `init` and `handle`, against the in-memory
//! `Ctx` (CLAUDE.md §12 rule 13): files and events are inspected on the fake backend.

use std::time::Duration;

use serde_json::{json, Value};
use swe_core::testing::TestEnv;
use swe_core::{ErrorCode, Module, Result, StoreBackend};
use swe_records::Records;

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
    assert_eq!(ev.payload, json!({"collection": "leetcode", "id": "two-sum", "item": item}));
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
