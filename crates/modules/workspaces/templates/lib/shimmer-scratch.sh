# Helpers for the scratch template (ADR 0025): each language's starter files and run command,
# and where the current scratch folder is. Steps load it after shimmer-open.sh:
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"
#
# POSIX sh only. This is your copy: Shimmer never changes it after the workspace is created, so
# change a starter file here if you like.

# SCRATCH_DIR with a leading ~ expanded (the default is ~/scratch).
shimmer_scratch_root() {
    case "${SCRATCH_DIR:-~/scratch}" in
        "~") echo "$HOME" ;;
        "~/"*) echo "$HOME/${SCRATCH_DIR#"~/"}" ;;
        "") echo "$HOME/scratch" ;;
        *) echo "$SCRATCH_DIR" ;;
    esac
}

# The folder this run made, written by the folder step for the steps after it.
shimmer_scratch_current() {
    cat "$SHIMMER_STATE_DIR/current" 2>/dev/null
}

# The file to open first, relative to the folder.
shimmer_scratch_main() {
    case "$1" in
        python) echo "main.py" ;;
        rust) echo "src/main.rs" ;;
        javascript) echo "main.js" ;;
        typescript) echo "main.ts" ;;
        go) echo "main.go" ;;
        c) echo "main.c" ;;
        cpp) echo "main.cpp" ;;
        java) echo "Main.java" ;;
        shell) echo "main.sh" ;;
    esac
}

# The command that runs it, from inside the folder.
shimmer_scratch_run() {
    case "$1" in
        python) echo "python3 main.py" ;;
        rust) echo "cargo run" ;;
        javascript) echo "node main.js" ;;
        typescript)
            if shimmer_has bun; then
                echo "bun main.ts"
            elif shimmer_has deno; then
                echo "deno run main.ts"
            else
                echo "npx tsx main.ts"
            fi
            ;;
        go) echo "go run ." ;;
        c) echo "cc main.c -o main && ./main" ;;
        cpp) echo "c++ -std=c++20 main.cpp -o main && ./main" ;;
        java) echo "java Main.java" ;;
        shell) echo "sh main.sh" ;;
    esac
}

# The program the run command needs, for the check step (a warning, not a failure: you may
# only want to write code here).
shimmer_scratch_program() {
    case "$1" in
        python) echo "python3" ;;
        rust) echo "cargo" ;;
        javascript) echo "node" ;;
        typescript) shimmer_has bun || shimmer_has deno || echo "npx" ;;
        go) echo "go" ;;
        c) echo "cc" ;;
        cpp) echo "c++" ;;
        java) echo "java" ;;
    esac
}

# Write the starter files into folder $2. Never over a file that's already there.
shimmer_scratch_starter() {
    lang=$1
    dir=$2
    main=$(shimmer_scratch_main "$lang")
    [ -n "$main" ] || return 0
    [ -e "$dir/$main" ] && return 0
    mkdir -p "$(dirname "$dir/$main")"
    case "$lang" in
        python)
            printf 'def main():\n    print("hello from scratch")\n\n\nif __name__ == "__main__":\n    main()\n' >"$dir/$main"
            ;;
        rust)
            printf '[package]\nname = "scratch"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n' >"$dir/Cargo.toml"
            printf 'fn main() {\n    println!("hello from scratch");\n}\n' >"$dir/$main"
            printf '/target\n' >"$dir/.gitignore"
            ;;
        javascript)
            printf 'console.log("hello from scratch");\n' >"$dir/$main"
            ;;
        typescript)
            printf 'const greeting: string = "hello from scratch";\nconsole.log(greeting);\n' >"$dir/$main"
            ;;
        go)
            printf 'module scratch\n\ngo 1.21\n' >"$dir/go.mod"
            printf 'package main\n\nimport "fmt"\n\nfunc main() {\n\tfmt.Println("hello from scratch")\n}\n' >"$dir/$main"
            ;;
        c)
            printf '#include <stdio.h>\n\nint main(void) {\n    printf("hello from scratch\\n");\n    return 0;\n}\n' >"$dir/$main"
            printf 'main\n' >"$dir/.gitignore"
            ;;
        cpp)
            printf '#include <iostream>\n\nint main() {\n    std::cout << "hello from scratch\\n";\n    return 0;\n}\n' >"$dir/$main"
            printf 'main\n' >"$dir/.gitignore"
            ;;
        java)
            printf 'public class Main {\n    public static void main(String[] args) {\n        System.out.println("hello from scratch");\n    }\n}\n' >"$dir/$main"
            ;;
        shell)
            printf '#!/bin/sh\nset -eu\n\necho "hello from scratch"\n' >"$dir/$main"
            ;;
    esac
}

# Move a folder to the trash, never delete it. Prints nothing and fails when there's no trash
# command here.
shimmer_trash() {
    if shimmer_has gio; then
        gio trash "$1"
    elif shimmer_has trash-put; then
        trash-put "$1"
    elif shimmer_has trash; then
        trash "$1"
    else
        return 1
    fi
}
