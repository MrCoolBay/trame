# Trame

Application desktop macOS, en Rust, qui orchestre plusieurs agents de code
(Claude Code, Codex, Gemini CLI) travaillant **en parallele dans un unique
repertoire de travail partage**.

> **Ce fichier est le cadrage canonique du projet.** Il est volontairement neutre :
> Trame orchestre trois harnesses differents, et le jour ou une session Trame lance
> Codex ou Gemini sur le depot Trame lui-meme, c'est ce fichier qu'ils liront.
> `CLAUDE.md` l'importe et n'ajoute que ce qui est specifique a Claude Code.
> Une information a **un seul domicile** : ne recopie rien d'ici vers ailleurs.

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
| Interception | **Validee** : annoncer `fs.writeTextFile` fait desactiver les outils d'ecriture natifs de l'agent | Trous nommes et mesures : `Bash`, hors-bande, PTY. Un filet dont on ignore les trous est pire qu'un filet dont on les connait. | [0016](docs/adr/0016-interception-avant-disque-validee.md) |
| Adaptateur ACP | **Epingle** a `@zed-industries/claude-code-acp` 0.16.2, malgre sa depreciation | Le successeur ne retire plus `Write` ni `Edit` : migrer supprimerait le mecanisme central en silence. Un canari le surveille. | [0017](docs/adr/0017-adaptateur-acp-epingle.md) |
| Concurrence | Acteurs tokio, un par domaine | mpsc + oneshot. **Aucun etat partage.** | [0006](docs/adr/0006-acteurs-tokio.md) |
| Controle de concurrence | Optimiste, validation du read-set | Le locking pessimiste famine sur des transactions de plusieurs minutes. | [0007](docs/adr/0007-concurrence-optimiste-read-set.md) |
| Stockage | SQLite via `rusqlite`, append-only | On voudra requeter en transverse projets. | [0008](docs/adr/0008-journal-sqlite-append-only.md) |
| Licence | **Open source, MIT OR Apache-2.0** | Convention Rust. La protection ne vient pas d'une clause. Remplace le choix FSL de l'ADR 0009. | [0013](docs/adr/0013-licence-open-source-mit-apache.md) |
| Parallelisme | Par **projets**, pas par sessions | 2-5 sessions par projet. 5 projets × 3 sessions = 15 agents, tous surs. | [0010](docs/adr/0010-parallelisme-par-projets.md) |
| Forge **pilotee** | GitLab **self-hosted** en premiere cible | `base_url` est un champ de premiere classe des le depart. `ChangeRequest`, jamais `PullRequest`. | [0011](docs/adr/0011-gitlab-self-hosted-en-premier.md) |
| Hebergement de **Trame** | GitHub | En MIT/Apache, l'hebergement doit etre la ou sont les contributeurs. **N'affecte pas la ligne au-dessus** : Trame est heberge sur GitHub et parle GitLab. | [0019](docs/adr/0019-heberger-trame-sur-github.md) |
| Granularite v0.1 | Fichier entier, pas de hunks | 90 % de la valeur pour 5 % du travail. On raffine apres mesure. | [0012](docs/adr/0012-granularite-fichier-en-v0-1.md) |
| Ecriture disque | **Le registre ecrit**, il ne rend pas qu'un verdict | Un invariant qui repose sur la discipline de l'appelant n'est pas un invariant. | [0014](docs/adr/0014-le-registre-ecrit-sur-disque.md) |
| Backpressure | Canal borne a 64, on attend en saturation | Une file non bornee transforme une surcharge en fuite memoire. Une saturation est un bug, pas un manque de capacite. | [0015](docs/adr/0015-canal-admit-borne.md) |
| Interface | **Elle observe, elle ne pilote pas** : un `Receiver<Observation>`, aucun `RegistryHandle` | Le daemon est le produit, la GUI est interchangeable — et c'est ce qui autorise a parier sur un framework pre-1.0. | [0022](docs/adr/0022-decoupage-daemon-gui.md) |
| Trou lecture | **Ouvert**, et mesure en **mode ombre** | Le fermer sans mesurer le taux de faux positifs serait un pari sur l'invariant 8. L'ombre compte ce qu'on aurait dit et ne dit rien ; la distribution des tailles donnera le seuil. | [0027](docs/adr/0027-trou-lecture-ouvert-et-mesure-en-ombre.md) |
| Hooks de la CLI | `trame-hook` demande au daemon par une **socket unix par projet** ; un daemon absent fait **echouer** le hook | Sur le chemin d'admission, l'absence de reponse n'est jamais un oui. Un hook qui sort 0 sans avoir consulte la politique tue l'invariant en silence. | [0025](docs/adr/0025-ipc-hook-daemon.md) |
| Outil d'ecriture maison | **Non.** Piste documentee, pas construite | Elle doublerait la surface du chemin d'ecriture, et rien ne dit que l'agent choisirait notre outil plutot que `Write` qu'il connait. Trois declencheurs de reexamen, tous observables. | [0024](docs/adr/0024-pas-de-serveur-mcp-maison.md) |
| Bibliotheque de composants | `gpui-component` **0.5.1**, crates.io | Le champ multi-ligne valait la dependance ; le reste non. `Styled` expose et `.refine_style()` qui raffine par-dessus le preset donnent l'echappatoire — on habille la bibliotheque au lieu d'etre habille par elle. **`multi_line(true)` est un piege, `auto_grow(min, max)` est le seul chemin correct.** | [0028](docs/adr/0028-adoption-de-gpui-component.md) |
| Framework GUI | `gpui` de l'**amont Zed**, epingle a 0.2.2 | Propriete du crate etablie par la team crates-io, parite d'API constatee (sonde rebatie sans toucher `main.rs`), une version et non une branche git. `gpui-ce` reste l'echappatoire, deja testee. | [0023](docs/adr/0023-gpui-amont-pour-la-gui.md) |

