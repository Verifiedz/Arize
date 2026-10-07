# Supervised: stop with one clear sentence before downloading anything, and start a fresh report.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-offline.sh"

for name in $SKIP; do
    case " $SHIMMER_OFFLINE_PARTS " in
        *" $name "*) ;;
        *) shimmer_fail "SKIP: there's no part called '$name' (there are: $SHIMMER_OFFLINE_PARTS)" ;;
    esac
done
for pair in "WARM_BUILD:no yes" "DATA_SAVER:no yes" "PULL_ON_RETURN:no yes" "PUSH_ON_RETURN:no yes" \
    "GITHUB_SNAPSHOT:projects everything no"; do
    name=${pair%%:*}
    eval "value=\${$name}"
    [ -z "$value" ] && continue
    case " ${pair#*:} " in *" $value "*) ;; *) shimmer_fail "$name must be one of: ${pair#*:} (not '$value')" ;; esac
done
[ -n "$PROJECTS$CLONE_REPOS" ] || shimmer_fail "set PROJECTS: the project folders to get ready"
[ -z "$CLONE_REPOS" ] || shimmer_has git || shimmer_fail "CLONE_REPOS needs git, which isn't installed"

mkdir -p "$SHIMMER_STATE_DIR"
rm -f "$SHIMMER_RESULTS"
missing=""
while IFS= read -r p; do
    [ -n "$p" ] || continue
    case "$p" in /*) ;; *) shimmer_fail "'$p' isn't a full path: use ~/… or /…" ;; esac
    if [ -d "$p" ]; then
        echo "project: $p ($(shimmer_offline_kinds "$p" | sed 's/^$/nothing recognised/'))"
    else
        # A repo still to be cloned is fine; any other missing folder is a typo.
        cloned=no
        for repo in $CLONE_REPOS; do
            [ "$(basename "$p")" = "$(shimmer_offline_repo_name "$repo")" ] && cloned=yes
        done
        [ "$cloned" = yes ] && echo "project: $p (to be cloned)" || missing="$missing $p"
    fi
done <<PROJECTS
$(shimmer_offline_projects)
PROJECTS
[ -z "$missing" ] || shimmer_fail "these project folders don't exist:$missing"
mkdir -p "$(shimmer_offline_dir)" || shimmer_fail "can't make $(shimmer_offline_dir)"
echo "saving to: $(shimmer_offline_dir)"
