#!/usr/bin/env python3
"""Fail if an interface crate can name the registry.

# What this guards, and why a type could not

Invariant 7 and [ADR 0022](../docs/adr/0022-decoupage-daemon-gui.md) say the interface
observes and does not drive: it receives a `Receiver<Observation>`, never a
`RegistryHandle`.

That was enforced by the *shape of `App`'s fields* — App held no handle — and ADR 0022
described it as being "in the typing". It was not. `trame-view` depended on
`trame-registry` and called `admit` six times, and `apps/trame-tui` carried the
dependency too. Nothing stopped either from admitting a write; they merely did not.

An enum with no `Admit` variant stops nobody who also holds a `RegistryHandle`. The
enforcement has to be the **crate graph**: a crate that does not depend on
`trame-registry` cannot name `admit`, whatever it wants.

# Why a script rather than a test

A test would live inside one of the crates it checks, and would therefore need the
dependency it exists to forbid. The property is about the dependency graph, so it is
checked from outside it — the same reason `check-features` is a script.

# Dev-dependencies are allowed, deliberately

`apps/trame-tui` keeps `trame-registry` as a **dev-dependency**: the measurement harness
in `examples/notice_experiment.rs` drives the registry on purpose, which is what an
experiment is for. Under cargo's resolver that does not put the handle in the shipped
binary.

So the rule is precise: **no interface crate may have `trame-registry` as a normal
dependency.** Dev is fine, and the distinction is the whole point — the experiment can
reach admission, the product's interface cannot.
"""

from __future__ import annotations

import pathlib
import re
import sys

# The crates that must not be able to name the registry.
INTERFACE_CRATES = [
    "crates/trame-view",
    "apps/trame-tui",
    "apps/trame-gui",
]

FORBIDDEN = "trame-registry"


def normal_dependencies(manifest: str) -> set[str]:
    """Crate names under [dependencies], ignoring dev and build sections.

    Deliberately narrow: this reads the section a crate declares, not the resolved graph.
    A transitive path to trame-registry through trame-daemon is expected and fine — the
    daemon is what owns admission. What must not exist is a DIRECT edge, because that is
    what lets an interface write `registry.admit(...)`.
    """
    names: set[str] = set()
    section = None
    for line in manifest.split("\n"):
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped.strip("[]")
            continue
        if section != "dependencies" or not stripped or stripped.startswith("#"):
            continue
        match = re.match(r"^([A-Za-z0-9_-]+)\s*(=|\.)", stripped)
        if match:
            names.add(match.group(1))
    return names


def self_test() -> bool:
    """★ The negative control, run before every report.

    Two manifests that differ by exactly one line. If the detector cannot tell them
    apart, it is not checking anything — and this file would happily report GREEN on a
    repository where the door had been reopened.
    """
    breached = """
[package]
name = "trame-view"

[dependencies]
trame-core.workspace = true
trame-daemon.workspace = true
trame-registry.workspace = true

[dev-dependencies]
chrono.workspace = true
"""
    clean = """
[package]
name = "trame-view"

[dependencies]
trame-core.workspace = true
trame-daemon.workspace = true

[dev-dependencies]
trame-registry.workspace = true
chrono.workspace = true
"""
    ok = True
    if FORBIDDEN not in normal_dependencies(breached):
        print("  SELF-TEST FAILED: a direct dependency was not detected")
        ok = False
    if FORBIDDEN in normal_dependencies(clean):
        print("  SELF-TEST FAILED: a dev-dependency was reported as a normal one")
        ok = False
    return ok


def main() -> int:
    if "--self-test" in sys.argv:
        if self_test():
            print("SELF-TEST: GREEN — the detector separates a normal dep from a dev dep")
            return 0
        print("SELF-TEST: RED — the detector proves nothing, fix it first")
        return 1

    if not self_test():
        print("SELF-TEST: RED — refusing to report with a detector that proves nothing")
        return 1

    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    breaches = []
    for crate in INTERFACE_CRATES:
        manifest = root / crate / "Cargo.toml"
        if not manifest.exists():
            print(f"BOUNDARY: RED — {crate}/Cargo.toml is missing; the check cannot run")
            return 1
        if FORBIDDEN in normal_dependencies(manifest.read_text()):
            breaches.append(crate)

    if not breaches:
        print(
            f"BOUNDARY: GREEN — none of the {len(INTERFACE_CRATES)} interface crates can name "
            f"{FORBIDDEN}"
        )
        return 0

    print(f"BOUNDARY: RED — {len(breaches)} interface crate(s) can reach admission:\n")
    for crate in breaches:
        print(f"  {crate} depends on {FORBIDDEN}")
    print(
        f"\nAn interface observes, it does not drive (invariant 7, ADR 0022). A crate that\n"
        f"can name {FORBIDDEN} can call admit, and a write that did not come from an agent\n"
        f"has no provenance — a journal row without provenance is worse than no row.\n"
        f"\n"
        f"If this crate genuinely needs the registry, it is not an interface crate. If a\n"
        f"test or an example needs it, move it to [dev-dependencies]: the experiment may\n"
        f"reach admission, the shipped interface may not.\n"
        f"\n"
        f"To ask the interface for something, add a variant to trame_daemon::Command."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