## Non-objectifs — a refuser explicitement

- ❌ Windows, Linux
- ❌ Un editeur de code embarque (**ce n'est pas un IDE**)
- ❌ Un modele ou un agent proprietaire
- ❌ SaaS, comptes, backend, multi-utilisateur
- ❌ Worktrees, conteneurs, copy-on-write, microVMs
- ❌ Webhooks, polling, declenchement automatique de sessions
- ❌ **Toute forme d'isolation**

## Invariants d'architecture

Ce sont des invariants, pas des preferences. Une violation est un bug.

1. **Un acteur possede son etat.** Jamais de `Arc<Mutex<_>>` pour de l'etat
   metier. La communication passe par `mpsc` en entree et `oneshot` en retour.
   Ce qui donne la serialisation et l'ordre total **par construction**, sans
   verrou. Un `Arc` sur une valeur immuable (une horloge, une config) n'est pas
   concerne.
2. **Le registre est le point de passage unique des ecritures d'agent faites par les
   outils de fichiers** — `Write`, `Edit`, `NotebookEdit` — et c'est **lui qui ecrit**
   (ADR 0014). Les ecritures par le shell de l'agent (`Bash`) y echappent : c'est mesure,
   assume, et ca doit etre affiche tel quel (ADR 0016). Il ne rend pas un verdict en laissant l'appelant ecrire : un
   invariant qui repose sur la discipline de chaque site d'appel n'est pas un
   invariant. Une ecriture qui contourne le registre est une ecriture sans provenance,
   donc une ligne fausse dans le journal — pire que pas de journal.
   Les ecritures hors-bande (`sed -i`, hooks, build, formatters) sont rattrapees par
   FSEvents et jamais admises.
   **Symetriquement, le read-set ne contient que les lectures faites par l'outil de
   lecture ACP.** Une lecture par `Grep`, `Glob` ou `Bash` echappe au registre — et c'est
   plus grave qu'une ecriture qui echappe : sans entree de read-set, `StaleRead` ne se
   declenche jamais et rien ne le signale. La sortie est **mesuree mais pas construite** :
   le hook `PostToolUse` rend les fichiers lus par `Grep` et `Glob`
   ([sonde 3](docs/sondes/2026-08-12-postooluse.md)). Tant que ce n'est pas implemente,
   l'enonce ci-dessus reste vrai tel quel.
   Cote ecriture, le **watcher FSEvents** rattrape le hors-bande : il n'empeche rien, mais
   il empeche le registre de devenir **faux**. Sans lui, un `sed -i` laisse un `FileState`
   perime et le `StaleRead` correspondant ne se declenche jamais. Ces ecritures sont
   attribuees a `SessionId::EXTERNAL` et journalisees avec `origin = observed`, **sans
   verdict** — personne ne les a admises.
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
   est unique : `core <- journal <- registry <- {agent, vcs} <- daemon <- view <- {tui, gui}`.
   Une interface ne recoit qu'un `Receiver<Observation>`, **jamais un `RegistryHandle`** :
   « elle observe, elle ne pilote pas » est dans le typage (ADR 0022).
8. **Silencieux quand c'est propre.** ~95 % du trafic doit passer sans un mot.
   Un outil qui crie au loup est desactive en une semaine — c'est le risque
   produit numero un, avant tout risque technique.
9. **Rien n'est bloque en v0.1.** Le registre observe, journalise et informe. Le
   blocage se decidera apres mesure du taux reel de faux positifs.
10. **L'empreinte d'une lecture ne se calcule que sur le contenu servi en reponse a
    `fs/read_text_file`** — jamais sur le payload d'un hook (ADR 0020). La CLI y injecte
    un `<system-reminder>` : l'empreinte ne correspondrait a **aucun** etat du disque, et
    l'echec serait totalement silencieux — read-set peuple, `StaleRead` mort, aucun test
    casse. Quand un hook rapporte un chemin (`Grep`, `Glob`), Trame **relit le fichier**
    pour l'empreinter ; le hook fournit des chemins, jamais du contenu.

## Structure

```
crates/
├── trame-core/       # types partages, coutures. Aucune dependance interne.
├── trame-journal/    # SQLite append-only, global au workspace
├── trame-registry/   # ★ l'acteur d'admission — le coeur du produit
├── trame-agent/      # trait AgentBackend, AcpBackend, PtyBackend
├── trame-vcs/        # trait VcsBackend, ButBackend
├── trame-daemon/     # Supervisor, orchestration, canal d'observation
└── trame-view/       # etat d'affichage + ouverture d'un projet, partages par les interfaces
apps/
├── trame-tui/        # ratatui — le rendu terminal, et rien d'autre
└── trame-gui/        # gpui (amont Zed) — l'application desktop
```

`trame-vcs` est encore quasi vide. **C'est voulu** : les frontieres de
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

Zero warning tolere. La CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml))
echoue sur le moindre warning clippy, ce qui inclut la documentation manquante sur
un item public.

