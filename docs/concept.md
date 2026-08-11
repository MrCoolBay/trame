# Trame — orchestrateur d'agents de code, desktop macOS, local-first

> Document de synthèse. État : brainstorm consolidé, pas encore une spec figée.
> Révision 2 — ajout du multi-projet, du cadrage macOS et du modèle de licence.
> Révision 3 — bascule en open source : la section 8 est réécrite, le tableau des
> risques et la question ouverte 5 sont recalés. Voir l'ADR 0013.

**Nom de code** : `Trame` (le fil horizontal du tissage — plusieurs navettes, un seul tissu ; ça décrit littéralement le modèle). Alternatives : `Loom`, `Canut`.

---

## 1. Le pitch en une phrase

Une application desktop macOS écrite en Rust qui fait tourner plusieurs agents de code en parallèle, **par projet, dans un répertoire de travail unique par projet**, en attribuant automatiquement chaque modification à une branche virtuelle, et en rendant la coordination entre agents **explicite et observable** au lieu de silencieuse.

---

## 2. Le problème

L'orchestration multi-agents repose aujourd'hui sur deux modèles, bancals tous les deux pour le dev solo ou la petite équipe.

### Modèle worktree (Xirp, la plupart des outils)

Un worktree git par session. Isolation physique réelle, mais :

- duplication du workspace : réinstallation des dépendances, recompilation, reconfiguration par worktree ;
- N branches à relire et à faire atterrir séparément à la fin ;
- lourdeur disproportionnée à 3 sessions ;
- conçu pour l'échelle Spotify (1300 ingés), pas pour la nôtre.

### Modèle virtual branches (GitButler)

Un seul répertoire de travail, les changements sont **étiquetés** plutôt qu'isolés. Conceptuellement supérieur :

- pas de divergence dans le temps → **le conflit n'a pas d'endroit où naître** ;
- assignation au hunk en continu, pendant que le contexte est frais ;
- hunk locking : dépendances inter-branches détectées automatiquement ;
- restack automatique sur les piles de branches ;
- conflits *first-class* : le rebase réussit toujours, le conflit devient une donnée du graphe au lieu d'un état modal qui prend le repo en otage.

**Mais** : ce modèle échange un mode d'échec **bruyant** (git s'arrête, met des marqueurs, impossible de ne pas voir) contre un mode d'échec **silencieux** (dernier écrivain gagne, personne n'est prévenu). Excellent deal pour un humain seul. Mauvais deal pour N agents autonomes.

### La thèse

Le trou dans le marché n'est ni l'isolation ni le git. C'est **la coordination**.

> Garder les virtual branches — le modèle est le bon — et ajouter la couche qui rend les collisions bruyantes au lieu de silencieuses. Puis multiplier le parallélisme par les **projets** plutôt que par les sessions.

---

## 3. Principes de design (non négociables)

| # | Principe | Conséquence |
|---|---|---|
| 1 | **Desktop macOS uniquement** | Pas d'abstraction cross-platform. On exploite FSEvents, Keychain, launchd, APFS. |
| 2 | **Local-first** | Binaire unique. Pas de serveur, pas de compte, pas de cloud. Données en local. |
| 3 | **Répertoire de travail unique par projet** | Pas de worktree, pas de copy-on-write. |
| 4 | **Multi-projet dès l'architecture** | Le parallélisme s'obtient en ajoutant des projets, pas des sessions. |
| 5 | **2–5 sessions par projet** | Tout ce qui ne sert qu'au-delà est hors scope. |
| 6 | **ACP en premier, PTY en secours** | Les écritures sont interceptées *avant* le disque quand c'est possible. |
| 7 | **Observabilité totale** | Chaque écriture journalisée avec sa provenance. Auditabilité par construction. |
| 8 | **Silencieux quand c'est propre** | 95 % du trafic sans friction, sinon la feature est désactivée en une semaine. |
| 9 | **Pas un IDE** | Aucun éditeur embarqué. L'utilisateur garde Zed / VS Code. |

---

## 4. Le multi-projet : l'insight central

