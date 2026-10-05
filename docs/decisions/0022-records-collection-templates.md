# 0022. Collection templates: ready-made trackers created with one command

Status: proposed · Raised by Dev B (records roadmap, item 1; issue #78) · Needs sign-off: Dev A
(two new ops in `records`, docs/protocol.md) · Dev C notified · No `core` or `proto` change

## Context

CLAUDE.md §1 calls Shimmer "a records engine that a LeetCode tracker is a forty-line
configuration of", and §8 lists what that engine is for: job applications, LeetCode, OSS repos,
outreach, career fairs, saved links. Today a fresh install has one collection, LeetCode, seeded
on first start (ADR 0008), and **no op creates another**: a job tracker means learning the
collection format and writing a file into `data/records/collections/` by hand.

ADRs 0016–0021 gave collections what real trackers need: titles, deadline and URL roles, unique
fields, `repeat_complete`, generated ids, queries, safe removal and reference lists. This ADR
packages that into templates, starting with job hunting, the use case Shimmer's M6 story is
built around (§1.3: an OA deadline turns into a notification).

## Decision

### 1. A template is an ordinary collection file

Built-in templates live in `crates/modules/records/templates/<id>.toml`, compiled into the binary
with `include_str!`. Each is a normal collection file (ADR 0008 and everything since) whose
`[collection] id` is the template's own id. There is no second format: the collection parser
checks templates, and a template is exactly the file the user gets.

`leetcode.toml` moves from `collections/` into `templates/`. Seeding on first start (ADR 0008)
uses it from there and behaves exactly as before.

### 2. Two ops

| Op | Params | Result |
|---|---|---|
| `records.templates` | `{}` | `{"templates":[…]}`: each template as `records.collections` shows a collection, sorted by id |
| `records.create_collection` | `{"id","template","label"?}` | the new collection, as `records.collections` shows it |

`records.create_collection`:

- `id` must be a valid collection id (`invalid_params`), so a request can never name a path
  outside `collections/`.
- `template` must exist, or `not_found` naming the ones that do.
- **Never overwrites.** If `collections/<id>.toml` exists, even malformed, it is `conflict`.
- **The template's text is copied, comments and all**, with only `[collection] id` (and `label`,
  when given) changed, using the same comment-keeping edit as ADR 0021.
- **Checked before writing**: the result goes through the collection parser. A template that
  would produce an invalid collection is a bug and fails loudly instead of being written.
- **One transaction** writes the file and emits `records.collection.created` with
  `{collection, template}` (the topic seeding already uses).
- The result **forgets it came from a template**: it is the user's file, edited like any other,
  and later improvements to the template never change it.

### 3. The first set

**`job-applications`**: one record per job you're tracking.

- `title = "{company}: {position}"`.
- `status`: `todo` = saved, not applied yet; `done` = applied. `records complete` stamps
  `applied_on`, and `repeat_complete = "refuse"`: applying happens once.
- Fields: `position`*, `company`* (required); from the posting, `url` (role `url`, `unique`, so
  the same posting can't be saved twice), `employment_type`, `level`, `location`, `job_id`,
  `term`, `duration`, `salary`, `applicants` (how many have applied, from the posting site),
  `posted_on`, `description`; for your own tracking, `stage`,
  `priority`, `source`, `referral`, `apply_by`, `oa_deadline`, `follow_up_on` (all three role
  `deadline`, so the calendar can remind you), `applied_on`, `notes`.
- `stage` is the coarse pipeline *after* applying (`screening`, `assessment`, `interviewing`,
  `offer`, `accepted`, `declined`, `rejected`, `withdrawn`, `ghosted`; empty = waiting), in
  that order, so `--filter "stage>=interviewing"` and `--sort stage` follow the pipeline (ADR
  0019). Whether you applied is `status`, not a stage.
- A heatmap view on `applied_on`.
- `related = ["interviews"]`.

**`interviews`**: one record per round, any number of rounds of any kind.

- `title = "{application}: {kind}"`. `records complete` = the round happened (stamps `done_on`,
  `repeat_complete = "refuse"`).
- Fields: `application`* (the job record's id: plain text until reference fields exist, roadmap
  item 13), `kind`* (`recruiter-screen`, `oa`, `take-home`, `case-study`, `technical`,
  `system-design`, `behavioral`, `hiring-manager`, `team-match`, `other`), `round`,
  `scheduled_on` (role `deadline`), `format`, `interviewer`, `prep`, `outcome` (`waiting`,
  `passed`, `failed`), `done_on`, `notes`.
- `related = ["job-applications"]`.

Each template's comments explain its fields and how completing works, since the file is the only
documentation the user will open.

### 4. CLI

```
$ shimmer records templates
ID                LABEL             DESCRIPTION
interviews        Interviews        One record per interview round, linked to a job application
job-applications  Job applications  Position, company, stage, deadlines and the date you applied
leetcode          LeetCode          Problems by difficulty, with a heatmap of solves

$ shimmer records new jobs --from job-applications
✓ created collection jobs from job-applications
  add one: shimmer records add jobs --position … --company …
  goes with: interviews (shimmer records new interviews --from interviews)
```

- `records new ID --from TEMPLATE [--label TEXT]`. `--from` is required: a collection with no
  fields is not useful, and the list is short.
- **No pack aliases** for these two: setting up a tracker is occasional, like the commands in
  ADR 0021 §6.

### 5. The convention for templates in any module

Workspace templates (#68) and any later kind follow the same rules, so people learn them once:

1. `shimmer <thing> templates` lists them; `shimmer <thing> new <id> --from <template>` creates
   one. The ops are `<module>.templates` and a create op in the same module.
2. **Each module owns its templates.** A module can only write its own namespace and can't call
   another (CLAUDE.md §1.3, §5), so there is no shared "templates" module.
3. Built-ins are **embedded in the binary**, and a test creates every one.
4. Creating **never overwrites** (`conflict`), is **checked with the module's normal parser
   before writing**, and writes everything in **one transaction with one event**.
5. The result is **ordinary data** that forgets where it came from.

No code is shared yet: the overlap is a lookup of a few lines. If workspace templates repeat it,
it moves into `core` then, with an ADR, as ADR 0014 did for other helpers.

## Consequences

- `crates/modules/records`: a `templates/` folder (with `leetcode.toml` moved in), the two ops,
  a test that creates every template and checks it, op tests against the in-memory `Ctx`
  (§12 rule 13).
- `crates/cli`: `records templates`, `records new`.
- `docs/protocol.md`: both ops; `records.collection.created` gains `template`.
  `crates/mockd/fixtures/core.json` lists the ops. Dev C told.

## Not done here

- **More templates**: background info (`education`, `employment`, `addresses`, reference lists
  via ADR 0021) and the rest of §8's list (`oss-contributions`, `outreach`, `career-fairs`,
  `reading-list`, `saved-links`). Each is a new file in `templates/`, no code (roadmap item 14).
- **User templates** in `$SHIMMER_HOME`, and a community template repository.
- **Kits** that create several collections and a workspace together (#79).
- **Reference fields** so `interviews.application` is a real link (#80).