**Deux exclusions dans la CI, et ce sont des conditions de validite, pas des oublis.**
`trame-gui` ne passe pas dans les jobs Linux — gpui n'a pas de couche plateforme sans
`x11`/`wayland`. Et `real_watcher` ne compile que sur macOS : `notify` choisit inotify sur
Linux, donc un vert Linux sur ce fichier mesurerait un autre backend que celui dont le titre
parle. Les deux sont couvertes par le job `macos`, qui tourne sur le chemin critique depuis qu'on a
mesure qu'un runner macOS GitHub est bien dans une session **Aqua** — la fenetre s'ouvre et le
test de fumee des shaders rend `SMOKE_OK`.

## Licence

Trame est **open source** sous **MIT OR Apache-2.0**
([ADR 0013](docs/adr/0013-licence-open-source-mit-apache.md)). Pas de CLA : une
contribution est offerte sous les memes termes, comme partout dans l'ecosysteme
Rust.

`but` (GitButler, sous FSL-1.1-MIT) est un **prerequis externe installe par
l'utilisateur, jamais vendorise** — c'est une contrainte de licence, pas une
preference d'empaquetage ([ADR 0003](docs/adr/0003-gitbutler-en-shell-out.md)).

## Ou en est le projet

**Phases 0 a 3 livrees, TUI incluse.** 139 tests. Les seuls `sleep` du depot sont dans
`real_watcher.rs`, ou FSEvents est un service du systeme qu'aucune horloge injectee ne
controle — et encore, par attente d'une condition avec plafond, pas par delai fixe.

- **Phase 0** — outillage, frontieres de crates, coutures, ADR, skills.
- **Phase 1** — `trame-journal` (six tables append-only, ecritures reelles) et
  `trame-registry` (l'acteur d'admission). Le scenario canonique passe : A lit
  `auth.rs`, B ecrit `auth.rs` (→ `Clean`), A ecrit `handlers.rs`
  (→ `StaleRead { auth.rs, par B }`). Deux fichiers differents, aucune collision
  d'ecriture.
