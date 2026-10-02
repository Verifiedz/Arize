#!/usr/bin/env python3
"""Enforce the dependency rules of CLAUDE.md §3.

    core      -> (nothing internal)
    proto     -> core
    store     -> core
    modules/* -> core                  NOT store, NOT daemon, NOT each other
    daemon    -> core, proto, store, modules/*
    cli, tui  -> core, proto           NOT store, NOT daemon, NOT modules
    mockd     -> core, proto           a server built like a client
    app       -> everything

and, from the same section, `app` is the only crate that declares a binary target.

Reads `cargo metadata --no-deps` (no network needed), so it sees exactly what the manifests
declare. Every dependency kind is checked, dev-dependencies included: a module that reaches
another module only from its tests is still coupled to it.

A crate under `crates/` that has no role here fails the check. That is deliberate: adding a
crate means deciding, in this file and in CLAUDE.md, what it may depend on.

    python3 .dev/check-deps.py              check the workspace
    python3 .dev/check-deps.py --self-test  test the checker itself
"""

import json
import subprocess
import sys
from pathlib import PurePosixPath

ROLES = ("core", "proto", "store", "module", "daemon", "cli", "tui", "mockd", "app")

# role -> roles it may depend on
ALLOWED = {
    "core": set(),
    "proto": {"core"},
    "store": {"core"},
    "module": {"core"},
    "daemon": {"core", "proto", "store", "module"},
    "cli": {"core", "proto"},
    "tui": {"core", "proto"},
    "mockd": {"core", "proto"},
    "app": set(ROLES),
}

# Why, for the failure message. Keyed by (from, to); falls back to the generic text.
WHY = {
    ("module", "module"): "no module may depend on another module; react to events instead (§3 rule 1, §12 rule 2)",
    ("module", "store"): "modules reach storage only through Ctx (§3, §12 rule 4)",
    ("module", "daemon"): "modules must not depend on the daemon (§3)",
    ("module", "proto"): "modules depend on core only (§3)",
    ("cli", "store"): "no client may depend on store, daemon or modules (§3 rule 2, §12 rule 3)",
    ("cli", "daemon"): "no client may depend on store, daemon or modules (§3 rule 2, §12 rule 3)",
    ("cli", "module"): "no client may depend on store, daemon or modules (§3 rule 2, §12 rule 3)",
    ("tui", "store"): "no client may depend on store, daemon or modules (§3 rule 2, §12 rule 3)",
    ("tui", "daemon"): "no client may depend on store, daemon or modules (§3 rule 2, §12 rule 3)",
    ("tui", "module"): "no client may depend on store, daemon or modules (§3 rule 2, §12 rule 3)",
    ("core", "proto"): "core depends on nothing internal (§3)",
    ("core", "store"): "core depends on nothing internal (§3)",
    ("core", "daemon"): "core depends on nothing internal (§3)",
    ("proto", "store"): "proto depends on core only (§3)",
    ("proto", "daemon"): "proto depends on core only (§3)",
    ("mockd", "store"): "mockd is built like a client: core and proto only, no store, daemon or modules (§3)",
    ("mockd", "daemon"): "mockd is built like a client: core and proto only, no store, daemon or modules (§3)",
    ("mockd", "module"): "mockd is built like a client: core and proto only, no store, daemon or modules (§3)",
}


def role_of(manifest_path, root):
    """Map a crate's Cargo.toml to its role, or None if it has no place in §3."""
    rel = PurePosixPath(manifest_path).relative_to(root).parent.parts
    if len(rel) == 2 and rel[0] == "crates" and rel[1] in ROLES and rel[1] != "module":
        return rel[1]
    if len(rel) == 3 and rel[:2] == ("crates", "modules"):
        return "module"
    return None


def check(packages, root):
    """Return a list of human-readable violations for `cargo metadata` package records."""
    roles, errors = {}, []
    for p in packages:
        role = role_of(p["manifest_path"], root)
        if role is None:
            errors.append(
                f"{p['name']} ({p['manifest_path']}): no role in CLAUDE.md §3. "
                "Add one to ROLES/ALLOWED in .dev/check-deps.py and to CLAUDE.md before adding the crate."
            )
        else:
            roles[p["name"]] = role
    for p in packages:
        if roles.get(p["name"]) != "app" and any("bin" in t.get("kind", []) for t in p.get("targets", [])):
            errors.append(
                f"{p['name']}: declares a binary target, but `app` is the only [[bin]] (§3). "
                "Expose it as a subcommand of `shimmer` instead."
            )
    for p in packages:
        src = roles.get(p["name"])
        if src is None:
            continue
        for dep in p["dependencies"]:
            dst = roles.get(dep["name"])
            if dst is None or dst in ALLOWED[src]:
                continue
            kind = dep.get("kind") or "normal"
            why = WHY.get((src, dst), f"{src} may depend on: {', '.join(sorted(ALLOWED[src])) or 'nothing internal'} (§3)")
            errors.append(f"{p['name']} [{src}] -> {dep['name']} [{dst}] ({kind}): {why}")
    return errors


def workspace_metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"], check=True, capture_output=True, text=True
    ).stdout
    meta = json.loads(out)
    return meta["packages"], meta["workspace_root"]


# ------------------------------------------------------------------ self-test


def _pkg(root, path, name, deps=(), kind=None, bin=False):
    return {
        "name": name,
        "manifest_path": f"{root}/{path}/Cargo.toml",
        "dependencies": [{"name": d, "kind": kind} for d in deps],
        "targets": [{"kind": ["bin"] if bin else ["lib"]}],
    }


