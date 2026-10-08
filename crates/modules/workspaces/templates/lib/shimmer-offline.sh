# Helpers for the offline-prep template (ADR 0025): your projects, what each one is built with,
# and how to download, check and build it for working with no network. Steps load it after
# shimmer-open.sh:
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"
#
# POSIX sh only. This is your copy: Shimmer never changes it after the workspace is created, so
# add an ecosystem of your own here if you like.

SHIMMER_RESULTS="$SHIMMER_STATE_DIR/results"
SHIMMER_OFFLINE_PARTS="machine repos deps docker build verify docs pages github ai"

# ---------------------------------------------------------------- results

# One line for the summary: "ok|warn|info<TAB>part<TAB>text".
shimmer_offline_result() {
    mkdir -p "$SHIMMER_STATE_DIR"
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >>"$SHIMMER_RESULTS"
}

# Is PART left out (SKIP), or one that needs big downloads under DATA_SAVER?
shimmer_offline_skipped() {
    case " $SKIP " in *" $1 "*) return 0 ;; esac
    if [ "${DATA_SAVER:-no}" = "yes" ]; then
        case "$1" in docker | ai) return 0 ;; esac
    fi
    return 1
}

# "1 commit", "3 commits".
shimmer_offline_count() {
    if [ "$1" -eq 1 ]; then echo "1 $2"; else echo "$1 ${2}s"; fi
}

# Run a command with a time limit when this system has `timeout` (macOS doesn't by default).
shimmer_offline_limit() {
    seconds=$1
    shift
    if shimmer_has timeout; then
        timeout "$seconds" "$@"
    else
        "$@"
    fi
}

# ---------------------------------------------------------------- paths

# A path with a leading ~ expanded.
shimmer_offline_expand() {
    case "$1" in
        "~") echo "$HOME" ;;
        "~/"*) echo "$HOME/${1#"~/"}" ;;
        *) echo "$1" ;;
    esac
}

# Where saved pages, the GitHub snapshot and Python wheels go (OFFLINE_DIR).
shimmer_offline_dir() {
    shimmer_offline_expand "${OFFLINE_DIR:-~/offline}"
}

