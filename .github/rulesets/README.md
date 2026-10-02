# Branch rulesets

GitHub rulesets are repository settings, not files: GitHub does not read this folder. These JSON
files are the rulesets we agreed on, kept here so changes to them are reviewed like code. The repo
owner applies them by hand.

| File | Applies to | Why |
|---|---|---|
| `master.json` | `master` | Everything reaches `master` through an approved PR with green CI. |
| `integration-branches.json` | `post-*`, `integration/*` | Umbrella branches that collect several PRs before going to `master` get the same review, so nothing lands in them unreviewed. |

Feature branches (anything else) have no rules on purpose: their authors must be able to push to
them freely, including after a PR is open. They are protected by the PR into a protected branch.
A ruleset that matches every branch blocks those pushes (`GH013: Changes must be made through a
pull request`), which is what happened on 2026-10-01.

Both rulesets:

- **Require a PR with 1 approval** that is not from the person who pushed last
  (`require_last_push_approval`), so an author cannot add commits after someone else's approval
  and merge them unreviewed.
- **Dismiss approvals when new commits are pushed** (`dismiss_stale_reviews_on_push`): an approval
  covers exactly the code that was reviewed.
- **Require review conversations to be resolved** before merging.
- **Require CI to pass** on both platforms (`check (ubuntu-latest)`, `check (macos-latest)`, the
  job names from `.github/workflows/ci.yml`; update them here if those change).
- **Allow only merge commits**, no squash or rebase merges. Stacked PRs (a PR based on another
  PR's branch) break when their base is squash-merged.
- **Block force-pushes.** `master` also blocks deletion; integration branches can be deleted once
  merged.
- **Have no bypass list**, including for the owner, so the rules apply to everyone.

Naming: give an umbrella branch a `post-` or `integration/` prefix (e.g. `post-m2-fixes`,
`integration/workspaces`) to get these rules.

## Applying them (repo owner)

From an up-to-date clone, either in the browser:

1. Settings → Rules → Rulesets → **New ruleset** → **Import a ruleset** → pick a file here.
2. Check the imported rules, then **Create**.
3. Delete the old rulesets this replaces (`Master Branch`, `master`, `master-rule`).

or with the GitHub CLI:

    gh api -X POST repos/Verifiedz/Shimmer/rulesets --input .github/rulesets/master.json
    gh api -X POST repos/Verifiedz/Shimmer/rulesets --input .github/rulesets/integration-branches.json
    gh api -X DELETE repos/Verifiedz/Shimmer/rulesets/23237114   # old "Master Branch"
    gh api -X DELETE repos/Verifiedz/Shimmer/rulesets/23405436   # old "master"
    gh api -X DELETE repos/Verifiedz/Shimmer/rulesets/23497827   # old "master-rule"

To change a ruleset later: edit its file here in a PR, then apply it with
`gh api -X PUT repos/Verifiedz/Shimmer/rulesets/<id> --input <file>` (or in Settings).

Check with `curl -s https://api.github.com/repos/Verifiedz/Shimmer/rules/branches/<branch>`:
`master` and `post-*` branches list the rules above, any other branch returns `[]`.

These are applied by hand on purpose. A GitHub Action could apply them automatically, but it
would need an owner-level token stored as a secret, letting anyone who can edit a workflow
rewrite the protection rules.
