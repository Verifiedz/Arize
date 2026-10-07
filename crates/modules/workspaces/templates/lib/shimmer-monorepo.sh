# Helpers for the monorepo template (ADR 0025): which tool a monorepo uses, and how that tool
# runs several apps' dev servers. Steps load it after shimmer-open.sh:
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-monorepo.sh"
#
# POSIX sh only. This is your copy: Shimmer never changes it after the workspace is created.

# The monorepo tool, from what's at the root: turbo, nx, pnpm, yarn, bun, npm, cargo, go, or
# nothing when none of them is there.
shimmer_mono_tool() {
    dir=$1
    if [ -f "$dir/turbo.json" ]; then
        echo turbo
    elif [ -f "$dir/nx.json" ]; then
        echo nx
    elif [ -f "$dir/pnpm-workspace.yaml" ]; then
        echo pnpm
    elif [ -f "$dir/package.json" ] && grep -q '"workspaces"[[:space:]]*:' "$dir/package.json"; then
        pm=$(shimmer_package_manager "$dir")
        echo "${pm:-npm}"
    elif [ -f "$dir/Cargo.toml" ] && grep -q '^\[workspace\]' "$dir/Cargo.toml"; then
        echo cargo
    elif [ -f "$dir/go.work" ]; then
        echo go
    fi
}

# The dev task's name: DEV_TARGET, or with `auto`, `serve` for Nx (its convention) and `dev`
# everywhere else.
shimmer_mono_target() {
    tool=$1
    [ "${DEV_TARGET:-auto}" = "auto" ] || {
        echo "$DEV_TARGET"
        return
    }
    if [ "$tool" = "nx" ]; then echo serve; else echo dev; fi
}

# A JavaScript tool's command runner for a binary installed in the repo (turbo, nx).
shimmer_mono_exec() {
    case "$(shimmer_package_manager "$1")" in
        pnpm) echo "pnpm exec" ;;
        yarn) echo "yarn" ;;
        bun) echo "bunx" ;;
        *) echo "npx --no-install" ;;
    esac
}

# The command line(s) that run APPS, one per line as "label<TAB>command". Turborepo and Nx run
# every app in one process (their own labelled output); the others get one process per app.
# DEV_COMMAND replaces `auto`: with `{app}` in it, it runs once per app; without, once.
shimmer_mono_commands() {
    dir=$1
    tool=$(shimmer_mono_tool "$dir")
    target=$(shimmer_mono_target "$tool")
    tab=$(printf '\t')
    if [ "${DEV_COMMAND:-auto}" != "auto" ]; then
        case "$DEV_COMMAND" in
            *"{app}"*)
                for app in $APPS; do
                    printf '%s%s%s\n' "$(shimmer_mono_label "$app")" "$tab" "$(printf '%s' "$DEV_COMMAND" | sed "s|{app}|$app|g")"
                done
                ;;
            *) printf 'all%s%s\n' "$tab" "$DEV_COMMAND" ;;
        esac
        return
    fi
    case "$tool" in
        turbo)
            filters=""
            for app in $APPS; do filters="$filters --filter=$app"; done
            printf 'all%s%s turbo run %s%s\n' "$tab" "$(shimmer_mono_exec "$dir")" "$target" "$filters"
            ;;
        nx)
            projects=$(printf '%s' "$APPS" | tr -s ' ' ',' | sed 's/^,//; s/,$//')
            printf 'all%s%s nx run-many -t %s -p %s\n' "$tab" "$(shimmer_mono_exec "$dir")" "$target" "$projects"
            ;;
        *)
            for app in $APPS; do
                case "$tool" in
                    pnpm) cmd="pnpm --filter $app run $target" ;;
                    yarn) cmd="yarn workspace $app run $target" ;;
                    bun) cmd="bun run --filter $app $target" ;;
                    npm) cmd="npm run $target -w $app" ;;
                    cargo) cmd="cargo run -p $app" ;;
                    go) cmd="go run ./$app" ;;
                    *) return 0 ;;
                esac
                printf '%s%s%s\n' "$(shimmer_mono_label "$app")" "$tab" "$cmd"
            done
            ;;
    esac
}

# An app name or folder as a log-file label: apps/web → apps-web.
shimmer_mono_label() {
    printf '%s' "$1" | tr '/ ' '--'
}

# The program a tool needs, for the check step.
shimmer_mono_program() {
    case "$1" in
        turbo | nx)
            pm=$(shimmer_package_manager "$2")
            echo "${pm:-npm}"
            ;;
        *) echo "$1" ;;
    esac
}
