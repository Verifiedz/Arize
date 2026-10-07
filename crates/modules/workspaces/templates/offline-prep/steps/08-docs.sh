# Supervised: documentation that works offline. Each Rust project's `cargo doc` (the docs of the
# exact versions it uses), rustup's own offline docs (the book, std), and tldr's pages. Python's
# and Go's docs already work offline once their packages are downloaded: `python3 -m pydoc -b`,
# `go doc <package>`.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

shimmer_offline_skipped docs && exit 0
rust=no
while IFS= read -r p; do
    [ -n "$p" ] || continue
    case " $(shimmer_offline_kinds "$p") " in *" rust "*) ;; *) continue ;; esac
    rust=yes
    shimmer_has cargo || continue
    name=$(basename "$p")
    if shimmer_offline_run "$p" "cargo doc --offline --quiet"; then
        shimmer_offline_result ok docs "$name: Rust docs for every dependency (linked from the start page)"
    else
        shimmer_offline_result warn docs "$name: cargo doc failed (see the docs step's log)"
    fi
done <<PROJECTS
$(shimmer_offline_existing)
PROJECTS
if [ "$rust" = yes ] && shimmer_has rustup; then
    if rustup component add rust-docs </dev/null >/dev/null 2>&1; then
        shimmer_offline_result ok docs "Rust's own docs (the book, std): rustup doc"
    fi
fi
if shimmer_has tldr; then
    if tldr --update </dev/null >/dev/null 2>&1; then
        shimmer_offline_result ok docs "tldr pages updated: tldr <command> works offline"
    fi
fi
exit 0
