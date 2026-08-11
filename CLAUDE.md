# Trame

Application desktop macOS, en Rust, qui orchestre plusieurs agents de code
(Claude Code, Codex, Gemini CLI) travaillant **en parallele dans un unique
repertoire de travail partage**.

## La these — lis ces cinq lignes avant tout le reste

Tous les outils concurrents (Conductor, Xirp, Crystal, pdb-env) isolent chaque
agent : un worktree git ou une copie de repertoire par session. L'isolation
supprime les collisions, mais elle rend la coordination **impossible** — chaque
agent est aveugle aux autres par construction. Trame fait le pari inverse :
**repertoire partage + coordination appliquee**.

> Quand l'agent A s'apprete a ecrire, si un fichier qu'il a **lu** a ete modifie
> depuis par une autre session, A raisonne sur un monde qui n'existe plus.
> Trame le detecte et **l'en informe**.

C'est le seul mecanisme du produit qui n'existe nulle part ailleurs, et le seul
qui soit structurellement inatteignable pour les concurrents : trop tard au niveau
de la forge (le code est deja ecrit), impossible au niveau du systeme de fichiers
(les agents sont isoles).

**Tout le reste de ce projet existe pour servir ce mecanisme.** Devant une decision
de conception, la question est : est-ce que ca sert cet avis-la ? Si non, c'est
probablement hors scope.

Le mode d'echec a attraper, celui qui ne produit **aucune collision d'ecriture** :

```
1. Session A lit auth.rs, memorise la signature de verify_token()
2. Session B ecrit auth.rs, renomme verify_token() -> validate_token()
3. Session A ecrit handlers.rs, appelle verify_token()

Deux fichiers differents. Un verrou par fichier ne voit rien. L'arbre est casse.
```

## Decisions prises — ne pas les rouvrir

Elles sont tranchees. Si tu penses qu'une est mauvaise, **dis-le et argumente**,
mais ne devie pas sans validation. Un ADR par ligne dans [`docs/adr/`](docs/adr/).

| Decision | Choix | Raison | ADR |
|---|---|---|---|
| Plateforme | macOS uniquement | FSEvents, Keychain, launchd. Pas d'abstraction cross-platform. | [0001](docs/adr/0001-macos-uniquement.md) |
| Isolation | **Aucune.** Repertoire de travail unique par projet | C'est la condition de possibilite de la coordination. | [0002](docs/adr/0002-aucune-isolation.md) |
| VCS | GitButler via la CLI `but`, en shell-out | La surface necessaire fait ~7 commandes. Reimplementer serait 6-18 mois sur une commodite. | [0003](docs/adr/0003-gitbutler-en-shell-out.md) |
| Parsing VCS | `but ... --format json` systematiquement | API structuree, pas du scraping. | [0004](docs/adr/0004-parsing-json-du-vcs.md) |
| Transport agent | ACP en premier, PTY en secours | ACP permet d'intercepter les ecritures **avant** le disque. Indispensable. | [0005](docs/adr/0005-acp-en-premier-pty-en-secours.md) |
| Concurrence | Acteurs tokio, un par domaine | mpsc + oneshot. **Aucun etat partage.** | [0006](docs/adr/0006-acteurs-tokio.md) |
| Controle de concurrence | Optimiste, validation du read-set | Le locking pessimiste famine sur des transactions de plusieurs minutes. | [0007](docs/adr/0007-concurrence-optimiste-read-set.md) |
| Stockage | SQLite via `rusqlite`, append-only | On voudra requeter en transverse projets. | [0008](docs/adr/0008-journal-sqlite-append-only.md) |
| Licence | FSL-1.1-MIT (Fair Source) | Conversion automatique en MIT a 2 ans. | [0009](docs/adr/0009-licence-fsl-1-1-mit.md) |
| Parallelisme | Par **projets**, pas par sessions | 2-5 sessions par projet. 5 projets × 3 sessions = 15 agents, tous surs. | [0010](docs/adr/0010-parallelisme-par-projets.md) |
| Forge | GitLab **self-hosted** en premiere cible | `base_url` est un champ de premiere classe des le depart. | [0011](docs/adr/0011-gitlab-self-hosted-en-premier.md) |
| Granularite v0.1 | Fichier entier, pas de hunks | 90 % de la valeur pour 5 % du travail. On raffine apres mesure. | [0012](docs/adr/0012-granularite-fichier-en-v0-1.md) |

## Non-objectifs — a refuser explicitement