def self_test():
    root = "/ws"
    base = [
        _pkg(root, "crates/core", "shimmer-core"),
        _pkg(root, "crates/proto", "shimmer-proto", ["shimmer-core"]),
        _pkg(root, "crates/store", "shimmer-store", ["shimmer-core"]),
        _pkg(root, "crates/modules/records", "shimmer-records", ["shimmer-core"]),
        _pkg(root, "crates/modules/notify", "shimmer-notify", ["shimmer-core"]),
        _pkg(root, "crates/daemon", "shimmer-daemon", ["shimmer-core", "shimmer-proto", "shimmer-store", "shimmer-records"]),
        # Third-party crates are invisible to the check, whatever they are called.
        _pkg(root, "crates/cli", "shimmer-cli", ["shimmer-core", "shimmer-proto", "serde"]),
        _pkg(root, "crates/tui", "shimmer-tui", ["shimmer-core", "shimmer-proto"]),
        _pkg(root, "crates/mockd", "shimmer-mockd", ["shimmer-core", "shimmer-proto"]),
        _pkg(root, "crates/app", "shimmer", ["shimmer-daemon", "shimmer-cli", "shimmer-tui", "shimmer-mockd", "shimmer-store"], bin=True),
    ]
    assert check(base, root) == [], check(base, root)

    def with_extra(name, extra, kind=None):
        pkgs = [dict(p) for p in base]
        for p in pkgs:
            if p["name"] == name:
                p["dependencies"] = p["dependencies"] + [{"name": d, "kind": kind} for d in extra]
        return check(pkgs, root)

    cases = [
        ("core -> proto", with_extra("shimmer-core", ["shimmer-proto"]), "shimmer-core [core] -> shimmer-proto [proto]"),
        ("proto -> store", with_extra("shimmer-proto", ["shimmer-store"]), "shimmer-proto [proto] -> shimmer-store [store]"),
        ("module -> module", with_extra("shimmer-records", ["shimmer-notify"]), "shimmer-records [module] -> shimmer-notify [module]"),
        ("module -> store", with_extra("shimmer-records", ["shimmer-store"]), "shimmer-records [module] -> shimmer-store [store]"),
        ("module -> daemon", with_extra("shimmer-notify", ["shimmer-daemon"]), "shimmer-notify [module] -> shimmer-daemon [daemon]"),
        ("cli -> store", with_extra("shimmer-cli", ["shimmer-store"]), "shimmer-cli [cli] -> shimmer-store [store]"),
        ("cli -> module", with_extra("shimmer-cli", ["shimmer-records"]), "shimmer-cli [cli] -> shimmer-records [module]"),
        ("tui -> daemon", with_extra("shimmer-tui", ["shimmer-daemon"]), "shimmer-tui [tui] -> shimmer-daemon [daemon]"),
        ("store -> daemon", with_extra("shimmer-store", ["shimmer-daemon"]), "shimmer-store [store] -> shimmer-daemon [daemon]"),
        ("daemon -> cli", with_extra("shimmer-daemon", ["shimmer-cli"]), "shimmer-daemon [daemon] -> shimmer-cli [cli]"),
        ("mockd -> store", with_extra("shimmer-mockd", ["shimmer-store"]), "shimmer-mockd [mockd] -> shimmer-store [store]"),
        ("mockd -> daemon", with_extra("shimmer-mockd", ["shimmer-daemon"]), "shimmer-mockd [mockd] -> shimmer-daemon [daemon]"),
        ("mockd -> module", with_extra("shimmer-mockd", ["shimmer-records"]), "shimmer-mockd [mockd] -> shimmer-records [module]"),
        ("daemon -> mockd", with_extra("shimmer-daemon", ["shimmer-mockd"]), "shimmer-daemon [daemon] -> shimmer-mockd [mockd]"),
        ("dev-dependency counts", with_extra("shimmer-records", ["shimmer-notify"], "dev"), "(dev)"),
    ]
    for name, errors, needle in cases:
        assert len(errors) == 1 and needle in errors[0], f"{name}: {errors}"

    unknown = check(base + [_pkg(root, "crates/newthing", "shimmer-newthing", ["shimmer-core"])], root)
    assert len(unknown) == 1 and "no role in CLAUDE.md" in unknown[0], unknown

    # `app` is the only [[bin]]: a binary anywhere else fails, `app` may have several.
    for path, name in [("crates/mockd", "shimmer-mockd"), ("crates/tui", "shimmer-tui"), ("crates/modules/records", "shimmer-records")]:
        pkgs = [dict(p, targets=[{"kind": ["lib"]}, {"kind": ["bin"]}]) if p["name"] == name else p for p in base]
        errors = check(pkgs, root)
        assert len(errors) == 1 and f"{name}: declares a binary target" in errors[0], (name, errors)
    two_bins = [dict(p, targets=[{"kind": ["bin"]}, {"kind": ["bin"]}]) if p["name"] == "shimmer" else p for p in base]
    assert check(two_bins, root) == []
    assert check([dict(p, targets=[{"kind": ["example"]}, {"kind": ["test"]}]) if p["name"] == "shimmer-mockd" else p for p in base], root) == []
    stray = check(base + [_pkg(root, "tools/thing", "thing")], root)
    assert len(stray) == 1 and "no role" in stray[0], stray
    print("check-deps self-test: ok")


def main():
    if "--self-test" in sys.argv[1:]:
        self_test()
        return 0
    packages, root = workspace_metadata()
    errors = check(packages, root)
    if errors:
        print("Dependency rules (CLAUDE.md §3) violated:\n", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        print(f"\n{len(errors)} violation(s).", file=sys.stderr)
        return 1
    print(f"dependency rules: ok ({len(packages)} crates)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
