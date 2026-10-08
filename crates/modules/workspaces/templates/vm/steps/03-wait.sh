# Supervised: wait until the VM answers on its SSH port, then say whether an SSH key works.
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
. "$SHIMMER_WORKSPACE_DIR/lib/shimmer-vm.sh"

[ -n "$VM_HOST" ] || exit 0
port=${SSH_PORT:-22}
i=0
while [ $i -lt 280 ]; do
    host=$(shimmer_vm_host)
    if [ -n "$host" ] && shimmer_port_open "$host" "$port"; then
        echo "$host:$port is up"
        target=$(shimmer_ssh_target "$host")
        if ssh -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=accept-new -p "$port" "$target" true >/dev/null 2>&1; then
            echo "an SSH key works: nothing will ask for a password"
        elif [ -z "$SSH_PASSWORD_ITEM" ]; then
            echo "no SSH key set up yet: ssh will ask for your password. To stop that, run once:"
            echo "  ssh-copy-id -p $port $target"
        fi
        exit 0
    fi
    sleep 2
    i=$((i + 2))
done
shimmer_fail "${host:-the VM} didn't answer on port $port within 280 seconds (is SSH running inside the VM?)"