- **Phase 2** — `trame-agent` : `AgentBackend`, flux normalise, `AcpBackend` pour
  Claude Code, `PtyBackend` en squelette honnete. L'interception avant disque est
  **validee, run live inclus** (ADR 0016) : deux sessions Claude Code reelles ont demande
  a ecrire, nous avons refuse, rien n'a atteint le disque.

**Une regle nee du run live** : toute cle de fichier passe par `trame_core::ProjectRoot`.
L'agent renvoie des chemins absolus et resolus (`/private/var/…` quand la racine est
`/var/…`) ; sans normalisation, `StaleRead` cesse de se declencher **sans que rien ne
casse**, et les tests passent quand meme.

- **Phase 3** — 3.1 et 3.2 livrees : le registre **ecrit** apres admission (ADR 0014), et
  `SessionPilot` cable la chaine complete. Le test de bout en bout fait passer le scenario
  canonique par le vrai transport, jusqu'a l'avis pose devant le prompt suivant.
  3.3 livree cote outillage : `just experiment` (`-p trame-tui --example notice_experiment`) mesure
  les trois variantes d'avis sur de vraies sessions.

  3.3 **tranchee, et le texte livre a change** : `StaleFile` ne porte **pas** de resume du
  changement et le registre ne calcule aucun diff a l'admission (ADR 0018) — cette partie tient,
  et la mesure du 2026-08-13 la renforce plutot que l'inverse.
  **Mais le texte livre n'avait jamais ete mesure.** Les `5/5` puis `3/3` portaient sur
  `ConfigurableNotice::Neutral`, un jumeau de `StaleReadNotice` a une ligne pres. Mesure directe
  de la production : **`3/6`**, contre `3/3` pour la neutre et `3/3` pour la directive, meme jour
  et memes conditions. La chaine que Trame envoyait etait **la seule des trois qui echouait**.
  La troisieme ligne — `Re-read it before continuing if your work depends on it.` — a ete
  **retiree**, et le rejeu donne **`6/6`**. L'avis fait deux lignes :

  ```
  [Trame] auth.rs was changed by session "refactor-api"
          after you read it (a few seconds ago).
  ```

  Le mecanisme, qui vaut au-dela de ce cas : **un agent qui recoit un fait agit ; un agent qui
  recoit un fait plus la permission de l'ignorer l'ignore une fois sur deux.** Et la reserve, a
  ne pas perdre : **six runs ne donnent aucune puissance statistique** — ce qui est solide est la
  direction, pas l'amplitude.

  3.4 — le **watcher FSEvents**, puis la **TUI**. `trame_daemon::observe` porte le canal
  d'observation, a sens unique : l'interface recoit un `Receiver<Observation>` et **aucun
  `RegistryHandle`**, donc elle ne peut structurellement pas piloter. Elle affiche un
  panneau par session avec son etat, le flux des verdicts, les `StaleRead` distincts des
  `Clean`, la distinction **admis / observe**, et une banniere de degradation quand
  `can_intercept_writes` est faux. `trame-tui <projet> [--scenario]` ouvre le vrai journal,
  le vrai registre et le vrai watcher.

  Ce que le rendu en terminal reel a trouve, et que les tests ne voyaient pas : le watcher
  emettait ses observations **sans savoir** si le registre les avait retenues. Comme le
  registre ecrit lui-meme (ADR 0014), FSEvents remonte ses propres ecritures, et
  l'interface les affichait comme hors-bande — l'inverse exact de la verite.
  `RegistryHandle::observe_external_write` rend desormais un
  `ExternalWrite::{Recorded, Echo}`.

### Dette de validation, a ne pas oublier

Le `15/15` de la manche signalait un **test qui ne discrimine plus**, pas un message optimal —
et c'etait plus vrai qu'on ne le croyait : **il ne mesurait meme pas le bon texte.** Le
dispositif s'est mis a discriminer le jour ou on lui a donne `StaleReadNotice` a mesurer, et il
a immediatement produit un echec, `3/6`.

Le scenario reste court (trois tours), le contexte accumule faible, et le changement mesure — un
identifiant renomme — est le plus lisible qui existe. `Grep`/`Glob`/`Bash` etaient fermes ; cette
limite-la a ete levee (ADR 0018) et le read-set s'est peuple quand meme.