C'est la meilleure idée de la révision 2, et elle mérite d'être comprise pour ce qu'elle est.

**Deux sessions dans deux projets différents ne peuvent physiquement pas entrer en collision.** Répertoires de travail distincts, dépôts distincts, index distincts. L'isolation est gratuite et parfaite.

Conséquence directe :

```
5 projets × 3 sessions = 15 agents actifs
… sans jamais sortir du point de fonctionnement sûr (3 par working dir)
```

Là où Xirp achète le parallélisme en payant des worktrees, on l'achète en ajoutant des projets — ce que le développeur fait déjà naturellement (le back, le front, l'infra, le side project). **Le scaling se fait sur l'axe qui est déjà isolé.**

### La hiérarchie

```
Workspace (l'application)
 └── Project (un dossier + un dépôt git)
      ├── Working directory unique
      ├── Write Registry dédié
      ├── Branches virtuelles
      └── Session (un agent + un objectif)
```

### Ce qui est par projet vs global

| Par projet | Global (workspace) |
|---|---|
| Write Registry (un acteur par projet) | Journal SQLite unique (colonne `project_id`) |
| Compteur de séquence | Réservations de ressources (**ports, bases de dev**) |
| Working directory + backend VCS | Budget de concurrence (CPU, RAM) |
| Watcher FSEvents | Quotas et rate limits API (liés au compte, pas au projet) |
| Branches virtuelles, règles, config agent | Identifiants dans le Keychain |

Le point subtil : **les réservations de ressources doivent être globales**. Le port 3000 est machine-wide. Deux projets qui lancent chacun leur dev server, c'est le premier vrai conflit inter-projets — et c'est aussi ce qui justifie enfin un registre de ressources qui, en mono-projet, était marginal.

### À construire dès la v0.1

Même si l'UI n'affiche qu'un seul projet au départ, **le Supervisor et le registre par projet doivent exister dès le premier commit**. Rétrofitter un registre singleton global en registre par projet, c'est une réécriture, pas un refactor.

---

## 5. Architecture générale

```
┌────────────────────────────────────────────────────────────────┐
│  UI          v0 : TUI (ratatui)      v1 : Tauri + Nuxt         │
│  sidebar projets · grille sessions · diffs · timeline          │
└──────────────────────────────┬─────────────────────────────────┘
                               │ IPC local (UDS, JSON-RPC)
┌──────────────────────────────▼─────────────────────────────────┐
│  Core — daemon Rust / tokio (LaunchAgent launchd)              │
│                                                                │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  SUPERVISOR (acteur racine)                              │  │
│  │  HashMap<ProjectId, ProjectHandle>                        │  │
│  │  ├─ Resource Claims  (ports, DB, machine-wide)           │  │
│  │  ├─ Concurrency Budget (sémaphore global)                │  │
│  │  └─ Journal SQLite (partagé, append-only)                │  │
│  └───────────┬──────────────────────────┬───────────────────┘  │
│              │                          │                      │
│  ┌───────────▼────────────┐  ┌──────────▼─────────────┐        │
│  │ PROJECT « portailfcd » │  │ PROJECT « lyra-rp »    │  ...   │
│  │  ├─ Session Manager    │  │  ├─ Session Manager    │        │
│  │  ├─ Agent Transport ×N │  │  ├─ Agent Transport ×N │        │
│  │  ├─ WRITE REGISTRY     │  │  ├─ WRITE REGISTRY     │        │
│  │  ├─ VCS Layer          │  │  ├─ VCS Layer          │        │
│  │  └─ FSEvents Watcher   │  │  └─ FSEvents Watcher   │        │
│  └───────────┬────────────┘  └──────────┬─────────────┘        │
└──────────────┼──────────────────────────┼──────────────────────┘
               │                          │
     ┌─────────▼─────────┐      ┌─────────▼─────────┐
     │ ~/dev/portailfcd  │      │ ~/dev/lyra-rp     │
     │ working dir unique│      │ working dir unique│
     └───────────────────┘      └───────────────────┘
```

Le **core** est le produit. L'UI est interchangeable et arrive en second.

