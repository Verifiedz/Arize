//! What `records.list` asks of a collection (ADR 0019): `filter`, `search` and `sort`, checked
//! against the collection once and then applied to each record's wire shape. Pure: no `Ctx`, no
//! I/O (§12 rule 10). The index (M4) will answer the same params.

use std::cmp::Ordering;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value};
use shimmer_core::{Error, Result};

use crate::schema::{Collection, FieldType};
use crate::values::{self, Now};

/// At most this many sort keys: enough for any real ordering, and a cap on silly requests.
const MAX_SORT_KEYS: usize = 5;

/// A checked `records.list` request.
#[derive(Debug)]
pub struct Query {
    conditions: Vec<(String, Kind, Op)>,
    search: Option<String>,
    sort: Vec<(String, Kind, Direction)>,
    /// The string and enum fields `search` looks in.
    searchable: Vec<String>,
    /// For comparing `datetime` values and plain dates as moments (ADR 0024 §1).
    tz: Tz,
}

/// What a key is, which decides the operators it takes and how its values compare.
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Id,
    Status,
    Field(FieldType, Vec<String>),
    /// A list field, by the type of its items (ADR 0024 §2).
    List(FieldType, Vec<String>),
}

#[derive(Debug)]
enum Op {
    Eq(Value),
    Ne(Value),
    Lt(Value),
    Lte(Value),
    Gt(Value),
    Gte(Value),
    In(Vec<Value>),
    Set(bool),
    Contains(String),
    /// The list holds this item (strings ignoring case).
    Has(Value),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Direction {
    Ascending,
    Descending,
}

impl Query {
    /// Check `filter`, `search` and `sort` against `c`. Every mistake is `invalid_params` naming
    /// the key, so a typo is never a filter that silently matches nothing.
    pub fn new(
        c: &Collection,
        filter: &Map<String, Value>,
        search: Option<&str>,
        sort: &[String],
        now: &Now,
    ) -> Result<Self> {
        let mut conditions = Vec::new();
        for (key, value) in filter {
            no_status(c, key)?;
            let kind = kind(c, key)
                .ok_or_else(|| Error::invalid_params(format!("cannot filter '{}' on unknown field '{key}'", c.id)))?;
            match value {
                // A plain value is exact equality, exactly as before ADR 0019 (`null` = unset).
                Value::Object(ops) => {
                    if ops.is_empty() {
                        return Err(bad(key, "an operator object needs at least one operator"));
                    }
                    for (name, arg) in ops {
                        conditions.push((key.clone(), kind.clone(), op(c, key, &kind, name, arg, now)?));
                    }
                }
                // On a list, a plain value is `has` and `null` is "unset" (ADR 0024 §2).
                Value::Null if matches!(kind, Kind::List(..)) => {
                    conditions.push((key.clone(), kind.clone(), Op::Set(false)))
                }
                plain if matches!(kind, Kind::List(..)) => {
                    conditions.push((key.clone(), kind.clone(), op(c, key, &kind, "has", plain, now)?))
                }
                plain => conditions.push((key.clone(), kind.clone(), Op::Eq(stored(key, &kind, plain, now)?))),
            }
        }

        let search = match search {
            Some(s) if s.trim().is_empty() => return Err(Error::invalid_params("search must not be empty")),
            Some(s) => Some(s.to_lowercase()),
            None => None,
        };

        if sort.len() > MAX_SORT_KEYS {
            return Err(Error::invalid_params(format!("sort takes at most {MAX_SORT_KEYS} keys")));
        }
        let mut keys = Vec::new();
        for raw in sort {
            let (name, direction) = match raw.strip_prefix('-') {
                Some(name) => (name, Direction::Descending),
                None => (raw.as_str(), Direction::Ascending),
            };
            no_status(c, name)?;
            let kind = kind(c, name)
                .ok_or_else(|| Error::invalid_params(format!("cannot sort '{}' on unknown field '{name}'", c.id)))?;
            if matches!(kind, Kind::List(..)) {
                return Err(Error::invalid_params(format!("'{name}' is a list; lists can't be sorted")));
            }
            if keys.iter().any(|(k, _, _): &(String, Kind, Direction)| k == name) {
                return Err(Error::invalid_params(format!("sort names '{name}' twice")));
            }
            keys.push((name.to_owned(), kind, direction));
        }

        let searchable = c
            .fields
            .iter()
            .filter(|f| matches!(f.item_kind(), FieldType::String | FieldType::Enum | FieldType::Ref))
            .map(|f| f.name.clone())
            .collect();
        Ok(Self { conditions, search, sort: keys, searchable, tz: now.tz })
    }