**Deux choses a ne pas confondre dans un rapport d'avancement :**

- **La formulation de la troisieme ligne de l'avis** est ouverte, avec une mesure a l'appui et
  deux causes candidates encore confondues — le pronom `it` et la conditionnelle `if your work
  depends on it`. La discriminer demande de varier un point a la fois.
- **La question du resume** reste fermee. Aucune des trois formes ne porte de diff, et la neutre
  en dit **moins** que la production tout en reussissant mieux : l'echec ne s'explique pas par un
  manque de contexte. Le declencheur de reouverture reste le cas realiste — session longue,
  changement subtil, plan deja engage. Detail dans l'ADR 0018.

Les phases et leurs points d'arret sont dans [`docs/concept.md`](docs/concept.md)
(section Roadmap). **Une phase a la fois, arret a chaque point de controle.**

## Comment travailler ici

- Un ADR par decision non triviale, avec une section « ce qui invaliderait cette
  decision » contenant une condition **observable**.
- **Les tests avant le cablage** sur tout ce qui touche a la concurrence.
- Pas de `sleep` dans les tests. L'horloge s'injecte.
- Si quelque chose est ambigu sur l'architecture : **demander**, pas deviner.
- **Ce qui traverse une frontiere se voit tourner pour de vrai.** Voir ci-dessous.

### ★ La regle nee de neuf fois le meme bug

> **Tout mecanisme qui traverse une frontiere — protocole tiers, systeme de fichiers,
> terminal — doit avoir ete vu tourner pour de vrai avant d'etre considere comme acquis.**
> Les tests etablissent qu'il est coherent avec ce qu'on croit de la frontiere. Ils
> n'etablissent jamais ce que la frontiere fait.

Neuf fois sur ce projet, le meme mode d'echec. A chaque fois c'est **l'execution reelle** qui a
tranche, jamais la suite de tests — qui etait verte.

| Ce qui etait affirme | Ce qui se passait | Ce qui l'a trouve |
|---|---|---|
| le flux emet `Done` en fin de tour (phase 2) | le test **emettait lui-meme** la notification attendue, qui n'existe pas | la premiere manche avec un vrai agent, bloquee entre deux tours |
| `PostToolUse` se declenche apres un refus (sonde 3) | le heredoc etait le stdin de python, le hook n'observait **rien** | un comptage : « `pre.jsonl` devrait contenir une ligne par appel » |
| l'interface distingue admis et observe (TUI) | le watcher affichait les ecritures **du registre** comme hors-bande | le rendu dans un vrai terminal, avant qu'un test existe |
| le watcher constate le hors-bande pendant toute la session (`--tui`) | un `?` sur l'ouverture de session relachait le socle, le watcher **s'arretait** | une ecriture faite a la main pendant un run, qui n'apparaissait pas |
| `real_watcher` teste FSEvents (CI) | `notify` choisit **inotify** sur Linux : un job Linux aurait valide un autre backend | la lecture du code en preparant la migration de CI — **le premier attrape avant degat** |
| l'echo d'une ecriture admise ne consomme pas de sequence | l'assertion comparait le compteur **global** pour une propriete **par fichier** ; les ecritures de fixture le faisaient avancer | le job macOS de la CI. Le test passait **par chance** depuis des semaines, sur une coincidence de timing propre a une machine |
| la manche mesure l'avis du produit (ADR 0018) | elle mesurait `ConfigurableNotice`, **jumeau** de `StaleReadNotice` a une ligne pres. Le texte livre fait `3/6` la ou le jumeau fait `3/3` | une **relecture cote a cote**, en traduisant le depot. Ni un test ni un run : les deux chaines lues dans la meme heure |
| un `_ => panic!()` alerte si une variante de `Command` apparait | `#[non_exhaustive]` ne contraint que les **autres** crates : dans le crate qui definit le type, le bras etait **mort** | **clippy**, `unreachable_pattern`. Le premier cas de la serie trouve par un lint |

Le mecanisme est toujours le meme, et c'est pour ca qu'il se repete : **une sortie plausible
ne declenche aucune verification.** Un test vert, un flux credible, un ecran qui se remplit —
rien de tout cela ne demande a etre regarde de plus pres. Un plantage, si.

