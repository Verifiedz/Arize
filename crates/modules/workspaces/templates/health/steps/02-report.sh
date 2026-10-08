# Supervised: run every check that isn't in SKIP. Read-only. A check that fails or doesn't
# apply here (no battery, no systemd) adds nothing and never stops the others.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-health.sh"

mkdir -p "$SHIMMER_STATE_DIR"
for name in $SHIMMER_HEALTH_CHECKS; do
    case " $SKIP " in *" $name "*) continue ;; esac
    echo "== $name"
    out=$("health_$name" 2>&1)
    printf '%s\n' "$out"
    printf '%s\n' "$out" | grep -E '^(ok|warn|info)	' >>"$SHIMMER_RESULTS"
done
exit 0
