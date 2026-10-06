# Supervised: start the VM, unless it's already running.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-vm.sh"

if shimmer_vm_running; then
    echo "$VM_NAME is already running"
    exit 0
fi
shimmer_vm_start || shimmer_fail "$VM_SOFTWARE couldn't start '$VM_NAME' (its message is above)"
echo "started $VM_NAME"
