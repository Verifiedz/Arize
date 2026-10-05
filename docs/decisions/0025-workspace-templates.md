# 0025. Workspace templates: ready-made workspaces you fill in with your own answers

Status: proposed · Raised by Dev B (issue #68) · Needs sign-off: Dev A (two new ops in
`workspaces`, docs/protocol.md; the scripts run through his `Launcher`) · Dev C notified ·
No `core` or `proto` change

## Context

A workspace today means writing a `workspace.toml` and a numbered shell script per step by hand
(ADRs 0010, 0012). That is the right format, but an empty folder is a poor start: most people
want "open my project the way a web developer does", not a lesson in process groups. It also
makes testing workspaces on real machines (#34) slow, because every test starts with writing
scripts from scratch.

Record collections solved the same problem with templates (ADR 0022). This ADR does the same
for workspaces, following ADR 0022 §5's convention so people learn it once:
`shimmer workspaces templates` lists them, `shimmer workspaces new ID --from TEMPLATE`
creates one, built-ins are embedded in the binary, creating never overwrites, and the result
is ordinary data that forgets where it came from.

What's different from records: a workspace runs scripts on someone's machine, so a template
can't be "just a file to copy". It needs the person's own paths, editor and links, and its
scripts must work on Linux and macOS without being rewritten for each person.

## Decision

### 1. A template is a workspace folder plus a list of questions

```
crates/modules/workspaces/templates/web-project/
  template.toml          the questions (this ADR)
  workspace.toml         an ordinary workspace.toml (ADR 0012), with no [env]
  steps/01-check.sh      ordinary step scripts
  steps/…
  cleanup.sh
  lib/shimmer-open.sh    shared helper (§5), copied with the rest
```

```toml
# template.toml
[template]
label = "Web project"
description = "Editor, dev server, localhost and your links (live site, host, repo) in one go"

[[question]]
name = "PROJECT_DIR"
prompt = "Project folder"
kind = "folder"
required = true

[[question]]
name = "CODE_EDITOR"
prompt = "Editor"
kind = "choice"
choices = ["vscode", "cursor", "zed", "webstorm", "intellij", "sublime", "neovim", "none"]
default = "vscode"

[[question]]
name = "LINKS"
prompt = "Other links to open (live site, hosting dashboard, analytics…)"
kind = "urls"
help = "Separate them with spaces. Leave empty for none."
```

The template's id is its folder name, like a workspace's (ADR 0012 §3).

**Question fields**

| Field | Required | Meaning |
|---|---|---|
| `name` | yes | The `[env]` variable the answer becomes (§2). |
| `prompt` | yes | What to ask, one line. |
| `kind` | yes | `text`, `folder`, `file`, `url`, `urls`, `choice` or `command` (§3). |
| `required` | no | Default `false`. A required answer can't be empty. |
| `default` | no | Used when the answer is left empty. |
| `choices` | `choice` only | The allowed answers. |
| `help` | no | One more line shown under the prompt. |

`template.toml` is checked like every other file Shimmer reads: unknown keys rejected, wrong
types rejected, every problem naming the file and the question.

### 2. Answers go into `[env]`; scripts are never rewritten

Creating a workspace writes every answer into its `workspace.toml` `[env]`, in question order,
**including empty ones** (`LINKS = ""`), so the person can see and fill in every setting later
by editing one table. The scripts are copied byte for byte. There is no text substitution
inside scripts: they read `"$PROJECT_DIR"` like any hand-written workspace does (ADR 0012 §1).
That keeps them readable, safe to inspect, and identical for everyone who uses the template.

Question names follow `[env]`'s rules (ADR 0012 §6: `[A-Za-z_][A-Za-z0-9_]*`, never
`SHIMMER_…`), and also may not be a variable other programs rely on, since `[env]` is passed
to everything the scripts start: `PATH`, `HOME`, `USER`, `SHELL`, `PWD`, `TMPDIR`, `LANG`,
`TERM`, `TERMINAL`, `EDITOR`, `VISUAL` and `BROWSER` are rejected. That is why the editor
question is `CODE_EDITOR`, not `EDITOR`.

### 3. Kinds, and what is checked when