# The name a repo in CLONE_REPOS gets as a folder: owner/repo or a URL → repo.
shimmer_offline_repo_name() {
    name=${1%/}
    name=${name##*/}
    name=${name##*:}
    echo "${name%.git}"
}

# Every project folder, one per line: PROJECTS (one per line in workspace.toml, or separated by
# spaces), then each CLONE_REPOS entry's folder in CLONE_DIR. Missing ones are listed too: the
# check step says which.
shimmer_offline_projects() {
    {
        case "$PROJECTS" in
            *"
"*) printf '%s\n' "$PROJECTS" ;;
            *) for p in $PROJECTS; do echo "$p"; done ;;
        esac
        for repo in $CLONE_REPOS; do
            echo "$(shimmer_offline_expand "${CLONE_DIR:-~/code}")/$(shimmer_offline_repo_name "$repo")"
        done
    } | while IFS= read -r p; do
        p=$(printf '%s' "$p" | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')
        [ -n "$p" ] && shimmer_offline_expand "$p"
    done | awk '!seen[$0]++'
}

# ---------------------------------------------------------------- ecosystems

# What a project is built with, from the files at its top, separated by spaces: rust node python
# go maven gradle ruby php dotnet elixir dart swift haskell zig deno.
shimmer_offline_kinds() {
    d=$1
    kinds=""
    [ -f "$d/Cargo.toml" ] && kinds="$kinds rust"
    [ -f "$d/package.json" ] && kinds="$kinds node"
    { [ -f "$d/pyproject.toml" ] || [ -f "$d/requirements.txt" ]; } && kinds="$kinds python"
    [ -f "$d/go.mod" ] && kinds="$kinds go"
    [ -f "$d/pom.xml" ] && kinds="$kinds maven"
    { [ -f "$d/build.gradle" ] || [ -f "$d/build.gradle.kts" ]; } && kinds="$kinds gradle"
    [ -f "$d/Gemfile" ] && kinds="$kinds ruby"
    [ -f "$d/composer.json" ] && kinds="$kinds php"
    ls "$d"/*.sln "$d"/*.csproj "$d"/*.fsproj >/dev/null 2>&1 && kinds="$kinds dotnet"
    [ -f "$d/mix.exs" ] && kinds="$kinds elixir"
    [ -f "$d/pubspec.yaml" ] && kinds="$kinds dart"
    [ -f "$d/Package.swift" ] && kinds="$kinds swift"
    { [ -f "$d/stack.yaml" ] || ls "$d"/*.cabal >/dev/null 2>&1; } && kinds="$kinds haskell"
    [ -f "$d/build.zig.zon" ] && kinds="$kinds zig"
    { [ -f "$d/deno.json" ] || [ -f "$d/deno.jsonc" ]; } && kinds="$kinds deno"
    echo "${kinds# }"
}

# The project's own wrapper (./gradlew, ./mvnw) when it has one, else the installed tool.
shimmer_offline_wrapper() {
    if [ -x "$1/$2" ]; then echo "./$2"; else echo "$3"; fi
}

# The Python tool a project uses: uv, poetry, or pip with requirements.txt.
shimmer_offline_python_tool() {
    if [ -f "$1/uv.lock" ] && shimmer_has uv; then
        echo uv
    elif [ -f "$1/poetry.lock" ] && shimmer_has poetry; then
        echo poetry
    elif [ -f "$1/requirements.txt" ]; then
        echo pip
    fi
}

# The pip of the project's own virtualenv, if it has one.
shimmer_offline_venv_pip() {
    for v in .venv venv; do
        [ -x "$1/$v/bin/pip" ] && {
            echo "$1/$v/bin/pip"
            return
        }
    done
}

# The command that downloads KIND's dependencies, run inside the project. Nothing when this
# machine doesn't have the tool (the caller says so).
shimmer_offline_fetch_command() {
    d=$1
    case "$2" in
        rust) shimmer_has cargo && echo "cargo fetch" ;;
        node)
            case "$(shimmer_package_manager "$d")" in
                pnpm) shimmer_has pnpm && echo "pnpm install --frozen-lockfile" ;;
                yarn) shimmer_has yarn && echo "yarn install --immutable || yarn install --frozen-lockfile" ;;
                bun) shimmer_has bun && echo "bun install --frozen-lockfile" ;;
                *)
                    shimmer_has npm || return 0
                    if [ -f "$d/package-lock.json" ]; then echo "npm ci"; else echo "npm install"; fi
                    ;;
            esac
            ;;
        python)
            case "$(shimmer_offline_python_tool "$d")" in
                uv) echo "uv sync" ;;
                poetry) echo "poetry install" ;;
                pip)
                    pip=$(shimmer_offline_venv_pip "$d")
                    wheels="$(shimmer_offline_dir)/wheels/$(basename "$d")"
                    if [ -n "$pip" ]; then
                        echo "$(shimmer_quote "$pip") install -r requirements.txt && $(shimmer_quote "$pip") download -q -r requirements.txt -d $(shimmer_quote "$wheels")"
                    elif shimmer_has python3; then
                        echo "python3 -m pip download -q -r requirements.txt -d $(shimmer_quote "$wheels")"
                    fi
                    ;;
            esac
            ;;
        go) shimmer_has go && echo "go mod download" ;;
        maven) echo "$(shimmer_offline_wrapper "$d" mvnw mvn) -q dependency:go-offline" ;;
        gradle) echo "$(shimmer_offline_wrapper "$d" gradlew gradle) --quiet dependencies" ;;
        ruby) shimmer_has bundle && echo "bundle install" ;;
        php) shimmer_has composer && echo "composer install --no-interaction" ;;
        dotnet) shimmer_has dotnet && echo "dotnet restore" ;;
        elixir) shimmer_has mix && echo "mix deps.get" ;;
        dart)
            if grep -q 'sdk: flutter' "$d/pubspec.yaml" 2>/dev/null; then
                shimmer_has flutter && echo "flutter pub get"
            else
                shimmer_has dart && echo "dart pub get"
            fi
            ;;
        swift) shimmer_has swift && echo "swift package resolve" ;;
        haskell)
            if [ -f "$d/stack.yaml" ]; then
                shimmer_has stack && echo "stack build --only-dependencies"
            else
                shimmer_has cabal && echo "cabal update && cabal build --only-dependencies"
            fi
            ;;
        zig) shimmer_has zig && echo "zig build --fetch" ;;
        deno) shimmer_has deno && echo "deno install" ;;
    esac
}

# The command that proves KIND works with no network, without using it: each tool's offline
# mode, or a look at what's on disk. Nothing when there's no such check.
shimmer_offline_verify_command() {
    d=$1
    case "$2" in
        rust) echo "cargo fetch --offline" ;;
        node)
            # Every dependency in package.json has its folder in node_modules (npm, pnpm, yarn
            # and bun all put them there). Not `npm ls`: it fails on harmless peer warnings.
            if shimmer_has node; then
                echo "node -e $(shimmer_quote 'const p = require("./package.json"), fs = require("fs"); const missing = Object.keys({ ...p.dependencies, ...p.devDependencies }).filter((d) => !fs.existsSync("node_modules/" + d)); if (missing.length) { console.error("missing from node_modules: " + missing.join(" ")); process.exit(1); }')"
            else
                echo "test -d node_modules"
            fi
            ;;
        python)
            case "$(shimmer_offline_python_tool "$d")" in
                uv) echo "uv sync --offline" ;;
                poetry) echo "test -d .venv || poetry env info -p >/dev/null" ;;
                pip)
                    wheels="$(shimmer_offline_dir)/wheels/$(basename "$d")"
                    echo "python3 -m pip install --dry-run -q --no-index --find-links $(shimmer_quote "$wheels") -r requirements.txt"
                    ;;
            esac
            ;;
        go) echo "GOFLAGS=-mod=mod GOPROXY=off go list -deps ./... >/dev/null" ;;
        maven) echo "$(shimmer_offline_wrapper "$d" mvnw mvn) -o -q dependency:resolve" ;;
        gradle) echo "$(shimmer_offline_wrapper "$d" gradlew gradle) --offline --quiet dependencies >/dev/null" ;;
        ruby) echo "bundle install --local" ;;
        php) echo "test -d vendor" ;;
        dart) echo "$(grep -q 'sdk: flutter' "$d/pubspec.yaml" 2>/dev/null && echo flutter || echo dart) pub get --offline" ;;
        elixir) echo "test -d deps" ;;
        swift) echo "test -d .build/checkouts" ;;
        deno) echo "deno install --cached-only" ;;
    esac
}

# The command that compiles KIND once (WARM_BUILD), so the first build offline is quick.
# Compiled languages only: a web app's build often reaches the network (fonts, telemetry).
shimmer_offline_build_command() {
    d=$1
    case "$2" in
        rust) echo "cargo build --offline && cargo test --offline --no-run" ;;
        go) echo "go build ./... && go test -run '^$' ./..." ;;
        maven) echo "$(shimmer_offline_wrapper "$d" mvnw mvn) -q -o test-compile" ;;
        gradle) echo "$(shimmer_offline_wrapper "$d" gradlew gradle) --offline --quiet testClasses" ;;
        dotnet) echo "dotnet build --no-restore" ;;
        swift) echo "swift build" ;;
        haskell) if [ -f "$d/stack.yaml" ]; then echo "stack build"; else echo "cabal build"; fi ;;
        zig) echo "zig build" ;;
    esac
}

# The project's compose files.
shimmer_offline_compose_files() {
    for f in compose.yaml compose.yml docker-compose.yaml docker-compose.yml; do
        [ -f "$1/$f" ] && echo "$1/$f"
    done
    return 0
}

# Hosts the project's .env files point at that aren't this machine: those need the network
# whatever is downloaded.
shimmer_offline_remote_hosts() {
    cat "$1"/.env "$1"/.env.local "$1"/.env.development 2>/dev/null |
        grep -v '^[[:space:]]*#' |
        grep -oE '[a-z][a-z0-9+.-]*://([^/@[:space:]"]*@)?[A-Za-z0-9.-]+' |
        sed -E 's|^[a-z0-9+.-]*://([^@]*@)?||' |
        grep '\.' | grep -vE '^(localhost|127\.|0\.0\.0\.0|host\.docker\.internal)' |
        sort -u | head -n 5 | tr '\n' ' ' | sed 's/ $//'
}

# The GitHub owner/repo of a project's origin, if it's on GitHub.
shimmer_offline_github_repo() {
    url=$(git -C "$1" remote get-url origin 2>/dev/null) || return 0
    case "$url" in
        *github.com[:/]*)
            repo=${url#*github.com}
            repo=${repo#[:/]}
            echo "${repo%.git}"
            ;;
    esac
}

# A URL as a file name: https://doc.rust-lang.org/book/ch01.html → doc.rust-lang.org-book-ch01.html
shimmer_offline_page_name() {
    name=$(printf '%s' "$1" | sed -E 's|^[a-z]+://||; s|[?#].*||; s|/+$||' | tr -c 'A-Za-z0-9._-' '-')
    case "$name" in *.html | *.htm) echo "$name" ;; *) echo "$name.html" ;; esac
}

# Run COMMAND inside folder DIR for the step's log, with nothing to answer (input is /dev/null).
shimmer_offline_run() {
    echo
    echo "== $(basename "$1"): $2"
    (cd "$1" && sh -c "$2") </dev/null
}

# Each existing project, as lines for a `while read -r project` loop.
shimmer_offline_existing() {
    shimmer_offline_projects | while IFS= read -r p; do
        [ -d "$p" ] && echo "$p"
    done
    return 0
}