- ❌ Windows, Linux
- ❌ Un editeur de code embarque (**ce n'est pas un IDE**)
- ❌ Un modele ou un agent proprietaire
- ❌ SaaS, comptes, backend, multi-utilisateur
- ❌ Worktrees, conteneurs, copy-on-write, microVMs
- ❌ Webhooks, polling, declenchement automatique de sessions
- ❌ **Toute forme d'isolation**

Le mot « open source » n'apparait nulle part dans ce depot. Le terme est
**Fair Source** — la clause de non-concurrence de la FSL la rend non compatible
OSI, et se tromper de vocabulaire coute plus cher que ca n'en a l'air.

## Invariants d'architecture

Ce sont des invariants, pas des preferences. Une violation est un bug.

1. **Un acteur possede son etat.** Jamais de `Arc<Mutex<_>>` pour de l'etat
   metier. La communication passe par `mpsc` en entree et `oneshot` en retour.
   Ce qui donne la serialisation et l'ordre total **par construction**, sans
   verrou. Un `Arc` sur une valeur immuable (une horloge, une config) n'est pas
   concerne.
2. **Le registre est le point de passage unique des ecritures.** Rien n'ecrit a
   cote. Une ecriture qui contourne le registre est une ecriture sans provenance,
   donc une ligne fausse dans le journal.
3. **Le numero de sequence est par projet, jamais global.** Un compteur global
   serait un point de contention entre projets qui, par construction, ne peuvent
   pas entrer en collision. Contrainte `UNIQUE(project_id, seq)`.
4. **Aucun `unwrap()` / `expect()` / `panic!()` en dehors des tests.** Denies par
   clippy au niveau du workspace ; `clippy.toml` porte les exemptions de test.
5. **Erreurs : `thiserror` dans les bibliotheques, `anyhow` uniquement dans les
   binaires.** Une bibliotheque qui renvoie `anyhow::Error` force son appelant a
   faire du pattern matching sur des chaines.
6. **Toute I/O est instrumentee avec `tracing`.** Jamais de `println!` /
   `eprintln!` — les deux sont denies par clippy. Les logs vont sur stderr :
   stdout appartient au terminal alternatif de ratatui et au JSON-RPC.
7. **`trame-core` ne depend d'aucun crate interne.** La direction de dependance
   est unique : `core <- journal <- registry <- {agent, vcs} <- daemon <- tui`.
8. **Silencieux quand c'est propre.** ~95 % du trafic doit passer sans un mot.
   Un outil qui crie au loup est desactive en une semaine — c'est le risque
   produit numero un, avant tout risque technique.
9. **Rien n'est bloque en v0.1.** Le registre observe, journalise et informe. Le
   blocage se decidera apres mesure du taux reel de faux positifs.

## Regle de controle de version — GitButler workspace mode

Ce depot est en **workspace mode GitButler**.

> **Ne jamais utiliser `git commit`, `git add`, `git push`.**

Le flux correct :

```sh
but status --format json    # TOUJOURS d'abord : recupere les cliId courants
but commit <branch-id> -m "message" --changes <file-ids>
```

Attention a la forme du drapeau : c'est **`--format json`**, pas `--json` (qui
n'existe pas et echoue). Le `cliId` d'un fichier est la valeur a passer a
`--changes`, en liste separee par des virgules.

Les IDs de branche et de fichier sont **volatils** : ils changent a chaque
mutation de l'arbre. Un ID lu il y a trois commandes est un ID perime. `but status
--format json` avant chaque mutation, sans exception.

La skill GitButler est installee par `but agent setup` — ne pas en ecrire une
autre a la main, elle serait redondante et divergerait. Elle prime sur le bloc
ci-dessous pour la **syntaxe exacte** des commandes ; le bloc ci-dessus reste la
regle du projet quant a ce qui est interdit.

<!-- gitbutler-agent-setup:start -->
## Version control

- Use GitButler (`but`) for version-control inspection and write operations, including status, diffs, branching, committing, pushing, and history edits.
- Assume multiple agents may be working in this repository. Do not move, amend, squash, discard, commit, push, or otherwise modify another agent's work unless the user asks.
- For commit just/only/specific changes on a new branch (selected-change requests), use the two-command fast path from the GitButler skill: `but diff`, then `but commit <branch> -c -m "message" --changes <id>,<id>`.
- For that fast path, after the commit succeeds, stop and summarize; do not run separate branch, staging, status, or diff commands unless the commit output is missing information you need.
- Use the installed GitButler skill for command recipes and syntax before guessing flags, using `--help`, or translating Git habits directly.
- After a successful GitButler write command, use the workspace state it returns. Rerun status or diff only when that output lacks information you need or files changed since.
- Use a dedicated GitButler branch for each agent session, unless the user asks for a different branch structure. Commit only changes that belong to that session.
- Do not push or open pull requests unless the user asks.
- Keep commit messages and pull request descriptions succinct: explain what changed, why it changed, and any important decision.

### Amend local fixes into the right commits

- For small cleanup or follow-up fixes, amend an unpublished local commit when the change clearly belongs with that commit's intent.
- Do not create tiny fixup commits unless the user asks.
- Use GitButler to move the relevant changes into the commit where they belong.
- Ask before rewriting pushed, reviewed, shared, or ambiguous history.

### Split unrelated changes into separate commits

- If one file contains unrelated changes, split them by hunk instead of committing the whole file.
- Keep tests with the behavior they verify.
- Split generated output, docs-only edits, or mechanical cleanup into separate commits when each commit remains coherent on its own.
- If the split is ambiguous, summarize the options before committing.
<!-- gitbutler-agent-setup:end -->

## Structure

```
crates/
├── trame-core/       # types partages, coutures. Aucune dependance interne.
├── trame-journal/    # SQLite append-only, global au workspace
├── trame-registry/   # ★ l'acteur d'admission — le coeur du produit
├── trame-agent/      # trait AgentBackend, AcpBackend, PtyBackend
├── trame-vcs/        # trait VcsBackend, ButBackend
└── trame-daemon/     # Supervisor, orchestration
apps/
└── trame-tui/        # ratatui
```

Plusieurs de ces crates sont quasi vides. **C'est voulu** : les frontieres de
crates *sont* l'architecture. Les poser maintenant coute une journee, les
retrofitter coute une reecriture.

### Les coutures de `trame-core`

Definies des la phase 0, presque inutiles aujourd'hui, structurantes dans six mois :

- `TaskSource` — d'ou vient le travail. Une seule implementation en v0.1 :
  `ManualTask` (l'utilisateur tape son prompt).
- `Forge` — ou va le resultat. **Nommage neutre : `ChangeRequest`, jamais
  `PullRequest`.** GitLab est la cible primaire, pas un citoyen de seconde zone.
- `PromptContributor` — pipeline de composition du prompt. **Pas speculatif** :
  c'est par ce mecanisme que l'avis de lecture perimee est injecte. La v0.1 en a
  besoin.
- `BranchTarget` — `New(BranchName)` ou `Existing(BranchId)`. Sans ca, traiter les
  commentaires de review d'une MR imposerait un refactor.
- `Session.work_item: Option<WorkItemRef>` — ferme la chaine auditable complete
  `issue -> session -> agent -> ecritures -> hunks -> branche -> MR`.
- `Clock` — toute lecture de l'heure passe par la. Le registre prend des decisions
  qui dependent du temps ; les tester avec l'horloge systeme imposerait des
  `sleep`, donc des tests lents et instables.

## Commandes

```sh
just check       # compile tout le workspace, tests inclus
just test        # la suite complete
just test-one X  # un seul test, par nom, avec --nocapture
just lint        # fmt --check + clippy -D warnings. Ce que la CI verifie.
just run         # le daemon, logs sur stderr
just tui         # le TUI
just ci          # lint + test + build release, en local avant de pousser
just status      # but status --format json
```

Zero warning tolere. La CI ([`.gitlab-ci.yml`](.gitlab-ci.yml), pas GitHub
Actions) echoue sur le moindre warning clippy, ce qui inclut la documentation
manquante sur un item public.

## Ou en est le projet

Phase 0 terminee : outillage, frontieres de crates, coutures, ADR, skills.

Phase 1 a venir : `trame-journal` (schema SQLite) et `trame-registry` (l'acteur
d'admission), testables **sans qu'aucun agent ne tourne**. Le scenario canonique a
couvrir est celui de la these ci-dessus : A lit `auth.rs`, B ecrit `auth.rs`
(→ `Clean`), A ecrit `handlers.rs` (→ `StaleRead { auth.rs, par B }`).

Les phases et leurs points d'arret sont dans [`docs/concept.md`](docs/concept.md)
(section Roadmap). **Une phase a la fois, arret a chaque point de controle.**

## Comment travailler ici

- Un ADR par decision non triviale. Format et criteres :
  [`.claude/skills/adr-format/SKILL.md`](.claude/skills/adr-format/SKILL.md).
- **Les tests avant le cablage** sur tout ce qui touche a la concurrence.
- Pas de `sleep` dans les tests. L'horloge s'injecte.
- Sur ce projet greenfield, les subagents se lancent **en sequence**, pas en
  parallele : les crates n'ont pas encore de frontieres stables et des agents
  paralleles se marcheraient dessus. L'ironie n'echappera a personne — c'est
  exactement le probleme que Trame resout, et Trame n'existe pas encore.
- Si quelque chose est ambigu sur l'architecture : **demander**, pas deviner.
