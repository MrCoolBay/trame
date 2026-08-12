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

# Ce que la CI verifie : format + clippy sans le moindre warning + etancheite des features.
lint: check-features
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

# Garde-fou : la feature `test-support` de trame-core donne acces a `ManualClock`,
# une horloge manipulable. Elle ne doit etre activee QUE par des dev-dependencies,
# jamais atteignable depuis un build de production.
#
# Deplacer une de ces trois lignes de `[dev-dependencies]` vers `[dependencies]`
# ferait fuiter l'horloge de test dans le binaire livre, sans aucun warning. D'ou
# cette verification, qui echoue si une arete hors-dev active la feature.
check-features:
    #!/usr/bin/env sh
    if cargo tree --workspace -e features -i trame-core --edges no-dev 2>/dev/null \
        | grep -q 'test-support'; then
        echo "ECHEC : la feature test-support est atteignable hors dev-dependencies." >&2
        echo "Cherche un 'features = [\"test-support\"]' sous [dependencies]." >&2
        exit 1
    fi
    echo "features : test-support confinee aux dev-dependencies"

# Toute la suite de tests.
test:
    cargo test --workspace --all-targets

# Un seul test, par nom. Ex: `just test-one stale_read`
test-one NAME:
    cargo test --workspace {{NAME}} -- --nocapture

# Le daemon, en premier plan, logs sur stderr.
run:
    RUST_LOG=trame=debug cargo run -p trame-daemon

# Les logs vont sur stderr : `just tui 2>/tmp/tui.log` si l'affichage te gene.
# Le TUI : observe le projet courant, journal, registre et watcher FSEvents reels.
tui projet=".":
    RUST_LOG=trame=debug cargo run -p trame-tui -- {{projet}}

# A lancer dans un terminal PROPRE : Claude Code refuse de demarrer dans une autre session.
# Consomme des jetons. Pendant que ca tourne, depuis un autre terminal :
#   echo '// ajoute a la main' >> /tmp/trame-experience-live/notes.txt
# ★ Le scenario canonique EN DIRECT, avec deux vraies sessions Claude Code.
manche-tui:
    cargo run -p trame-tui --example experience_avis -- --tui

# La manche de mesure, sans interface : trois variantes d'avis sur de vraies sessions.
manche *args:
    cargo run -p trame-tui --example experience_avis -- {{args}}

# A lit auth.rs, B l'ecrit, A ecrit ailleurs -> StaleRead a l'ecran. Le chemin est
# obligatoire et un repertoire contenant .git est refuse : ce mode ECRIT dans le projet.
# Le TUI avec le scenario canonique joue par le VRAI registre, mais SANS agent.
tui-scenario projet:
    RUST_LOG=trame=debug cargo run -p trame-tui -- {{projet}} --scenario

# ★ Le canari : verifie que l'adaptateur ACP retire toujours les outils d'ecriture
# natifs de l'agent. Notre invariant d'interception depend de ce comportement, qui est un
# detail d'implementation NON SPECIFIE d'un paquet tiers — et qui a deja disparu une fois
# dans le paquet successeur. Ne consomme aucun jeton : le vrai `claude` n'est pas lance.
#
# Viser une version candidate avant de s'y engager :
#   TRAME_ACP_COMMAND=/chemin/vers/claude-agent-acp just canari
canari:
    cargo test -p trame-agent --test canari_interception -- --nocapture

# Ce que la CI fait, en local, avant de pousser.
ci: lint test canari
    cargo build --workspace --release

# Emplacement du journal global. Utile quand on doute de ce qu'on inspecte.
journal-path:
    @echo "$HOME/Library/Application Support/Trame/"

# Etat GitButler. A lancer AVANT toute mutation pour recuperer les IDs courants.
# `--format json`, pas `--json` : ce dernier n'existe pas.
status:
    but status --format json