---

## 6. Les modules

### 6.1 Supervisor

Acteur racine. Possède la table des projets et les ressources partagées.

```rust
struct Supervisor {
    projects: HashMap<ProjectId, ProjectHandle>,
    claims: ResourceClaims,        // "port:3000" -> (ProjectId, SessionId)
    budget: Semaphore,             // sessions actives simultanées, tous projets
    journal: JournalHandle,
}

enum SupervisorMsg {
    AddProject { path: PathBuf, reply: oneshot::Sender<Result<ProjectId>> },
    OpenProject(ProjectId),
    CloseProject(ProjectId),       // drop watcher, sessions persistées
    RemoveProject(ProjectId),
    ClaimResource { resource: String, session: SessionId, reply: ... },
    ListAllSessions(oneshot::Sender<Vec<SessionSummary>>),
}
```

**Ajout d'un projet** = choisir un dossier, puis détection : est-ce un dépôt git ? un workspace GitButler existe-t-il ? quelle toolchain (`package.json`, `Cargo.toml`, `pyproject.toml`) ? La toolchain détermine ce qui constitue l'**état partagé** du projet (`node_modules` + ports, `target/`, `.venv`) et donc les ressources à réserver.

**Fermeture** ≠ suppression : on relâche le watcher et les backends, mais les sessions restent persistées et reprennent à la réouverture.

### 6.2 Session Manager (par projet)

```rust
struct Session {
    id: SessionId,
    project_id: ProjectId,
    name: String,
    harness: Harness,           // ClaudeCode | Codex | Gemini | Custom
    target_branch: BranchName,  // branche virtuelle assignée
    state: SessionState,
    created_at: Timestamp,
}

enum SessionState {
    Idle, Thinking, Writing,
    AwaitingPermission(PermissionRequest),
    Done, Failed(String),
}
```

