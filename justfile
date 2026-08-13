# Trame — development recipes.
# `just` on its own lists the available recipes.

default:
    @just --list

# Compile the whole workspace, tests included, without producing a binary.
check:
    cargo check --workspace --all-targets

# Formatting.
fmt:
    cargo fmt --all

# What CI checks: formatting + clippy with zero warnings + feature tightness.
lint: check-features
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

# Guard rail: trame-core's `test-support` feature exposes `ManualClock`, a clock the
# tests can drive. It must be enabled ONLY through dev-dependencies, never reachable
# from a production build.
#
# Moving one of those three lines from `[dev-dependencies]` to `[dependencies]` would
# leak the test clock into the shipped binary, with no warning at all. Hence this
# check, which fails if a non-dev edge enables the feature.
check-features:
    #!/usr/bin/env sh
    if cargo tree --workspace -e features -i trame-core --edges no-dev 2>/dev/null \
        | grep -q 'test-support'; then
        echo "FAILED: the test-support feature is reachable outside dev-dependencies." >&2
        echo "Look for a 'features = [\"test-support\"]' under [dependencies]." >&2
        exit 1
    fi
    echo "features: test-support confined to dev-dependencies"

# ★ Fail if French has crept back into the code, the docs or the markdown.
#
# The English conversion is worthless the moment someone reintroduces French, and
# nothing else would notice: French comments compile, French assertion messages
# pass, French prose renders. Nobody re-reads a file that works.
#
# The detector runs its own negative control first — a set of known-French lines it
# must flag and known-English lines it must spare. It refuses to report on the
# repository at all if that fails, because a measuring device nobody has seen fail
# has not been verified. That control caught a real hole on its first run: the
# matching was case-sensitive, so a sentence-initial capital walked straight past.
check-language:
    @python3 scripts/no_french.py .

# Just the detector's negative control, when you have touched the detector itself.
check-language-self-test:
    @python3 scripts/no_french.py --self-test

# The whole test suite.
test:
    cargo test --workspace --all-targets

# A single test, by name. E.g. `just test-one stale_read`
test-one NAME:
    cargo test --workspace {{NAME}} -- --nocapture

# The daemon, in the foreground, logs on stderr.
run:
    RUST_LOG=trame=debug cargo run -p trame-daemon

# The desktop app. Observes the current project.
gui project=".":
    cargo run -p trame-gui -- {{project}}

# WRITES into the target project: the path is mandatory, and a directory containing
# .git is refused. While it runs, from another terminal:
#   echo '// added by hand' >> <project>/notes.txt
# The GUI with the canonical scenario played by the REAL registry, but with NO agent.
gui-scenario project:
    cargo run -p trame-gui -- {{project}} --scenario

# This is the only proof that shaders compiled AT LAUNCH work — `runtime_shaders` moves
# that cost from the build to startup, and a green build proves nothing about it.
# REQUIRES A GRAPHICAL SESSION: impossible on a headless runner (named hole, ADR 0023).
# ★ GUI smoke test: opens a window, requires an IMAGE to be produced, exits 0.
smoke:
    #!/usr/bin/env sh
    set -e
    project=$(mktemp -d)
    cargo run -q -p trame-gui -- "$project" --smoke
    rm -rf "$project"

# ★ SONDE, non adoptee : gpui-component 0.5.1 entre tes mains.
#
# Ordre de manipulation, du plus decisif au moins :
#   1. le champ multi-ligne — tape DIX LIGNES, puis selection a la souris, clic pour
#      placer le curseur, une ligne longue qui doit passer a la ligne, cmd-C/cmd-V,
#      et un caractere accentue ou une saisie IME
#   2. la liste de 1000 lignes, a la molette
#   3. les deux boutons : celui de droite est LEUR Button avec NOS methodes tailwind
#      enchainees dessus. S'ils se ressemblent, .refine_style() ne fait pas ce que la
#      lecture de la source annoncait
#   4. le compteur « observed » bouge tout seul : c'est NOTRE Receiver<Observation>,
#      attendu depuis l'executor de gpui
#
# Le demarrage s'imprime sur stderr : FIRST_FRAME_MS et COMPONENT_INIT_MS.
probe-component:
    cargo run -p trame-gui --example component_probe

# Logs go to stderr: `just tui 2>/tmp/tui.log` if the output gets in your way.
# The TUI: observes the current project, with the real journal, registry and FSEvents watcher.
tui project=".":
    RUST_LOG=trame=debug cargo run -p trame-tui -- {{project}}

# Run this in a CLEAN terminal: Claude Code refuses to start inside another session.
# Consumes tokens. While it runs, from another terminal:
#   echo '// added by hand' >> /tmp/trame-experience-live/notes.txt
# ★ The canonical scenario LIVE, with two real Claude Code sessions.
run-tui:
    cargo run -p trame-tui --example notice_experiment -- --tui

# The measurement round, with no interface: three notice variants on real sessions.
experiment *args:
    cargo run -p trame-tui --example notice_experiment -- {{args}}

# A reads auth.rs, B writes it, A writes elsewhere -> StaleRead on screen. The path is
# mandatory and a directory containing .git is refused: this mode WRITES into the project.
# The TUI with the canonical scenario played by the REAL registry, but with NO agent.
tui-scenario project:
    RUST_LOG=trame=debug cargo run -p trame-tui -- {{project}} --scenario

# ★ The canary: checks that the ACP adapter still removes the agent's native write
# tools. Our interception invariant depends on that behaviour, which is an UNSPECIFIED
# implementation detail of a third-party package — and which already disappeared once in
# the successor package. Consumes no tokens: the real `claude` is never launched.
#
# To aim at a candidate version before committing to it:
#   TRAME_ACP_COMMAND=/path/to/claude-agent-acp just canary
canary:
    cargo test -p trame-agent --test interception_canary -- --nocapture

# What CI does, locally, before pushing.
ci: lint test canary
    cargo build --workspace --release

# Reclaim disk. `target/` reached 21 GB during one long session — see the comment on
# [profile.dev] in Cargo.toml for why, measured rather than guessed.
#
# Keeps the release directory, which is where the probe binary lives. Pass `all` to wipe
# that too.
clean what="debug":
    #!/usr/bin/env sh
    before=$(du -sk target 2>/dev/null | cut -f1)
    if [ "{{what}}" = "all" ]; then
        cargo clean
    else
        rm -rf target/debug target/doc
    fi
    after=$(du -sk target 2>/dev/null | cut -f1)
    echo "target: $((before / 1024)) MB -> $((after / 1024)) MB"

# Where the global journal lives. Useful when unsure what you are inspecting.
journal-path:
    @echo "$HOME/Library/Application Support/Trame/"

# GitButler state. Run this BEFORE any mutation to get the current IDs.
# `--format json`, not `--json`: the latter does not exist.
status:
    but status --format json
