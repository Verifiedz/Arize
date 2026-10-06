# Cleanup (supervised): suspend or shut down the VM, as ON_STOP says. Run by `workspaces stop`,
# and by `workspaces cleanup` after a failed launch. Closes the SSH terminal it opened when it
# can (CLOSE_ON_STOP), and lists what it closed and what it left open.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-vm.sh"

# Windows first: an SSH terminal into a VM that's about to be suspended is no use.
shimmer_close_windows
rm -f "$SHIMMER_STATE_DIR/askpass"
if [ "$ON_STOP" = "leave-running" ]; then
    echo "leaving $VM_NAME running"
    exit 0
fi
if [ "$VM_SOFTWARE" != "custom" ] && ! shimmer_vm_running; then
    echo "$VM_NAME isn't running"
    exit 0
fi
shimmer_vm_stop || shimmer_fail "$VM_SOFTWARE couldn't $ON_STOP '$VM_NAME' (its message is above)"
echo "$ON_STOP: $VM_NAME"