Les frontieres etaient differentes — un protocole non specifie, un contrat de hook, un
terminal, un systeme de fichiers — et leur nature n'a rien change : chaque fois nous avions
**modelise** leur comportement et teste notre modele.

Le quatrieme est le plus instructif sur la duree de vie, parce qu'**aucun test ne pouvait le
voir** : le mecanisme fonctionnait, il ne vivait simplement pas assez longtemps. Une duree de
vie ne se teste pas en interrogeant une fonction — elle se constate en regardant l'ecran pendant
qu'on fait quelque chose.

**Le septieme est le pire de la serie**, parce que la frontiere n'etait pas dehors : c'etait
notre propre dispositif de mesure. Il fonctionnait parfaitement, il mesurait juste **autre chose
que le produit** — et son plafond `15/15` rendait la substitution indetectable pendant deux
campagnes.

> **Un harnais de mesure doit consommer le composant de production, pas un jumeau.** Si la
> mesure passe par un type dedie a l'experience, ce type se construit comme une **comparaison
> contre la production**, et un test constate qu'ils diffèrent.

Les trois proprietes qui ont rendu le piege invisible valent d'etre reconnues ailleurs : les
deux textes **se ressemblaient**, le **plafond masquait** tout ecart possible, et **aucun test
ne les comparait** — chacun etait epingle contre lui-meme. C'est le meme angle mort que le
compteur global du cinquieme cas : l'observable choisi ne pouvait pas exprimer la propriete.

### ★★ Le cas le plus vicieux : le motif applique a la boucle de verification

Les cas ci-dessus portent sur le produit — sauf le septieme, qui portait sur le dispositif de
mesure. Celui-ci porte sur **le controle lui-meme**, et c'est pour ca qu'il merite sa section.

```sh
just lint >/dev/null 2>&1 && echo "lint OK"     # ← NE JAMAIS ECRIRE CA
```

Quand la commande echoue, cette forme **n'affiche rien**. Pas d'erreur, pas de mention, rien —
et une absence de ligne se lit comme un succes quand on parcourt une sortie. C'est arrive deux
fois de suite dans la meme session, avec un commit par-dessus a chaque fois.

> **Regle : toute commande de controle affiche explicitement le succes ET l'echec.** Jamais l'un
> par l'absence de l'autre.

```sh
if just lint; then echo "LINT : VERT"; else echo "LINT : ROUGE"; fi
```

Cette forme a immediatement revele une **seconde** erreur que la CI n'avait pas encore vue.

Le corollaire vaut pour tout script de verification : un `python3` qui leve une exception avant
d'ecrire son fichier laisse le code inchange, et le test qui suit passe — en testant l'ancienne
version. **Verifier qu'une modification a eu lieu fait partie de la verification.**

Ce que ca impose, concretement :

- **L'ordre.** Voir tourner d'abord, verrouiller par un test ensuite. L'inverse produit des
  tests qui epinglent la croyance et non le comportement. Le troisieme bug est arrive avec
  139 tests verts et n'a coute qu'un run de dix secondes dans un pty.
- **Un controle negatif** sur tout dispositif de mesure : le faire echouer volontairement
  avant de croire a son succes. Detail et exemples dans la skill `concurrency-testing`.
- **★ Un controle negatif doit etre porte par un echantillon qui l'exerce seul.** Sinon un
  trou se cache derriere les autres signaux, et le controle passe en donnant l'impression
  d'avoir verifie. Huitieme cas du motif, detaille plus bas.
- **★ Un joker est l'inverse d'une checklist.** Un `match` exhaustif **sans** bras `_` est un
  point de controle a la compilation : ajouter une variante casse le build jusqu'a ce que
  quelqu'un vienne la classer. Ajouter `_ => panic!("une variante inconnue !")` detruit
  exactement cette propriete — et `#[non_exhaustive]` ne rattrape rien, il ne contraint que
  les **autres** crates, donc le bras est mort dans le crate qui definit le type.

  > Neuvieme cas du motif, et le premier trouve par **clippy** plutot que par un run. Un lint
  > est aussi un dispositif de mesure : `-D warnings` n'est pas une coquetterie de style.
