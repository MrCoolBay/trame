# Trame — recettes de developpement.
# `just` seul liste les recettes disponibles.

default:
    @just --list

# Compile tout le workspace, tests inclus, sans produire de binaire.
check:
    cargo check --workspace --all-targets

# Formatage.
fmt:
    cargo fmt --all

# Ce que la CI verifie : format + clippy sans le moindre warning.
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

# Toute la suite de tests.
test:
    cargo test --workspace --all-targets

# Un seul test, par nom. Ex: `just test-one stale_read`
test-one NAME:
    cargo test --workspace {{NAME}} -- --nocapture

# Le daemon, en premier plan, logs sur stderr.
run:
    RUST_LOG=trame=debug cargo run -p trame-daemon

# Le TUI.
tui:
    RUST_LOG=trame=debug cargo run -p trame-tui

# Ce que la CI fait, en local, avant de pousser.
ci: lint test
    cargo build --workspace --release

# Emplacement du journal global. Utile quand on doute de ce qu'on inspecte.
journal-path:
    @echo "$HOME/Library/Application Support/Trame/"

# Etat GitButler. A lancer AVANT toute mutation pour recuperer les IDs courants.
# `--format json`, pas `--json` : ce dernier n'existe pas.
status:
    but status --format json
