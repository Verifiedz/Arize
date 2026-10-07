# Supervised: start the VM, unless it's already running.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-vm.sh"

# vm.started: this workspace started the VM, so stop may suspend or shut it down. A VM you
# started yourself is left as it is.
started="$SHIMMER_STATE_DIR/vm.started"
mkdir -p "$SHIMMER_STATE_DIR"
if [ "$VM_SOFTWARE" = "custom" ]; then
    # Nothing can ask a custom VM whether it runs: it does if this workspace started it and
    # (when there's an address) it still answers.
    if [ -f "$started" ]; then
        host=$(shimmer_vm_host)
        if [ -z "$host" ] || shimmer_port_open "$host" "${SSH_PORT:-22}"; then
            echo "$VM_NAME is already running (started by the last activate)"
            exit 0
        fi
    fi
elif shimmer_vm_running; then
    if [ -f "$started" ]; then
        echo "$VM_NAME is already running (started by the last activate)"
    else
        echo "$VM_NAME was already running: stop will leave it running"
    fi
    exit 0
fi
shimmer_vm_start || shimmer_fail "$VM_SOFTWARE couldn't start '$VM_NAME' (its message is above)"
touch "$started"
echo "started $VM_NAME"