- **Un affichage qui ne separe pas deux evenements dans le temps sape la these.** Ce n'est pas
  cosmetique et c'est un critere de conception : **la these de Trame est un ordre** — « ce
  fichier a change *depuis* que tu l'as lu ». Un flux ou trois lignes portent le meme
  horodatage ne peut pas montrer l'ordre qu'il est cense demontrer. D'ou
  `trame_view::TIME_FORMAT` en millisecondes, epingle par un test.

  La generalisation utile : **avant de choisir une precision d'affichage, demander de quelle
  propriete du produit cet affichage est la preuve.**
- **Un canari** sur chaque comportement tiers dont depend un invariant — et un test qui
  verifie que le canari sait echouer.
- **Le dire quand on n'a pas vu.** Un composant seulement teste se rapporte comme tel. La
  phrase a eviter est « ca devrait marcher ».
- **Une propriete par fichier ne se teste pas avec un compteur global.** Le cinquieme cas du
  tableau est passe des semaines parce que l'assertion utilisait un proxy partage : n'importe
  quel evenement sans rapport le faisait bouger, et il ne bougeait pas sur ma machine. Choisir
  l'observable le plus etroit qui exprime la propriete.
- **Une autre machine est un dispositif de mesure.** Le job macOS de la CI a trouve en un run ce
  qu'aucun passage local n'avait vu, parce qu'il changeait l'ordonnancement. Un test vert sur une
  seule machine est un test vert sur une seule machine.
- **Un harnais mesure le composant de production, jamais un jumeau.** Septieme cas du tableau :
  la manche de l'ADR 0018 mesurait `ConfigurableNotice` en croyant mesurer `StaleReadNotice`.
  Quand un type existe pour l'experience, il se construit comme une **comparaison contre la
  production**, et un test constate qu'ils diffèrent.
- **Un plafond n'est pas un resultat, c'est un aveu.** `15/15` ne dit pas « le message est
  optimal », il dit « ce dispositif ne peut plus rien distinguer ». Tant qu'une manche n'a jamais
  rien mis en defaut, elle n'a pas encore montre qu'elle en etait capable.
- **Une commande morte dans un document est un mensonge executable.** Un flag renomme, une
  recette de `justfile` disparue, un chemin de test deplace : la prose autour peut rester juste,
  la commande, elle, echoue chez le lecteur. Un renommage n'est termine que quand les ADR, le
  README, les skills et la CI citent des commandes qui tournent.
- **Une convention se garde par un outil, pas par la vigilance.** Une convention que rien ne
  verifie tient le temps d'une session. `just check-language` echoue si du francais revient dans
  le code, la doc ou les markdown — et son **controle negatif tourne a chaque invocation**, avant
  de rapporter quoi que ce soit sur le depot. Ce controle a trouve un trou reel au premier essai :
  la recherche etait sensible a la casse, donc une majuscule de debut de phrase passait devant le
  garde-fou sans le declencher.

  La liste de mots est **volontairement courte** et mesuree a zero faux positif : `on`, `plus`,
  `son`, `sans`, `par`, `sur` sont de l'anglais, et `ce` matcherait `gpui-ce`. C'est l'invariant 8
  applique a notre propre outillage — **un garde-fou qui crie au loup est desactive en une
  semaine**, et le prix d'un mot manque est un commit de suite, celui d'un faux positif est le
  garde-fou lui-meme.

Ce n'est pas un argument contre les tests, qui sont 139 ici et non negociables. C'est un
argument sur **ce dont un test est la preuve** : de la coherence interne, jamais du
comportement de l'autre cote de la frontiere.

### ★★★ Huitieme cas : le controle negatif qui ne pouvait pas echouer

Les sept premiers cas portent sur le produit, ou sur la boucle de verification. Celui-ci porte
sur **le controle negatif lui-meme**, et c'est donc le motif une couche plus profonde que tous
les precedents.

Le garde-fou `check-language` venait d'etre ecrit. Pour verifier qu'il savait echouer, j'ai
retire un mot de sa liste — `francais` — et relance son auto-test. **Il est reste vert.** J'ai
failli lire ca comme « le garde-fou est solide ».

Ce que ca voulait dire en realite : l'echantillon censé exercer ce mot,
`"Le domaine s'ecrit en francais."`, contenait aussi `Le`. **Le detecteur l'attrapait par un
autre signal**, donc mon controle ne testait pas ce que j'affirmais qu'il testait.