| Kind | Checked by the daemon on create | Notes |
|---|---|---|
| `text` | one line | |
| `command` | one line | Run with `sh -c` by the script. `auto` is allowed where the template says so (§6). |
| `folder`, `file` | an absolute path, one line | The CLI turns `~/code/site` and `./site` into absolute paths before sending (§8). |
| `url` | starts with `http://` or `https://`, no spaces | |
| `urls` | each space-separated item is a `url` | URLs can't contain spaces, so a plain `for u in $LINKS` works in `sh`. |
| `choice` | one of `choices` | |

**The daemon does not check that a folder or file exists, or that an app is installed.**
Modules can't read outside their own namespace (CLAUDE.md §5), and the answer may be right
for the machine that runs the workspace even if it's wrong where it was created (a synced
`$SHIMMER_HOME`). Every template's first step checks those things instead (§4), every time it
launches, which is also when they matter.

`auto` is an ordinary answer, not a feature of the format: a template's question may default
to `auto`, and its scripts decide what that means when they run (§6). Working things out at
launch rather than at creation means a workspace keeps working when the project changes, say
from `npm` to `pnpm`.

### 4. Every template starts with a check step

Step 1 of every built-in template is `check`, supervised, with a short timeout. It confirms
what the rest needs (the folder exists, the editor is installed, the port is free) and exits
non-zero with **one clear sentence** if not: `Cursor isn't installed (looked for the 'cursor'
command and Cursor.app)`. The workspace goes `dirty` with that step and its log (CLAUDE.md
§10.3) before anything opens, instead of failing halfway with a half-opened desktop.

Steps whose answer is empty **do nothing and exit 0**, so one template covers a static site, a
backend with no browser tabs and a full web app.

Steps never fail because the network is down. Shimmer is local-first (CLAUDE.md §1.5): a
`git pull` that can't reach the server says so in the step's log and lets the launch continue.

### 5. Linux and macOS differences live in one helper

Opening an app, a folder, a URL or a terminal differs between platforms (`xdg-open` vs `open`,
`code` vs `open -a "Visual Studio Code"`). A template doesn't repeat that in every step:
`lib/shimmer-open.sh` provides small functions, and steps load it with
`. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"`.

- `shimmer_open_url URL…`: the default browser.
- `shimmer_open_editor EDITOR PATH`: the editor on a folder or file. On macOS it uses
  `open -a`, which works even when the editor's shell command isn't installed.
- `shimmer_open_terminal TERMINAL DIR [COMMAND]`: a terminal window in `DIR`, optionally
  running `COMMAND` (e.g. `claude`).
- `shimmer_has_editor EDITOR`: for the check step.

It branches on `$SHIMMER_PLATFORM` (CLAUDE.md §10.1), is POSIX `sh` (ADR 0012 §5: Debian's `sh`
is `dash`), and is copied into each created workspace like any other file, so the workspace
keeps working even if a later Shimmer changes the helper. Files outside `steps/` are already
allowed in a workspace folder (ADR 0012 §6 only checks `steps/`).

### 6. The first two built-ins

**`web-project`**: develop a website or web app. Nothing in it is specific to one person or one
host: your setup is a set of answers.

| Question | Kind | Default | Used by |
|---|---|---|---|
| `PROJECT_DIR` | folder, required | | every step |
| `CODE_EDITOR` | choice | `vscode` | check, editor |
| `OPEN_PATH` | text | empty = the project folder | editor: a file, or a `.code-workspace`, inside the project |
| `GIT_ON_OPEN` | choice `pull`, `none` | `pull` | git |
| `INSTALL_COMMAND` | command | `auto` | install |
| `SERVICES_COMMAND` | command | empty | services, e.g. `docker compose up -d` |
| `DEV_COMMAND` | command | `auto` | dev server |
| `LOCAL_URL` | url or `auto` | `auto` | wait, browser |
| `REPO_PAGE` | choice `home`, `pulls`, `issues`, `actions`, `none` | `home` | browser |
| `LINKS` | urls | empty | browser: live site, Vercel/Netlify/… dashboard, analytics |
| `TERMINAL_APP` | choice `auto`, `kitty`, `foot`, `alacritty`, `wezterm`, `ghostty`, `gnome-terminal`, `konsole`, `terminal`, `iterm`, `none` | `auto` | terminal |
| `TERMINAL_COMMAND` | command | empty | terminal, e.g. `claude` |