- **Persistance** : une session survit au redémarrage de l'app *et* à la fermeture de l'UI (le daemon tourne sous launchd). Reprise sans perte d'état.
- **Sessions spéciales** : `session:human` (l'utilisateur dans son éditeur) et `session:external` (build, formatter, script) sont traitées exactement comme les autres. Ça unifie le modèle et supprime une catégorie entière de cas particuliers.

### 6.3 Agent Transport

Abstraction sur les harness. Le reste du core ne sait jamais si c'est de l'ACP ou du PTY.

```rust
trait AgentBackend {
    fn capabilities(&self) -> Capabilities;
    async fn send(&mut self, msg: UserMessage) -> Result<()>;
    fn events(&mut self) -> impl Stream<Item = AgentEvent>;
}

struct Capabilities {
    can_intercept_writes: bool,   // ACP: true, PTY: false
    can_inject_context: bool,
    can_request_permission: bool,
}

enum AgentEvent {
    Message(String),
    ToolCall { name: String, input: Value },
    FileWrite { path: PathBuf, content: String },  // ← interceptable
    PermissionRequest(PermissionRequest),
    Done,
    Error(String),
}
```

- `AcpBackend` — JSON-RPC sur stdio. Chemin privilégié : on voit les écritures avant qu'elles touchent le disque. C'est ce qui rend tout le reste possible.
- `PtyBackend` — pilotage de CLI via `portable-pty`. Mode dégradé : détection *a posteriori* via FSEvents, pas d'admission. **L'UI doit afficher la dégradation** plutôt que de laisser croire à une garantie qu'on n'a pas.

**Risque connu** : ACP est incomplet et inégal selon les harness (cf. `AskUserQuestion` indispo en plan mode). Le fallback n'est pas optionnel.

### 6.4 Write Registry — le cœur technique (un par projet)

**Ce n'est pas un système de locks.** Le locking pessimiste est inadapté : les agents tiennent leur transaction pendant des minutes, ne déclarent pas leur intention à l'avance, et bloquer un tool call en vol déclenche des timeouts côté harness.

Le modèle est celui des bases de données : **contrôle de concurrence optimiste avec validation du read-set**.

#### Pourquoi valider les lectures et pas seulement les écritures

Le mode d'échec le plus fréquent à 3 agents ne produit **aucune collision d'écriture** :

```
1. Agent A lit auth.rs, mémorise la signature de verify_token()
2. Agent B modifie auth.rs, renomme verify_token() → validate_token()
3. Agent A écrit handlers.rs, appelle verify_token()

→ Deux fichiers différents. Un verrou par fichier ne voit rien.
→ L'arbre est cassé.
```

Avec validation du read-set, au moment où A veut écrire on constate que `auth.rs` a changé depuis sa lecture. On ne sait pas *si* ça casse, mais on sait que **A raisonne sur un monde qui n'existe plus**. C'est l'invariant qui compte.

#### État maintenu

```rust
struct FileState {
    last_writer: SessionId,
    last_seq: u64,
    content_hash: Hash,           // blake3
    modified_regions: Vec<Range>, // v0.4+
    assigned_branch: BranchName,
}

struct SessionReadState {
    read_set:  HashMap<PathBuf, (Hash, Timestamp)>,
    write_set: HashSet<PathBuf>,
}

// par projet, pas global
seq_counter: u64  // ordre total local au projet
```

#### Quatre verdicts, pas un booléen

| Niveau | Situation | Réponse |
|---|---|---|
| **0 — Clean** | Aucun recouvrement | Admis, silencieux. ~95 % du trafic. |
| **1 — StaleRead** | Intersection sur le read-set uniquement | **Admis + on informe l'agent** : « `auth.rs` a changé depuis ta lecture, session B ». L'agent relit et s'adapte tout seul dans la grande majorité des cas. |
| **2 — DisjointWrite** | Même fichier, régions disjointes | Admis. Deux fonctions différentes d'un fichier de 2000 lignes, c'est légitime. Provenance enregistrée finement. |
| **3 — Overlap** | Régions qui se recouvrent | Bloqué → demande à l'humain, **via le mécanisme de permission ACP existant**. L'agent sait déjà attendre une permission, rien à lui apprendre. |

Le niveau 1 est le plus intéressant : la bonne réponse n'est pas de bloquer, c'est d'informer. Ce n'est possible que parce qu'on a un canal structuré vers l'agent.

#### Granularité — et où faire des compromis

- Fichier entier = trop grossier, on crie au loup en permanence.
- Ligne = trop fragile, les numéros dérivent à chaque édition au-dessus.
- **Hunks + quelques lignes de contexte** = le bon niveau. Difficulté : projeter les anciennes plages à travers les diffs successifs pour comparer dans un référentiel commun.

**Pour la v0 : ne pas faire ça.** Fichier entier + fenêtre temporelle (« deux écritures sur le même fichier à moins de 60 s ») = 90 % de la valeur pour 5 % du boulot. On raffine après avoir mesuré son propre taux de faux positifs.

Même logique pour le read-set : les agents lisent énormément (grep, glob, listings). Si on trace tout, le read-set explose et tout devient niveau 1. Filtrer sur les lectures substantielles uniquement, et faire décroître au-delà de ~10 min.

### 6.5 VCS Layer (par projet)

- Répertoire de travail unique, jamais de worktree.
- **L'attribution est déterministe** : chaque écriture admise porte son `session_id`, donc sa branche virtuelle. L'assignation hunk → branche n'est plus une heuristique, c'est une donnée. Trois agents finissent → trois branches déjà correctement remplies, zéro tri manuel.
- Deux backends derrière un trait `VcsBackend` :
  - `ButBackend` — shell-out vers la CLI `but`. Rapide à faire, on récupère l'oplog et le `but undo` gratuitement.
  - `GixBackend` — réimplémentation native sur `gitoxide`. Long terme, gros boulot, mais c'est la sortie propre côté licence.

### 6.6 Journal (global)

SQLite unique (`rusqlite`), append-only, dans `~/Library/Application Support/Trame/`.

```sql
projects(id, path, name, toolchain, added_at, last_opened_at)
sessions(id, project_id, name, harness, target_branch, state, created_at)
prompts(id, session_id, content, ts)
reads(id, project_id, session_id, path, hash, ts)
writes(id, project_id, session_id, seq, path, hash_before, hash_after, verdict, ts)
resource_claims(resource, project_id, session_id, claimed_at)

UNIQUE(project_id, seq)   -- la séquence est locale au projet
```

Base **globale, pas dans le repo** : ça ne pollue pas les projets, ça survit à leur suppression, et ça permet la timeline transverse (« qu'est-ce que j'ai fait cette semaine, tous projets confondus »).

**Ce module a de la valeur tout seul.** Même sans aucune détection de conflit, un outil qui répond à « qui a écrit cette ligne, dans quel projet, dans quelle session, en réponse à quel prompt » est immédiatement utile. C'est aussi l'angle auditabilité.

---

## 7. Cadrage macOS

Le choix mono-plateforme n'est pas qu'un renoncement, il débloque des choses.

### Ce qu'on gagne

| Sujet | Bénéfice |
|---|---|
| **FSEvents** | Watching récursif natif et efficace. Pas de limite de watches type inotify — ce qui compte quand on surveille 5 projets simultanément. |
| **Keychain** (`security-framework`) | Stockage propre des credentials agents. Pas de fichier de conf en clair. |
| **launchd LaunchAgent** | Le daemon survit à la fermeture de l'UI. Les sessions continuent, on rouvre la fenêtre plus tard. Structurant pour le multi-projet. |
| **Notifications natives** | Une session en attente de permission alerte sans que la fenêtre soit au premier plan. |
| **Item de barre de menus** | Compteur de sessions actives, tous projets. |
| **APFS** | Snapshots quasi gratuits si on veut de l'undo au-delà de l'oplog git. |
| **Une seule cible** | Pas de matrice CI, pas de conditionnels plateforme, pas de bugs Windows à distance. |

### Ce que ça coûte (à savoir avant de commencer)

- **Apple Developer Program (~99 €/an)** : sans signature + notarisation, Gatekeeper bloque l'app. Non négociable pour distribuer.
- **Pas de sandbox** → pas de Mac App Store. L'app a besoin d'un accès filesystem arbitraire et de spawner des process. Distribution directe (DMG) + **cask Homebrew**.
- **Mises à jour** : Sparkle ou l'updater Tauri, à câbler soi-même.
- **Permissions macOS** : accès aux dossiers utilisateur (TCC), à surveiller — l'UX de première ouverture doit être soignée.

### Ce que ça ne change pas

Le choix d'UI reste **Tauri v2 + Nuxt** (WKWebView). GPUI reste trop rugueux hors de Zed. Le « tout en Rust » s'applique au core, pas aux pixels — et si l'envie de porter ailleurs revient un jour, seul le core compte, il est déjà portable.

---

## 8. Licence : open source, MIT OR Apache-2.0

> **Révision 3.** Cette section disait le contraire : elle retenait FSL-1.1-MIT, une
> licence *source-available* avec clause de non-concurrence, et interdisait d'employer
> le terme « open source ». Ce choix est abandonné. L'historique du raisonnement est
> conservé dans l'ADR 0009, marqué remplacé par l'ADR 0013.

### Le choix

**MIT OR Apache-2.0**, au choix de l'utilisateur — la convention de l'écosystème Rust.

- Trame est **open source**, au sens OSI, sans guillemets ni précaution de vocabulaire.
- Tout est permis : usage, modification, fork, redistribution, y compris commerciale.
- Le dual laisse choisir : MIT pour la concision, Apache-2.0 pour la clause de brevet explicite que MIT n'a pas.
- `LICENSE-MIT` et `LICENSE-APACHE` à la racine. Le texte Apache est le fichier canonique d'`apache.org`, verbatim.

### Pourquoi la FSL n'a pas tenu

Le raisonnement initial était défensif : garder le code public tout en empêchant un acteur plus gros d'en faire un service concurrent. Trois objections, dont la troisième est la vraie :

1. **La protection était théorique.** La clause visait l'usage commercial concurrent. Trame est une application desktop locale : il n'y a pas de service à concurrencer. Ce qu'elle interdisait, personne n'allait le faire.
2. **Le coût était réel.** Licence non OSI ⇒ contributeurs découragés, empaquetage compliqué (Homebrew, nixpkgs), CLA nécessaire sur chaque contribution, et un point de vocabulaire à défendre dans chaque conversation publique.
3. **La protection ne vient pas de la licence.** Elle vient de l'exécution, de la marque, et du fait que le mécanisme de coordination est difficile à copier. Une clause ne protège pas une thèse produit.

### À prévoir

- **Pas de CLA.** Sous double licence permissive, une contribution est offerte sous les mêmes termes — c'est la convention explicite de l'écosystème Rust, rappelée dans le `README`.
- Une **marque déposée** sur le nom : c'est désormais le seul levier de protection, et c'était déjà le seul en pratique.
- Refuser une contribution proposée sous une troisième licence incompatible. Seul point de vigilance restant.

### Le point qui n'est PAS réglé par ce choix

**La licence de Trame ne donne aucun droit sur le code de GitButler**, et ne l'a jamais donné. C'était vrai sous FSL, ça reste vrai sous MIT/Apache : deux questions indépendantes. Ce qui porte l'analyse, c'est la **non-vendorisation** de `but` — voir la section risques.

Point nouveau, en revanche : sous licence permissive, un tiers peut redistribuer Trame commercialement. S'il empaquetait `but` avec, c'est **lui** qui se confronterait à la clause de GitButler. Raison de plus pour que `but` reste un prérequis documenté et jamais un binaire embarqué.

---

## 9. Stack

| Domaine | Choix | Note |
|---|---|---|
| Runtime | `tokio` | Chaque registre est **un acteur unique** : mpsc en entrée, oneshot en retour. Pas de `Mutex` partagé → sérialisation et ordre total par construction, par projet. |
| Git | `gix` (gitoxide) | Écosystème mûr ; GitButler a porté Git en Rust en passant toute la suite de tests du Git C. |
| Hash | `blake3` | Uniquement à l'admission et à la lecture, jamais l'arbre entier. |
| Stockage | `rusqlite` | Pas du JSONL : on voudra requêter en transverse projets. |
| PTY | `portable-pty` | Backend de secours. |
| Watcher | `notify` (FSEvents) | Un watcher par projet ouvert. Exclure `node_modules`, `target`, `.venv` via les règles `.gitignore`. |
| Keychain | `security-framework` | Credentials agents. |
| UI v0 | `ratatui` | Valide le modèle sans investir dans une UI qui va bouger dix fois. |
| UI v1 | Tauri v2 + Nuxt | WKWebView. Sidebar projets, grille de sessions, timeline. |
| Packaging | DMG signé + notarisé, cask Homebrew | Apple Developer Program requis. |

**Distribution** : `.app` bundle. Données dans `~/Library/Application Support/Trame/`. Aucun réseau sortant en dehors des agents eux-mêmes.

---

## 10. Roadmap

### v0.1 — « Le journal »
Daemon + TUI. **Supervisor et registre par projet en place dès le départ**, même si l'UI n'affiche qu'un projet. Sessions, transport ACP + PTY, point de sérialisation qui **ne fait que journaliser**. Zéro blocage, zéro friction, zéro risque.

### v0.2 — « L'attribution »
Provenance → assignation automatique des hunks aux branches virtuelles. **Livrable utile tel quel, tous les jours.**

### v0.3 — « Le multi-projet »
Ajout/fermeture/suppression de projets, détection de toolchain, réservations de ressources globales, vue transverse des sessions.

### v0.4 — « L'interface »
GUI Tauri. Sidebar projets, grille de sessions, diffs, timeline, item de barre de menus, notifications natives.

### v0.5 — « La coordination »
Read-set, notification niveau 1, puis recouvrement de régions et blocage niveau 3.

### v1 — Signature, notarisation, cask Homebrew, updater, docs.

> **Ne pas sauter à la v0.5.** Le risque produit n°1 est le taux de faux positifs : un outil qui crie au loup est désactivé dans la semaine. Rester en détection seule pendant un mois sur son propre workflow, mesurer combien de niveaux 2 et 3 passent réellement, *puis* décider de ce qui mérite un blocage.

---

## 11. Non-objectifs

- ❌ Windows et Linux (pour le moment)
- ❌ Un éditeur de code — ce n'est pas un IDE
- ❌ Un modèle ou un agent propriétaire — on orchestre l'existant
- ❌ Un SaaS, un compte, un backend
- ❌ 50 sessions dans un même projet / copy-on-write / scheduler distribué
- ❌ Un remplacement de git ou de la forge
- ❌ Le contexte organisationnel type Spotify Portal
- ❌ Le multi-utilisateur ou le partage de sessions

---

## 12. Risques identifiés

| Risque | Gravité | Mitigation |
|---|---|---|
| **Licence GitButler (FSL-1.1-MIT)** — non-concurrence sur les usages **commerciaux** | 🔴 Haute | Trois pistes, à faire valider par quelqu'un qui lit vraiment les licences : (1) traiter `but` comme dépendance externe installée par l'utilisateur, jamais vendorisée — comme Xirp ne fournit pas Claude Code ; (2) la clause vise les produits *commerciaux*, ce qui peut changer la lecture pour un projet gratuit ; (3) **la conversion FSL → MIT à deux ans** : les versions 2023-2024 du cœur virtual branches sont désormais sous MIT, donc réutilisables sans restriction. La piste 3 est la plus solide et la moins explorée. |
| **La licence de Trame ≠ résoudre le point ci-dessus** | 🔴 Haute | Deux questions indépendantes, quelle que soit la licence choisie pour Trame. Ne pas se rassurer à bon compte. Sous MIT/Apache, un tiers qui redistribuerait Trame **avec** `but` empaqueté se confronterait lui-même à la clause : ne jamais vendoriser. |
| **Trous dans ACP** | 🟠 Moyenne | Double transport dès le jour 1. Contribuer aux trous en amont plutôt que forker le protocole. |
| **Faux positifs du registre** | 🟠 Moyenne | Détection seule pendant un mois avant tout blocage. Granularité grossière en v0. |
| **Rétrofit du multi-projet** | 🟠 Moyenne | Supervisor + registre par projet dès le premier commit. C'est le seul choix architectural irréversible. |
| **Coût et friction de la distribution macOS** | 🟡 Faible | Apple Developer Program dès qu'on veut un testeur externe. À budgéter, pas à découvrir. |
| **Écritures hors-bande** (`sed -i`, hooks, build) | 🟡 Faible | Rattrapées par FSEvents, mais pas admises. Assumer et afficher. |
| **Interférence sémantique sans recouvrement de lecture** | 🟡 Faible | Aucun registre ne peut l'attraper. Seul filet réel : compilateur + tests. Piste : détecteur de quiescence — quand toutes les sessions d'un projet sont idle depuis X secondes, lancer le check. Donne un point de synchronisation naturel et règle en partie le problème du build partagé. |
| **Ressources partagées inter-projets** (ports, dev DB) | 🟡 Faible | Réservations déclaratives globales au niveau du Supervisor. |
| **Scope creep** | 🔴 Haute | La section 11 existe pour ça. |

---

## 13. Questions ouvertes

1. **Quel harness en premier ?** Détermine si on peut vraiment intercepter avant admission ou si on démarre en détection seule.
2. `but` CLI ou `gix` natif pour la v0.2 ? Et la piste « versions converties en MIT » change-t-elle l'arbitrage ?
3. Le niveau 1 informe-t-il l'agent automatiquement, ou l'humain décide-t-il ?
4. Un projet peut-il avoir plusieurs dépôts (monorepo vs multi-repo lié) ? Ou un projet = un dépôt, strictement ?
5. Positionnement : outil perso, ou produit avec un angle auditabilité / souveraineté pour le marché européen ? La licence est tranchée (open source, MIT OR Apache-2.0 — ADR 0013), le positionnement ne l'est pas.