> **Un controle negatif doit etre porte par un echantillon qui l'exerce seul.** Sinon le trou
> se cache derriere les autres signaux, et le controle passe en donnant l'impression d'avoir
> verifie.

Un controle refait proprement — quatre facons de casser le fichier, chacune devant le faire
rougir — a immediatement trouve **deux vrais trous** que le premier n'avait pas vus :

| ce qui etait casse | pourquoi rien ne le voyait |
|---|---|
| la recherche etait sensible a la casse | `"Le domaine…"` passait devant un garde-fou dont c'etait litteralement le travail |
| **aucun echantillon n'exercait la detection d'accents** | chaque ligne francaise de la liste contenait aussi un mot liste, donc la branche accents pouvait etre du code mort avec l'auto-test vert |

D'ou la forme retenue : deux echantillons marques `ONLY` dans
[`scripts/no_french.py`](scripts/no_french.py), l'un accent-seul et l'autre mot-seul. Chacun
porte **exactement un** signal.

**Ce que ce cas ajoute aux sept autres.** Ils disaient tous « ne crois pas un test vert sans
avoir vu le dispositif echouer ». Celui-ci ajoute le cran suivant : **avoir vu un dispositif
echouer ne suffit pas si on ne sait pas de quoi son echec est la preuve.** Un controle negatif
est lui-meme un dispositif de mesure, donc il tombe sous sa propre regle — et la recursion
s'arrete la, parce qu'un echantillon a signal unique ne laisse plus de place a un raccourci.

### ★★ L'ordre d'une conversion : prescriptions, code, prose

> **Quand une convention change, on corrige d'abord les documents qui la prescrivent, puis le
> code, puis la prose descriptive en dernier.**

Ce n'est pas une preference d'organisation, c'est ce qui separe reparer une fois de reparer a
l'infini :

> **Une regle perimee se reproduit a chaque session ; une prose perimee dort.**

Un commentaire francais oublie dans un fichier reste un commentaire francais. Une skill qui dit
« ecris tes commentaires en francais » **regenere du francais** a chaque session qui la lit, y
compris dans les fichiers qu'on vient de traduire.

**Pourquoi cette regle n'est pas evidente, et pourquoi tout le monde s'y fait piéger** : c'est
l'exact inverse de l'ordre par volume. Les prescriptions font quelques dizaines de lignes, la
prose en fait des milliers — donc on commence spontanement par le gros morceau, celui qui
ressemble au travail. L'ordre utile met en premier ce qui a l'air le plus petit.

**L'exemple a citer, parce qu'il est arrive ici** : au milieu de la traduction des messages
d'erreur, la skill `rust-conventions` — celle qui **gouverne** ces messages — prescrivait encore
« messages en minuscules, sans point final, en francais ». Je traduisais des chaines vers
l'anglais pendant que le document faisant autorite sur elles disait le contraire. Deux autres
faisaient de meme : `adr-format` et `doc-keeper`. La trouvaille initiale etait la meme un cran
plus tot : `test-writer.md` prescrivait des noms de tests en francais, avec des exemples
francais, juste apres que les 179 noms soient passes en anglais.

Le corollaire operationnel : **avant de commencer une conversion, chercher qui prescrit la regle
qu'on change.** `grep` sur les skills, les subagents et `AGENTS.md` coute une minute.


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

**`BUT_PAGER=cat` sur les sorties longues.** `but` ouvre `less` par defaut des que la sortie
depasse un ecran, ce qui bloque indefiniment dans un shell non interactif — un `but pull` a
ainsi tourne cinq minutes dans le vide avant d'etre tue. La forme sure :

```sh
BUT_PAGER=cat but pull
BUT_PAGER=cat but status
```

Les IDs de branche et de fichier sont **volatils** : ils changent a chaque
mutation de l'arbre. Un ID lu il y a trois commandes est un ID perime. `but status
--format json` avant chaque mutation, sans exception.

Le bloc ci-dessous est genere par `but agent setup` et fait autorite sur la
**syntaxe** des commandes. La regle ci-dessus fait autorite sur ce qui est
**interdit**. C'est le seul domicile de ce bloc dans le depot.

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
