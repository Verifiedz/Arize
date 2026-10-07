# Helpers for the scratch template (ADR 0025): each language's starter file and run command,
# and where the current scratch folder is. Steps load it after shimmer-open.sh:
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-open.sh"
#   . "$SHIMMER_WORKSPACE_DIR/lib/shimmer-scratch.sh"
#
# LANGUAGE is a language this file knows (below), or for any other language a file name such as
# main.odin: that file is made (empty) and opened, and RUN_COMMAND says how to run it.
#
# POSIX sh only. This is your copy: Shimmer never changes it after the workspace is created, so
# change a starter or add a language here if you like.

# Every language this file knows, for messages.
SHIMMER_SCRATCH_LANGUAGES="python rust javascript typescript go c cpp java kotlin swift csharp ruby php lua perl r julia haskell ocaml elixir clojure dart scala zig nim crystal fortran sql html shell"

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

# LANGUAGE as one of the names above (common other spellings accepted), `file` for a file name,
# or nothing when it's neither.
shimmer_scratch_lang() {
    lang=$(printf '%s' "${1:-python}" | tr '[:upper:]' '[:lower:]')
    case "$lang" in
        */*) return 0 ;;
        *.*) echo file ;;
        python | py | python3) echo python ;;
        rust | rs) echo rust ;;
        javascript | js | node) echo javascript ;;
        typescript | ts) echo typescript ;;
        go | golang) echo go ;;
        c) echo c ;;
        cpp | c++ | cxx) echo cpp ;;
        java) echo java ;;
        kotlin | kt) echo kotlin ;;
        swift) echo swift ;;
        csharp | c# | cs | dotnet) echo csharp ;;
        ruby | rb) echo ruby ;;
        php) echo php ;;
        lua) echo lua ;;
        perl | pl) echo perl ;;
        r) echo r ;;
        julia | jl) echo julia ;;
        haskell | hs) echo haskell ;;
        ocaml | ml) echo ocaml ;;
        elixir | ex | exs) echo elixir ;;
        clojure | clj) echo clojure ;;
        dart) echo dart ;;
        scala) echo scala ;;
        zig) echo zig ;;
        nim) echo nim ;;
        crystal | cr) echo crystal ;;
        fortran | f90) echo fortran ;;
        sql | sqlite) echo sql ;;
        html | web) echo html ;;
        shell | sh | bash) echo shell ;;
        none) echo none ;;
    esac
}

# For the folder's name: the language, or a file name's extension (main.odin → odin).
shimmer_scratch_label() {
    lang=$(shimmer_scratch_lang "$1")
    if [ "$lang" = "file" ]; then
        printf '%s' "${1##*.}" | tr -c 'A-Za-z0-9_-' '-'
    else
        echo "$lang"
    fi
}

# The file to open first, relative to the folder.
shimmer_scratch_main() {
    case "$(shimmer_scratch_lang "$1")" in
        file) echo "$1" ;;
        python) echo main.py ;;
        rust) echo src/main.rs ;;
        javascript) echo main.js ;;
        typescript) echo main.ts ;;
        go) echo main.go ;;
        c) echo main.c ;;
        cpp) echo main.cpp ;;
        java) echo Main.java ;;
        kotlin) echo main.kts ;;
        swift) echo main.swift ;;
        csharp) echo main.cs ;;
        ruby) echo main.rb ;;
        php) echo main.php ;;
        lua) echo main.lua ;;
        perl) echo main.pl ;;
        r) echo main.R ;;
        julia) echo main.jl ;;
        haskell) echo main.hs ;;
        ocaml) echo main.ml ;;
        elixir) echo main.exs ;;
        clojure) echo main.clj ;;
        dart) echo main.dart ;;
        scala) echo main.scala ;;
        zig) echo main.zig ;;
        nim) echo main.nim ;;
        crystal) echo main.cr ;;
        fortran) echo main.f90 ;;
        sql) echo main.sql ;;
        html) echo index.html ;;
        shell) echo main.sh ;;
    esac
}

# The command that runs it, from inside the folder: RUN_COMMAND when set.
shimmer_scratch_run() {
    if [ -n "$RUN_COMMAND" ]; then
        echo "$RUN_COMMAND"
        return
    fi
    case "$(shimmer_scratch_lang "$1")" in
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
        kotlin) echo "kotlinc -script main.kts" ;;
        swift) echo "swift main.swift" ;;
        csharp) echo "dotnet run main.cs" ;;
        ruby) echo "ruby main.rb" ;;
        php) echo "php main.php" ;;
        lua) echo "lua main.lua" ;;
        perl) echo "perl main.pl" ;;
        r) echo "Rscript main.R" ;;
        julia) echo "julia main.jl" ;;
        haskell) echo "runghc main.hs" ;;
        ocaml) echo "ocaml main.ml" ;;
        elixir) echo "elixir main.exs" ;;
        clojure) echo "clojure -M main.clj" ;;
        dart) echo "dart run main.dart" ;;
        scala) echo "scala run main.scala" ;;
        zig) echo "zig run main.zig" ;;
        nim) echo "nim c -r --hints:off main.nim" ;;
        crystal) echo "crystal run main.cr" ;;
        fortran) echo "gfortran main.f90 -o main && ./main" ;;
        sql) echo "sqlite3 < main.sql" ;;
        html) if shimmer_macos; then echo "open index.html"; else echo "xdg-open index.html"; fi ;;
        shell) echo "sh main.sh" ;;
    esac
}

# The program the run command needs, for the check step (a note, not a failure: you may only
# want to write code here). Nothing for your own RUN_COMMAND.
shimmer_scratch_program() {
    [ -n "$RUN_COMMAND" ] && return 0
    case "$(shimmer_scratch_lang "$1")" in
        python) echo python3 ;;
        rust) echo cargo ;;
        javascript) echo node ;;
        typescript) shimmer_has bun || shimmer_has deno || echo npx ;;
        go) echo go ;;
        c) echo cc ;;
        cpp) echo c++ ;;
        java) echo java ;;
        kotlin) echo kotlinc ;;
        swift) echo swift ;;
        csharp) echo dotnet ;;
        ruby) echo ruby ;;
        php) echo php ;;
        lua) echo lua ;;
        perl) echo perl ;;
        r) echo Rscript ;;
        julia) echo julia ;;
        haskell) echo runghc ;;
        ocaml) echo ocaml ;;
        elixir) echo elixir ;;
        clojure) echo clojure ;;
        dart) echo dart ;;
        scala) echo scala ;;
        zig) echo zig ;;
        nim) echo nim ;;
        crystal) echo crystal ;;
        fortran) echo gfortran ;;
        sql) echo sqlite3 ;;
    esac
}

# Write the starter into folder $2. Never over a file that's already there.
shimmer_scratch_starter() {
    lang=$(shimmer_scratch_lang "$1")
    dir=$2
    main=$(shimmer_scratch_main "$1")
    [ -n "$main" ] || return 0
    file="$dir/$main"
    [ -e "$file" ] && return 0
    mkdir -p "$(dirname "$file")"
    case "$lang" in
        file) : >"$file" ;;
        python)
            cat >"$file" <<'STARTER'
def main():
    print("hello from scratch")


if __name__ == "__main__":
    main()
STARTER
            ;;
        rust)
            printf '[package]\nname = "scratch"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n' >"$dir/Cargo.toml"
            printf '/target\n' >"$dir/.gitignore"
            cat >"$file" <<'STARTER'
fn main() {
    println!("hello from scratch");
}
STARTER
            ;;
        javascript) echo 'console.log("hello from scratch");' >"$file" ;;
        typescript)
            cat >"$file" <<'STARTER'
const greeting: string = "hello from scratch";
console.log(greeting);
STARTER
            ;;
        go)
            printf 'module scratch\n\ngo 1.21\n' >"$dir/go.mod"
            cat >"$file" <<'STARTER'
package main

import "fmt"

func main() {
	fmt.Println("hello from scratch")
}
STARTER
            ;;
        c)
            printf 'main\n' >"$dir/.gitignore"
            cat >"$file" <<'STARTER'
#include <stdio.h>

int main(void) {
    printf("hello from scratch\n");
    return 0;
}
STARTER
            ;;
        cpp)
            printf 'main\n' >"$dir/.gitignore"
            cat >"$file" <<'STARTER'
#include <iostream>

int main() {
    std::cout << "hello from scratch\n";
    return 0;
}
STARTER
            ;;
        java)
            cat >"$file" <<'STARTER'
public class Main {
    public static void main(String[] args) {
        System.out.println("hello from scratch");
    }
}
STARTER
            ;;
        kotlin) echo 'println("hello from scratch")' >"$file" ;;
        swift) echo 'print("hello from scratch")' >"$file" ;;
        csharp) echo 'Console.WriteLine("hello from scratch");' >"$file" ;;
        ruby) echo 'puts "hello from scratch"' >"$file" ;;
        php) printf '<?php\n\necho "hello from scratch\\n";\n' >"$file" ;;
        lua) echo 'print("hello from scratch")' >"$file" ;;
        perl) printf 'use strict;\nuse warnings;\n\nprint "hello from scratch\\n";\n' >"$file" ;;
        r) printf 'cat("hello from scratch\\n")\n' >"$file" ;;
        julia) echo 'println("hello from scratch")' >"$file" ;;
        haskell) printf 'main :: IO ()\nmain = putStrLn "hello from scratch"\n' >"$file" ;;
        ocaml) echo 'let () = print_endline "hello from scratch"' >"$file" ;;
        elixir) echo 'IO.puts("hello from scratch")' >"$file" ;;
        clojure) echo '(println "hello from scratch")' >"$file" ;;
        dart)
            cat >"$file" <<'STARTER'
void main() {
  print('hello from scratch');
}
STARTER
            ;;
        scala) echo '@main def hello(): Unit = println("hello from scratch")' >"$file" ;;
        zig)
            cat >"$file" <<'STARTER'
const std = @import("std");

pub fn main() void {
    std.debug.print("hello from scratch\n", .{});
}
STARTER
            ;;
        nim) echo 'echo "hello from scratch"' >"$file" ;;
        crystal) echo 'puts "hello from scratch"' >"$file" ;;
        fortran)
            printf 'main\n' >"$dir/.gitignore"
            printf 'program main\n    print *, "hello from scratch"\nend program main\n' >"$file"
            ;;
        sql) echo "select 'hello from scratch';" >"$file" ;;
        html)
            cat >"$file" <<'STARTER'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Scratch</title>
</head>
<body>
  <h1>hello from scratch</h1>
  <script>
    console.log("hello from scratch");
  </script>
</body>
</html>
STARTER
            ;;
        shell) printf '#!/bin/sh\nset -eu\n\necho "hello from scratch"\n' >"$file" ;;
    esac
}

# Move a folder to the trash, never delete it. Fails when there's no trash command here.
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