    /// Does this record (its wire shape) match every condition and the search?
    pub fn matches(&self, wire: &Value) -> bool {
        let got = |key: &str| wire.get(key).unwrap_or(&Value::Null);
        let conditions = self.conditions.iter().all(|(key, kind, op)| holds(kind, op, got(key), &self.tz));
        let search = self.search.as_deref().is_none_or(|needle| {
            std::iter::once("id")
                .chain(self.searchable.iter().map(String::as_str))
                .any(|k| texts(got(k)).any(|s| s.to_lowercase().contains(needle)))
        });
        conditions && search
    }

    /// Order records by the sort keys, unset values last in either direction, then by id.
    pub fn sort(&self, items: &mut [Value]) {
        items.sort_by(|a, b| {
            for (key, kind, direction) in &self.sort {
                let (x, y) =
                    (sort_value(kind, &a[key.as_str()], &self.tz), sort_value(kind, &b[key.as_str()], &self.tz));
                let order = match (x, y) {
                    (None, None) => Ordering::Equal,
                    (None, Some(_)) => Ordering::Greater,
                    (Some(_), None) => Ordering::Less,
                    (Some(x), Some(y)) => match direction {
                        Direction::Ascending => x.cmp(&y),
                        Direction::Descending => y.cmp(&x),
                    },
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            a["id"].as_str().cmp(&b["id"].as_str())
        });
    }
}

/// `status` on a collection that has none is its own error, not "unknown field" (ADR 0021 §1).
fn no_status(c: &Collection, key: &str) -> Result<()> {
    match key == "status" && !c.completable {
        true => Err(Error::invalid_params(format!("'{}' has no status (completable = false)", c.id))),
        false => Ok(()),
    }
}

fn kind(c: &Collection, key: &str) -> Option<Kind> {
    match key {
        "id" => Some(Kind::Id),
        "status" => Some(Kind::Status),
        // A ref filters and sorts like the id it holds (ADR 0024 §3).
        _ => c.field(key).map(|f| match (f.kind, f.item_kind()) {
            (FieldType::List, FieldType::Ref) => Kind::List(FieldType::String, Vec::new()),
            (FieldType::List, of) => Kind::List(of, f.values.clone()),
            (FieldType::Ref, _) => Kind::Field(FieldType::String, Vec::new()),
            (k, _) => Kind::Field(k, f.values.clone()),
        }),
    }
}

fn bad(key: &str, msg: &str) -> Error {
    Error::invalid_params(format!("filter on '{key}': {msg}"))
}

/// One operator, checked: allowed for the key's kind (ADR 0019 §1), with a value that fits.
fn op(c: &Collection, key: &str, kind: &Kind, name: &str, arg: &Value, now: &Now) -> Result<Op> {
    let ordered = match kind {
        Kind::Field(t, _) => matches!(t, FieldType::Int | FieldType::Date | FieldType::Datetime | FieldType::Enum),
        _ => false,
    };
    let list = matches!(kind, Kind::List(..));
    let allowed = match name {
        "has" => list,
        "set" => !matches!(kind, Kind::Id | Kind::Status),
        _ if list => false,
        "eq" | "ne" | "in" => true,
        "lt" | "lte" | "gt" | "gte" => ordered,
        "contains" => matches!(kind, Kind::Id | Kind::Field(FieldType::String, _)),
        other => return Err(bad(key, &format!("unknown operator '{other}'"))),
    };
    if !allowed {
        return Err(bad(key, &format!("'{name}' doesn't apply to {}", describe(kind))));
    }
    let value = |v: &Value| -> Result<Value> {
        let v = stored(key, kind, v, now)?;
        check_value(c, key, kind, &v)?;
        Ok(v)
    };
    Ok(match name {
        "eq" => Op::Eq(value(arg)?),
        "ne" => Op::Ne(value(arg)?),
        "lt" => Op::Lt(value(arg)?),
        "lte" => Op::Lte(value(arg)?),
        "gt" => Op::Gt(value(arg)?),
        "gte" => Op::Gte(value(arg)?),
        "in" => match arg.as_array() {
            Some(list) if !list.is_empty() => Op::In(list.iter().map(value).collect::<Result<_>>()?),
            _ => return Err(bad(key, "'in' takes a non-empty list")),
        },
        "set" => Op::Set(arg.as_bool().ok_or_else(|| bad(key, "'set' takes true or false"))?),
        "has" => Op::Has(value(arg)?),
        _ => match arg.as_str() {
            Some(s) if !s.is_empty() => Op::Contains(s.to_lowercase()),
            _ => return Err(bad(key, "'contains' takes some text")),
        },
    })
}

fn describe(kind: &Kind) -> String {
    match kind {
        Kind::Id => "the id".into(),
        Kind::Status => "status".into(),
        Kind::Field(t, _) => {
            let name = serde_json::to_value(t).unwrap_or_default().as_str().unwrap_or("?").to_owned();
            let article = if name.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
            format!("{article} {name} field")
        }
        Kind::List(..) => "a list field (use 'has' or 'set')".into(),
    }
}

/// A value compared against `key` must fit it: the field's own check, a string id, or a status.
fn check_value(c: &Collection, key: &str, kind: &Kind, v: &Value) -> Result<()> {
    match kind {
        Kind::Field(..) => c.field(key).map_or(Ok(()), |f| f.check(v)),
        Kind::List(..) => c.field(key).map_or(Ok(()), |f| f.check(&Value::Array(vec![v.clone()]))),
        Kind::Id if v.is_string() => Ok(()),
        Kind::Status if v == "todo" || v == "done" => Ok(()),
        Kind::Id => Err(bad(key, "the id is text")),
        Kind::Status => Err(bad(key, "status is \"todo\" or \"done\"")),
    }
}

/// A value someone typed for `key`, as it's stored: a `datetime` read in the configured timezone
/// (ADR 0024 §1). Everything else as given.
fn stored(key: &str, kind: &Kind, v: &Value, now: &Now) -> Result<Value> {
    match (kind, v) {
        (Kind::Field(FieldType::Datetime, _), Value::String(s)) => {
            values::datetime(s, &now.tz).map(Value::String).map_err(|want| bad(key, &format!("expected {want}")))
        }
        _ => Ok(v.clone()),
    }
}

/// Set means a value other than `null`, `""` or `[]` (ADR 0017's rule, ADR 0024 §2).
fn is_set(v: &Value) -> bool {
    !crate::schema::is_unset(v) && v.as_str() != Some("")
}

/// The text in a value: a string, or a list's string items.
fn texts(v: &Value) -> impl Iterator<Item = &str> {
    let list = v.as_array().map(|items| items.iter().filter_map(Value::as_str));
    v.as_str().into_iter().chain(list.into_iter().flatten())
}

fn holds(kind: &Kind, op: &Op, got: &Value, tz: &Tz) -> bool {
    let cmp = |want: &Value| -> Option<Ordering> { Some(sort_value(kind, got, tz)?.cmp(&sort_value(kind, want, tz)?)) };
    match op {
        Op::Eq(want) => got == want,
        // Unset matches: "stage is not rejected" includes records with no stage yet.
        Op::Ne(want) => got != want,
        Op::Lt(want) => cmp(want) == Some(Ordering::Less),
        Op::Lte(want) => matches!(cmp(want), Some(Ordering::Less | Ordering::Equal)),
        Op::Gt(want) => cmp(want) == Some(Ordering::Greater),
        Op::Gte(want) => matches!(cmp(want), Some(Ordering::Greater | Ordering::Equal)),
        Op::In(list) => is_set(got) && list.contains(got),
        Op::Set(want) => is_set(got) == *want,
        Op::Contains(needle) => got.as_str().is_some_and(|s| s.to_lowercase().contains(needle)),
        Op::Has(want) => got.as_array().is_some_and(|items| items.iter().any(|i| values::same(i, want))),
    }
}

/// A value as something that orders the way its type should: ints by number, dates by date,
/// enums by the order their values are listed, strings ignoring case. `None` when unset or when a
/// hand-edit left a value that doesn't fit the type: it never matches a comparison and sorts last.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Int(i64),
    Date(NaiveDate),
    Instant(DateTime<Utc>),
    Rank(usize),
    Text(String),
    Bool(bool),
}

fn sort_value(kind: &Kind, v: &Value, tz: &Tz) -> Option<Key> {
    if !is_set(v) {
        return None;
    }
    Some(match kind {
        Kind::Field(FieldType::Int, _) => Key::Int(v.as_i64()?),
        Kind::Field(FieldType::Date, _) => Key::Date(NaiveDate::parse_from_str(v.as_str()?, "%Y-%m-%d").ok()?),
        Kind::Field(FieldType::Datetime, _) => Key::Instant(values::instant(v.as_str()?, tz)?),
        Kind::Field(FieldType::Enum, values) => Key::Rank(values.iter().position(|x| Some(x.as_str()) == v.as_str())?),
        Kind::Field(FieldType::Bool, _) => Key::Bool(v.as_bool()?),
        Kind::Field(FieldType::String | FieldType::Ref, _) | Kind::Id | Kind::Status => {
            Key::Text(v.as_str()?.to_lowercase())
        }
        Kind::Field(FieldType::List, _) | Kind::List(..) => return None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shimmer_core::ErrorCode;

    use super::*;

    fn now() -> Now {
        Now {
            at: chrono::DateTime::parse_from_rfc3339("2026-10-05T12:00:00Z").unwrap().into(),
            tz: chrono_tz::America::Toronto,
        }
    }

    fn jobs() -> Collection {
        Collection::parse(
            "jobs",
            r#"
            [collection]
            id = "jobs"
            label = "Jobs"

            [[field]]
            name = "company"
            type = "string"

            [[field]]
            name = "stage"
            type = "enum"
            values = ["screening", "interviewing", "offer", "rejected"]

            [[field]]
            name = "salary"
            type = "int"

            [[field]]
            name = "oa_deadline"
            type = "date"

            [[field]]
            name = "remote"
            type = "bool"
            "#,
        )
        .unwrap()
    }

    fn job(id: &str, v: Value) -> Value {
        let mut w = json!({"id": id, "status": "todo", "company": null, "stage": null, "salary": null,
                           "oa_deadline": null, "remote": null});
        w.as_object_mut().unwrap().extend(v.as_object().unwrap().clone());
        w
    }

    fn query(filter: Value) -> Result<Query> {
        Query::new(&jobs(), filter.as_object().unwrap(), None, &[], &now())
    }

    fn sample() -> Vec<Value> {
        vec![
            job("acme", json!({"company": "Acme", "stage": "offer", "salary": 120, "oa_deadline": "2026-10-20"})),
            job(
                "google",
                json!({"company": "Google", "stage": "interviewing", "salary": 150, "oa_deadline": "2026-10-08", "remote": true}),
            ),
            job("initech", json!({"company": "Initech", "stage": "rejected", "salary": 90})),
            job("startup", json!({"company": "Startup", "remote": false})),
        ]
    }

    fn ids(q: &Query) -> Vec<String> {
        sample().iter().filter(|w| q.matches(w)).map(|w| w["id"].as_str().unwrap().to_owned()).collect()
    }

    #[test]
    fn lists_take_has_and_set_and_are_searched() {
        let c = Collection::parse(
            "p",
            "[collection]\nid = \"p\"\nlabel = \"P\"\n[[field]]\nname = \"stack\"\ntype = \"list\"\nof = \"string\"",
        )
        .unwrap();
        let q = |filter: Value, search: Option<&str>| Query::new(&c, filter.as_object().unwrap(), search, &[], &now());
        let rec = job("a", json!({"stack": ["Go", "Rust"]}));
        let none = job("b", json!({}));
        assert!(q(json!({"stack": {"has": "rust"}}), None).unwrap().matches(&rec), "ignoring case");
        assert!(!q(json!({"stack": {"has": "sql"}}), None).unwrap().matches(&rec));
        assert!(q(json!({"stack": {"set": false}}), None).unwrap().matches(&none));
        assert!(q(json!({}), Some("rus")).unwrap().matches(&rec), "search looks in list items");
        assert!(q(json!({"stack": "go"}), None).unwrap().matches(&rec), "a plain value is has");
        assert!(q(json!({"stack": null}), None).unwrap().matches(&none), "null is unset");
        assert!(q(json!({"stack": {"eq": "Go"}}), None).unwrap_err().message.contains("a list field"));
        let e = Query::new(&c, &Map::new(), None, &["stack".into()], &now()).unwrap_err();
        assert!(e.message.contains("can't be sorted"));
        assert!(query(json!({"company": {"has": "x"}})).unwrap_err().message.contains("doesn't apply"));
    }

    #[test]
    fn plain_values_are_exact_equality_as_before() {
        assert_eq!(ids(&query(json!({"stage": "offer"})).unwrap()), ["acme"]);
        assert_eq!(ids(&query(json!({"oa_deadline": null})).unwrap()), ["initech", "startup"], "null = unset");
        assert_eq!(ids(&query(json!({"status": "todo", "id": "google"})).unwrap()), ["google"]);
    }

    #[test]
    fn comparisons_on_dates_ints_and_enums() {
        let q = query(json!({"oa_deadline": {"gte": "2026-10-05", "lte": "2026-10-10"}})).unwrap();
        assert_eq!(ids(&q), ["google"], "a range; unset deadlines never match");
        assert_eq!(ids(&query(json!({"salary": {"gt": 100}})).unwrap()), ["acme", "google"]);
        assert_eq!(ids(&query(json!({"salary": {"lt": 100}})).unwrap()), ["initech"]);
        // Enums compare by the order their values are listed: made it to interviews or further.
        assert_eq!(ids(&query(json!({"stage": {"gte": "interviewing"}})).unwrap()), ["acme", "google", "initech"]);
    }

    #[test]
    fn ne_includes_unset_but_in_does_not() {
        assert_eq!(ids(&query(json!({"stage": {"ne": "rejected"}})).unwrap()), ["acme", "google", "startup"]);
        assert_eq!(ids(&query(json!({"stage": {"in": ["offer", "interviewing"]}})).unwrap()), ["acme", "google"]);
    }

    #[test]
    fn set_contains_and_search() {
        assert_eq!(ids(&query(json!({"oa_deadline": {"set": true}})).unwrap()), ["acme", "google"]);
        assert_eq!(ids(&query(json!({"remote": {"set": false}})).unwrap()), ["acme", "initech"]);
        assert_eq!(ids(&query(json!({"company": {"contains": "GOO"}})).unwrap()), ["google"], "ignores case");
        assert_eq!(ids(&query(json!({"id": {"contains": "tech"}})).unwrap()), ["initech"]);

        let search = |text: &str| Query::new(&jobs(), &Map::new(), Some(text), &[], &now()).unwrap();
        assert_eq!(ids(&search("ACME")), ["acme"]);
        assert_eq!(ids(&search("reject")), ["initech"], "enum values are searched too");
        assert_eq!(
            Query::new(&jobs(), &Map::new(), Some(" "), &[], &now()).unwrap_err().code,
            ErrorCode::InvalidParams
        );
    }

    #[test]
    fn mistakes_are_errors_naming_the_key() {
        for (filter, says) in [
            (json!({"stage": {"between": 1}}), "unknown operator 'between'"),
            (json!({"company": {"lt": "M"}}), "'lt' doesn't apply to a string field"),
            (json!({"remote": {"gt": true}}), "'gt' doesn't apply to a bool field"),
            (json!({"salary": {"contains": "1"}}), "'contains' doesn't apply to an int field"),
            (json!({"status": {"set": true}}), "'set' doesn't apply to status"),
            (json!({"oa_deadline": {"lt": "Friday"}}), "must be a date"),
            (json!({"stage": {"eq": "hired"}}), "must be one of"),
            (json!({"stage": {"in": []}}), "non-empty list"),
            (json!({"stage": {}}), "at least one operator"),
            (json!({"status": {"eq": "maybe"}}), "\"todo\" or \"done\""),
            (json!({"dificulty": "easy"}), "unknown field 'dificulty'"),
        ] {
            let e = query(filter.clone()).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidParams, "{filter}");
            assert!(e.message.contains(says), "{filter}: {}", e.message);
        }
    }

    #[test]
    fn sorting_puts_unset_last_in_either_direction() {
        let sorted = |keys: &[&str]| {
            let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
            let q = Query::new(&jobs(), &Map::new(), None, &keys, &now()).unwrap();
            let mut items = sample();
            q.sort(&mut items);
            items.iter().map(|w| w["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>()
        };
        assert_eq!(sorted(&["oa_deadline"]), ["google", "acme", "initech", "startup"]);
        assert_eq!(sorted(&["-oa_deadline"]), ["acme", "google", "initech", "startup"], "unset still last");
        assert_eq!(sorted(&["-salary"]), ["google", "acme", "initech", "startup"]);
        assert_eq!(sorted(&["stage"]), ["google", "acme", "initech", "startup"], "enum: listed order");
        assert_eq!(sorted(&[]), ["acme", "google", "initech", "startup"], "no keys: by id");
    }

    #[test]
    fn without_sort_ids_order_properly() {
        // `two-sum-again.toml` sorts before `two-sum.toml` as a path ('-' < '.'), not as an id.
        let q = Query::new(&jobs(), &Map::new(), None, &[], &now()).unwrap();
        let mut items = vec![job("two-sum-again", json!({})), job("two-sum", json!({}))];
        q.sort(&mut items);
        assert_eq!(items[0]["id"], "two-sum");
    }

    #[test]
    fn bad_sort_keys_are_errors() {
        let keys = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for (sort, says) in [
            (keys(&["nope"]), "unknown field 'nope'"),
            (keys(&["stage", "-stage"]), "twice"),
            (keys(&["id", "status", "stage", "salary", "company", "remote"]), "at most 5"),
        ] {
            let e = Query::new(&jobs(), &Map::new(), None, &sort, &now()).unwrap_err();
            assert!(e.message.contains(says), "{sort:?}: {}", e.message);
        }
    }
}