| # | Step | Mode | Does |
|---|---|---|---|
| 1 | `check` | supervised, 15 s | Folder exists; editor installed; tools the commands need exist; the local port isn't taken by something other than this workspace's own dev server. |
| 2 | `git` | supervised, 60 s | With `pull`: `git pull --ff-only` only when the working copy is clean and has an upstream. Offline or diverged: logged, never fatal. |
| 3 | `install` | supervised, 600 s | `auto`: the package manager's install, only when `node_modules` is missing or older than the lockfile. |
| 4 | `services` | supervised, 120 s | `SERVICES_COMMAND`, if any. |
| 5 | `editor` | detached | The editor on `OPEN_PATH` or the folder. |
| 6 | `dev-server` | detached | `DEV_COMMAND`, unless this workspace's dev server is already running (activating an `active` workspace again doesn't start a second one). |
| 7 | `wait` | supervised, 120 s | Waits until `LOCAL_URL` answers, so the browser never shows "can't connect". |
| 8 | `browser` | detached | `LOCAL_URL`, the repo page and `LINKS`. |
| 9 | `terminal` | detached | A terminal in the project, running `TERMINAL_COMMAND` if given. |
| | `cleanup.sh` | supervised, 30 s | Stops the dev server it started and frees the port. |

What `auto` means, decided when the step runs:

- **Package manager**, from the lockfile: `pnpm-lock.yaml` → pnpm, `yarn.lock` → yarn,
  `bun.lockb`/`bun.lock` → bun, otherwise npm. No `package.json`: no install, no dev command.
- **`DEV_COMMAND`**: `<pm> run dev` if `package.json` has a `dev` script, else `<pm> start` if it
  has `start`, else nothing.
- **`LOCAL_URL`**: from the framework in `package.json`: Next.js, Nuxt, Create React App →
  `http://localhost:3000`; Vite, SvelteKit → `:5173`; Astro → `:4321`; Gatsby → `:8000`.
  Unknown: no local tab, and `wait` does nothing.
- **Repo page**: from `git remote get-url origin`, with `git@github.com:you/site.git` turned
  into `https://github.com/you/site`. `pulls`/`issues`/`actions` are GitHub's paths; on other
  hosts only `home` is opened. No remote: nothing.
- **Node version**: when the project has `.nvmrc` or `.node-version` and `fnm`, `nvm` or
  `volta` is installed, the install and dev-server steps use that version.

**The dev server's output and process id** go in the system temp folder,
`${TMPDIR:-/tmp}/shimmer-<workspace id>/` (`dev-server.log`, `dev-server.pid`), never inside
`$SHIMMER_HOME`: only the daemon writes there (CLAUDE.md §2), and `logs/` is the launcher's
(ADR 0010 §3). The temp folder is the right lifetime too: a reboot clears it, and a reboot also
ends the server. A detached step is its own process group (ADR 0010 §3), so cleanup stops the
server and everything it started with one `kill` of that group.

**Secrets never go in answers.** `workspace.toml` is meant to be committable with the rest of
`$SHIMMER_HOME` (CLAUDE.md §7). API keys stay in the project's own `.env.local`. The template's
header comment says so, as the personal-data record templates do.

