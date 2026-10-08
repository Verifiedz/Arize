# Supervised: stop with one clear sentence if a threshold isn't a number, and start a fresh report.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-health.sh"

for name in DISK_WARN_PERCENT MEMORY_WARN_PERCENT BATTERY_WARN_PERCENT UPTIME_WARN_DAYS BIGGEST_FOLDERS; do
    eval "value=\${$name}"
    case "$value" in
        "" | *[!0-9]*) [ -z "$value" ] || shimmer_fail "$name must be a whole number, not '$value'" ;;
    esac
done
for name in $SKIP; do
    case " $SHIMMER_HEALTH_CHECKS " in
        *" $name "*) ;;
        *) shimmer_fail "SKIP: there's no check called '$name' (there are: $SHIMMER_HEALTH_CHECKS)" ;;
    esac
done
case "${NOTIFY:-never}" in never | on-warning) ;; *) shimmer_fail "NOTIFY must be never or on-warning" ;; esac
mkdir -p "$SHIMMER_STATE_DIR"
rm -f "$SHIMMER_RESULTS"
echo "checks: $SHIMMER_HEALTH_CHECKS${SKIP:+ (leaving out: $SKIP)}"