**`smoke-test`**: for testing workspaces themselves (#34). It opens no apps, so it behaves the
same on every machine and needs nothing installed.

| Question | Kind | Default |
|---|---|---|
| `FAIL_AT` | choice `none`, `check`, `cleanup` | `none` |

| # | Step | Mode | Does |
|---|---|---|---|
| 1 | `check` | supervised, 15 s | Prints every `SHIMMER_*` variable (checking the script ABI, CLAUDE.md §10.1) and runs `shimmer ping` through `SHIMMER_SOCKET` (checking the callback). Fails if `FAIL_AT = check`. |
| 2 | `wait` | supervised, 15 s | `sleep 2`: a supervised step that takes time. |
| 3 | `background` | detached | `sleep 300` in its own process group: a detached step that outlives the launch. |
| | `cleanup.sh` | supervised, 15 s | Stops `background`; fails if `FAIL_AT = cleanup`. |

So one template walks through `ready → launching → active`, `dirty` (with `FAIL_AT = check`),
cleanup back to `ready`, and a failed cleanup that stays `dirty`.

### 7. Two ops

| Op | Execution | Params | Result |
|---|---|---|---|
| `workspaces.templates` | inline | `{}` | `{"templates":[{"id","label","description","questions":[{"name","prompt","kind","required","default"?,"choices"?,"help"?}]}]}`, by id |
| `workspaces.create` | inline | `{"id","template","values"?,"label"?}` | the new workspace, as `workspaces.status` shows it |

`workspaces.create`:

- `id` must be a valid workspace id (ADR 0012 §6), so a request can never name a path outside
  the namespace.
- `template` must exist, or `not_found` naming the ones that do.
- `values` maps question names to strings. An unknown name, a missing required answer, or an
  answer that doesn't fit its kind (§3) is `invalid_params` naming the question. Left out means
  the default, or empty.
- **Never overwrites.** If anything exists under `data/workspaces/<id>/`, it is `conflict`.
- **Checked before writing**: the generated `workspace.toml` and the copied `steps/` go
  through ADR 0012's checks. A template that would produce an invalid workspace is a bug and
  fails loudly instead of being written.
- **One transaction** writes every file and emits `workspaces.workspace.created` with
  `{workspace, template}` (CLAUDE.md §7.1).
- The result **forgets it came from a template**: no `state.toml` (so it starts `ready`), no
  link back, and later changes to the template never touch it.

### 8. CLI

```
$ shimmer workspaces templates
ID           LABEL         DESCRIPTION
smoke-test   Smoke test    Opens nothing: checks that workspaces launch, fail and clean up
web-project  Web project   Editor, dev server, localhost and your links (live site, host, repo) in one go

$ shimmer workspaces new site --from web-project
Project folder: ~/code/site
Editor [vscode] (vscode, cursor, zed, webstorm, intellij, sublime, neovim, none): cursor
Other links to open (live site, hosting dashboard, analytics…) []: https://me.dev https://vercel.com/me/site/deployments
  Separate them with spaces. Leave empty for none.
…
✓ created workspace site from web-project
  start it: shimmer workspaces activate site
  change an answer: edit [env] in ~/.local/share/shimmer/data/workspaces/site/workspace.toml
```

- `shimmer workspaces new ID --from TEMPLATE [--label TEXT] [--set NAME=VALUE]…`.
- At a terminal it asks each question not given with `--set`, showing the default. Without a
  terminal it **never asks** (CLAUDE.md §15.1): unanswered questions take their defaults, and a
  missing required one is an error naming the `--set` to add.
- `folder` and `file` answers: the CLI expands `~` and makes relative paths absolute, since
  only the client knows the person's home and current folder.
- No pack aliases for these two, as for record templates (ADR 0022 §4).

## Consequences

- `crates/modules/workspaces`: a `templates/` folder embedded with `include_str!`, a pure
  `template.rs` (parse `template.toml`, check answers, build the `workspace.toml` text), the two
  ops, and tests against the in-memory `Ctx` (§12 rule 13): every built-in creates a workspace
  that passes ADR 0012, never overwrites, rejects bad answers, and launches against the
  `FakeLauncher`.
- The scripts are checked with `sh -n` in a test, and run for real in an end-to-end test of
  `smoke-test` (it needs nothing installed).
- `crates/cli`: `workspaces templates`, `workspaces new`.
- `docs/protocol.md`: both ops and the topic. `crates/mockd/fixtures/core.json` lists them.
  Dev C told.
- `web-project` is tested by hand on Linux and macOS before it ships: that is part of #34.

## Not done here

- More built-ins (a plain `project` template is next), user templates in `$SHIMMER_HOME`, and a
  community repository.
- Showing a project's tracker items on launch (`shimmer records list … --filter`), which needs
  the records query work (ADR 0019) on master first. A one-line step to add later.
- Placing windows on Hyprland workspaces or monitors, and `vercel env pull` (needs the Vercel
  CLI and a login): easy extra steps for anyone who wants them, not defaults.
- Windows (`.ps1` versions): the launch backend for Windows is still a sketch (ADR 0010 §7).
- Kits (#79), which create collections and a workspace together.
