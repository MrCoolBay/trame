# Trame — orchestrateur d'agents de code, desktop macOS, local-first

> **Révision 4.** Ce document décrit **ce qui existe**, pas ce qu'on imaginait au départ. Il a
> déjà divergé une fois — la roadmap plaçait le read-set en v0.5 alors qu'il est le livrable
> de la v0.1 — et cette révision existe pour que ça ne se reproduise pas.
>
> C'est la **source de vérité pour toute session future**. Les décisions et leurs raisons
> vivent dans [`adr/`](adr/) ; ce document dit où on en est et pourquoi. Quand il diverge du
> code, c'est lui qu'il faut corriger, immédiatement.
>
> Révisions précédentes : 2 (multi-projet, cadrage macOS, licence) · 3 (bascule open source).

**Nom de code** : `Trame` — le fil horizontal du tissage : plusieurs navettes, un seul tissu.

---

## 1. Le pitch en une phrase

Une application desktop macOS écrite en Rust qui fait tourner plusieurs agents de code en
parallèle, **par projet, dans un répertoire de travail unique par projet**, en attribuant
chaque modification à une branche virtuelle, et en rendant la coordination entre agents
**explicite et observable** au lieu de silencieuse.

---

## 2. Le problème

L'orchestration multi-agents repose sur deux modèles, bancals tous les deux pour le dev solo
ou la petite équipe.

### Modèle worktree (Conductor, Xirp, Crystal, la plupart des outils)

Un worktree git par session. Isolation physique réelle, mais duplication du workspace,
N branches à faire atterrir séparément, et une lourdeur disproportionnée à trois sessions.

Surtout : **l'isolation ne supprime pas seulement les collisions, elle supprime la
possibilité de coordonner.** Deux agents dans deux worktrees sont aveugles l'un à l'autre
*par construction*, et aucune couche ajoutée par-dessus n'y remédie.

### Modèle virtual branches (GitButler)

Un seul répertoire de travail, les changements sont **étiquetés** plutôt qu'isolés. Pas de
divergence dans le temps, donc **le conflit n'a pas d'endroit où naître**. Conceptuellement
supérieur.

**Mais** ce modèle échange un mode d'échec **bruyant** (git s'arrête, met des marqueurs)
contre un mode d'échec **silencieux** (dernier écrivain gagne, personne n'est prévenu).
Excellent deal pour un humain seul. Mauvais deal pour N agents autonomes.

### La thèse

> Garder les virtual branches — le modèle est le bon — et ajouter la couche qui rend les
> collisions bruyantes. Puis multiplier le parallélisme par les **projets** plutôt que par
> les sessions.

Le mode d'échec à attraper ne produit **aucune collision d'écriture** :

```
1. Agent A lit auth.rs, mémorise la signature de verify_token()
2. Agent B modifie auth.rs, renomme verify_token() → validate_token()
3. Agent A écrit handlers.rs, appelle verify_token()

→ Deux fichiers différents. Un verrou par fichier ne voit rien.
→ L'arbre est cassé.
```

---

## 3. Principes de design (non négociables)

| # | Principe | Conséquence |
|---|---|---|
| 1 | **Desktop macOS uniquement** | Pas d'abstraction cross-platform. FSEvents, Keychain, launchd, APFS. |
| 2 | **Local-first** | Binaire unique. Pas de serveur, pas de compte, pas de cloud. |
| 3 | **Répertoire de travail unique par projet** | Pas de worktree, pas de copy-on-write. |
| 4 | **Multi-projet dès l'architecture** | Le parallélisme s'obtient en ajoutant des projets. |
| 5 | **2–5 sessions par projet** | Tout ce qui ne sert qu'au-delà est hors scope. |
| 6 | **ACP en premier, PTY en secours** | Les écritures sont interceptées *avant* le disque quand c'est possible. |
| 7 | **Observabilité totale** | Chaque écriture journalisée avec sa provenance **et son origine**. |
| 8 | **Silencieux quand c'est propre** | ~95 % du trafic sans friction, sinon la feature est désactivée en une semaine. |
| 9 | **Pas un IDE** | Aucun éditeur embarqué. |
| 10 | **Un trou nommé vaut mieux qu'un trou ignoré** | Ajouté en révision 4. Voir §6.7. |

---

## 4. Le multi-projet : l'insight central

**Deux sessions dans deux projets différents ne peuvent physiquement pas entrer en
collision.** Répertoires, dépôts et index distincts. L'isolation est gratuite et parfaite.

```
5 projets × 3 sessions = 15 agents actifs
… sans jamais sortir du point de fonctionnement sûr (3 par working dir)
```

### La hiérarchie

```
Workspace (l'application)
 └── Project (un dossier + un dépôt git)
      ├── Working directory unique
      ├── Write Registry dédié
      ├── Watcher FSEvents dédié
      ├── Branches virtuelles
      └── Session (un agent + un objectif)
```

### Ce qui est par projet vs global

| Par projet | Global (workspace) |
|---|---|
| Write Registry (un acteur) | Journal SQLite unique (colonne `project_id`) |
| **Compteur de séquence** | Réservations de ressources (**ports, bases de dev**) |
| Working directory + backend VCS | Budget de concurrence (CPU, RAM) |
| Watcher FSEvents | Quotas et rate limits API |
| Branches virtuelles, config agent | Identifiants dans le Keychain |

Le point subtil : **les réservations de ressources doivent être globales.** Le port 3000 est
machine-wide. C'est le premier vrai conflit inter-projets.

C'est **le seul choix architectural irréversible** : le Supervisor et le registre par projet
existent depuis le premier commit ([ADR 0010](adr/0010-parallelisme-par-projets.md)).

---

## 5. Architecture

```
┌────────────────────────────────────────────────────────────────┐
│  UI          v0 : TUI (ratatui)      v1 : GUI (gpui-ce)         │
└──────────────────────────────┬─────────────────────────────────┘
                               │ Receiver<Observation> — a sens unique
┌──────────────────────────────▼─────────────────────────────────┐
│  Core — daemon Rust / tokio                                    │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  SUPERVISOR (acteur racine)                              │  │
│  │  ├─ Resource Claims  ├─ Concurrency Budget  ├─ Journal   │  │
│  └───────────┬──────────────────────────┬───────────────────┘  │
│  ┌───────────▼────────────┐  ┌──────────▼─────────────┐        │
│  │ PROJECT « portailfcd » │  │ PROJECT « lyra-rp »    │  ...   │
│  │  ├─ SessionPilot ×N    │  │  ├─ SessionPilot ×N    │        │
│  │  ├─ Agent Transport ×N │  │  ├─ Agent Transport ×N │        │
│  │  ├─ WRITE REGISTRY     │  │  ├─ WRITE REGISTRY     │        │
│  │  ├─ FSEvents Watcher   │  │  ├─ FSEvents Watcher   │        │
│  │  └─ VCS Layer          │  │  └─ VCS Layer          │        │
│  └────────────────────────┘  └────────────────────────┘        │
└────────────────────────────────────────────────────────────────┘
```

Le **core** est le produit. L'UI est interchangeable et arrive en second — et ce n'est pas une
formule : c'est ce qui autorise à parier sur un framework pré-1.0
([ADR 0022](adr/0022-decoupage-daemon-gui.md) et [0023](adr/0023-gpui-ce-pour-la-gui.md)). Une
interface ne reçoit **qu'un `Receiver<Observation>`**, jamais un `RegistryHandle` : « elle
observe, elle ne pilote pas » est dans le typage.

L'IPC local (UDS, JSON-RPC) de l'esquisse initiale n'existe pas et n'est pas nécessaire : les
deux vivent dans le même binaire. Le canal préfigure la frontière sans la coûter — il se
remplace par un socket sans changer la forme du code de l'interface.

### Les crates, et ce qu'elles contiennent réellement

```
crates/
├── trame-core/      ids · hash · clock · paths(ProjectRoot) · verdict
│                    project · session · prompt · notice · task_source · forge
├── trame-journal/   schema · records · store · actor
├── trame-registry/  state (★ la logique d'admission) · actor · msg
├── trame-agent/     backend · event · jsonrpc · acp · pty
├── trame-vcs/       (encore vide : constantes seulement)
├── trame-daemon/    session (SessionPilot) · watcher (FSEvents) · observe (canal UI)
└── trame-view/      state (état d'affichage pur) · source (journal+registre+watcher réels)
apps/
├── trame-tui/       run (boucle) · ui (rendu ratatui)
└── trame-gui/       vue (rendu gpui) · theme (couleurs et marqueurs)
```

Direction de dépendance unique, jamais violée :
`core ← journal ← registry ← {agent, vcs} ← daemon ← view ← {tui, gui}`.

---

## 6. Les modules

### 6.1 Supervisor

**Pas encore écrit.** Les frontières existent, la table des projets et les claims non. Le
cadrage est dans [ADR 0010](adr/0010-parallelisme-par-projets.md).

### 6.2 Session Manager

`SessionPilot` (`trame-daemon`) pilote une session : il consomme le flux de l'agent, parle au
registre, et pose l'avis devant le prochain message. La persistance des sessions et la reprise
après redémarrage ne sont pas faites.

**Sessions spéciales** : `SessionId::EXTERNAL` existe et sert aux écritures hors-bande (§6.5).
Une session `human` viendra sur le même modèle.

### 6.3 Agent Transport

```rust
#[async_trait]
pub trait AgentBackend: Send {
    fn capabilities(&self) -> Capabilities;
    async fn send(&mut self, msg: UserMessage) -> Result<(), AgentError>;
    fn events(&mut self) -> Option<AgentEventStream>;
    async fn shutdown(&mut self) -> Result<(), AgentError>;
}
```

`AcpBackend` fonctionne, une seule cible : **Claude Code**. `PtyBackend` est un squelette
`todo!()` dont la seule méthode réelle est `capabilities()` — et c'est la plus importante,
puisqu'elle annonce sa dégradation.

#### L'inversion qui rend le produit possible

**En ACP, Trame est le client et l'agent est le serveur.** Ce n'est pas l'agent qui écrit
puis nous prévient : c'est l'agent qui *demande* à Trame d'écrire, par `fs/write_text_file`.
Le point d'interception n'est pas un hook à installer, c'est le chemin normal du protocole.

Mieux : **annoncer `fs.writeTextFile` fait retirer les outils `Write` et `Edit` natifs** de
l'agent. Il ne *peut plus* écrire lui-même.
[ADR 0016](adr/0016-interception-avant-disque-validee.md) — validé en live : deux sessions
réelles ont demandé à écrire, nous avons refusé, rien n'a atteint le disque.

#### Trois choses apprises que la documentation ne dit pas

1. **Il n'existe aucun `sessionUpdate` de fin de tour.** La fin de tour est la **réponse à
   `session/prompt`**, avec son `stopReason`. Attendre une notification « end_of_turn » est une
   attente qui n'aboutit jamais — ça a coûté une manche expérimentale.
2. **`tool_call_update` arrive parfois sans `tool_call`.** Ne traduire que la forme initiale
   laisse des appels d'outil invisibles.
3. **Les chemins arrivent absolus et résolus.** Racine `/var/…` → l'agent répond
   `/private/var/…`. D'où `trame_core::ProjectRoot`, par lequel toute clé de fichier passe.

#### L'adaptateur est épinglé, et c'est un problème connu

`@zed-industries/claude-code-acp` **0.16.2**, déprécié. Le successeur
`@agentclientprotocol/claude-agent-acp` **ne retire plus `Write` ni `Edit`** : mesuré, pas
supposé.

```
0.16.2 : --disallowedTools AskUserQuestion,Read,Write,Edit  → interception possible
0.66.0 : --disallowedTools AskUserQuestion --tools default  → interception perdue
```

Migrer supprimerait le mécanisme central **en silence**. Un canari surveille ce comportement
tiers non spécifié à chaque `just ci`.
[ADR 0017](adr/0017-adaptateur-acp-epingle.md) regarde le coût en face et liste quatre
sorties, dont les hooks `PreToolUse` — la piste la moins explorée.

### 6.4 Write Registry — le cœur technique (un par projet)

**Ce n'est pas un système de locks.** Le locking pessimiste est inadapté : les agents tiennent
leur transaction plusieurs minutes, ne déclarent pas leur intention à l'avance, et bloquer un
tool call en vol déclenche des timeouts côté harness.

Le modèle est celui des bases de données : **concurrence optimiste avec validation du
read-set** ([ADR 0007](adr/0007-concurrence-optimiste-read-set.md)).

#### Le registre écrit, il ne rend pas qu'un verdict

`admit` **évalue, écrit, puis enregistre** — dans cet ordre, dans le même acteur
([ADR 0014](adr/0014-le-registre-ecrit-sur-disque.md)). Un invariant qui repose sur la
discipline de l'appelant n'est pas un invariant.

L'état n'est mis à jour **qu'après le succès du disque** : sinon le registre croirait le
fichier modifié et périmerait à tort les lectures des autres sessions.

#### Quatre verdicts, pas un booléen

| Niveau | Situation | Réponse | Statut |
|---|---|---|---|
| **0 — Clean** | Aucun recouvrement | Admis, silencieux. ~95 % du trafic. | ✅ |
| **1 — StaleRead** | Intersection sur le read-set | **Admis, et on informe l'agent.** | ✅ |
| **2 — DisjointWrite** | Même fichier, régions disjointes | Admis. | ⏳ v0.4 |
| **3 — Overlap** | Régions qui se recouvrent | Bloqué → demande à l'humain. | ⏳ v0.4 |

Les niveaux 2 et 3 **ne sont jamais produits** : à granularité fichier entier
([ADR 0012](adr/0012-granularite-fichier-en-v0-1.md)), ils sont indistinguables. Les variantes
existent pour que les ajouter soit un `match` à compléter.

**Rien n'est bloqué en v0.1.** Le registre observe, journalise et informe.

#### L'avis, et la mesure qui a tranché sa forme

`StaleFile` porte le chemin, l'auteur, les instants et la séquence — **et pas de résumé du
changement**. Le registre ne calcule **aucun diff** à l'admission.

Trois formulations ont été mesurées sur de vraies sessions, cinq runs chacune :

| variante | relit le fichier | bon nom | sur-écriture |
|---|---|---|---|
| **neutre** | 5/5 | 5/5 | 0/5 |
| directive | 5/5 | 5/5 | 0/5 |
| contextuelle (avec résumé) | 5/5 | 5/5 | 0/5 |

La neutre fait aussi bien, coûte le moins, et n'ordonne rien. L'hypothèse « l'agent ne suivra
l'avis que s'il sait *ce qui* a changé » est **réfutée**
([ADR 0018](adr/0018-pas-de-diff-dans-stalefile.md)).

**La mesure a précédé la dépense**, et c'est le point de méthode à retenir. Elle a aussi une
**dette de validation** explicite : scénario de trois tours, `Grep`/`Glob`/`Bash` fermés, peu
de contexte accumulé, un identifiant renommé. Le `15/15` signale un test qui **ne discrimine
plus**. Si une de ces limites saute, la question se rouvre légitimement.

#### État maintenu

```rust
struct FileState {
    last_writer: SessionId,      // ou SessionId::EXTERNAL
    last_seq: Seq,
    content_hash: ContentHash,   // blake3
    written_at: Timestamp,
    // modified_regions: Vec<Range> → v0.4
}

struct SessionState {
    name: String,
    read_set: HashMap<PathBuf, (ContentHash, Timestamp)>,  // TTL 10 min
    write_set: Vec<PathBuf>,
}
```

Filtrage du read-set : seules les lectures **substantielles** (`ReadKind::FullFile`). Les hits
de grep et les listings n'entrent pas — sinon le read-set explose et tout devient niveau 1.

### 6.5 Le watcher FSEvents — passé du confort à l'exigence

**Ce n'était pas prévu comme ça.** La révision 2 le listait comme un filet de confort pour
« assumer et afficher » les écritures hors-bande. C'est faux : c'est une **exigence de
correction**.

Une session *peut* écrire hors admission — `sed -i` dans un `Bash`, un hook git, un build,
l'utilisateur dans son éditeur. Sans watcher :

```
A lit auth.rs                  → read-set : hash v1
B fait `sed -i` sur auth.rs    → le disque a v2, le registre croit encore v1
A écrit handlers.rs            → Clean, alors qu'il devrait être StaleRead
```

Le problème n'est pas la couverture du journal : **le registre devient faux**, et le mécanisme
central échoue **silencieusement**. L'outil a l'air de fonctionner et ne fait rien.

`RegistryMsg::ObserveExternalWrite` répare ça. Trois propriétés :

- **Il n'empêche rien.** Quand FSEvents notifie, le fichier est écrit. Il n'y a pas de verdict
  à rendre. Le watcher rattrape l'état pour que les *prochaines* admissions soient justes.
- **Pas de double comptage.** Le registre écrit lui-même, donc FSEvents voit aussi ses propres
  écritures. Règle : *une observation dont l'empreinte est déjà celle connue est un écho, pas
  un événement.* Pas d'horodatage, pas de fenêtre de tolérance, pas de course. Traite
  gratuitement le formatter qui réécrit à l'identique.
- **Le bruit reste dehors.** Filtre sur les règles `.gitignore` du projet plus une liste
  d'exclusions en dur. Un `cargo build` ne noie pas le registre.

Ces écritures sont attribuées à `SessionId::EXTERNAL`, nommées « hors-bande » dans l'avis, et
journalisées avec `origin = observed` **sans verdict** — personne ne les a admises.

### 6.6 Journal (global)

SQLite unique (`rusqlite`), **append-only**, dans `~/Library/Application Support/Trame/`.
Base globale, jamais dans le dépôt : ça ne pollue pas les projets, ça survit à leur
suppression, et ça permet la timeline transverse
([ADR 0008](adr/0008-journal-sqlite-append-only.md)).

**Le schéma réel**, à jour :

```sql
projects(id, path, name, toolchain, added_at, last_opened_at)
sessions(id, project_id, name, harness, target_branch, work_item, initial_state, created_at)
prompts(id, session_id, content, ts)
reads(id, project_id, session_id, path, hash, ts)
writes(id, project_id, session_id, session_name, seq, path,
       hash_before, hash_after, verdict, origin, ts)
resource_claims(id, resource, project_id, session_id, claimed_at)

UNIQUE(project_id, seq)   -- la séquence est locale au projet
```

Quatre choix qui ne sont pas dans la version d'origine :

- **`initial_state`** et non `state`. Dans une table append-only, une colonne `state` serait
  lue comme un état courant et mentirait dès la première transition. Les transitions
  demanderont une table d'événements.
- **`session_name` dénormalisé** dans `writes`. Une ligne d'audit doit se lire seule, sans
  jointure, et survivre à la disparition de la session.
- **`origin`** — `admitted` ou `observed`. Confondre les deux rendrait le journal faux sur le
  seul point qui compte, la provenance.
- **`verdict` nullable** — `NULL` pour une écriture observée. Personne ne l'a admise, donc
  aucun verdict n'existe ; mettre une valeur serait un mensonge.

**Ce module a de la valeur tout seul.** Même sans détection de conflit, répondre à « qui a
écrit cette ligne, dans quel projet, dans quelle session, en réponse à quel prompt » est
immédiatement utile.

### 6.7 ★ La portée réelle de l'invariant, et les deux trous

C'est la section la plus importante de cette révision, parce que c'est celle qui manquait.

> **Le registre est le point de passage unique des écritures d'agent faites par les outils de
> fichiers — `Write`, `Edit`, `NotebookEdit`.**
>
> **Le read-set ne contient que les lectures faites par l'outil de lecture ACP.**

Ni plus, ni moins. C'est cette phrase-là qui doit être affichée à l'utilisateur.

#### Trou n° 1 — l'écriture par le shell

`Bash`, `BashOutput` et `KillShell` **restent disponibles** : ils ne sont retirés que si le
client annonce la capacité `terminal`, ce que Trame ne fait pas. Un `echo > fichier` échappe
donc à l'admission. **Mesuré** sur la ligne de commande réelle, et **confirmé** par sonde.

Atténué, pas fermé : le watcher FSEvents rattrape l'état (§6.5). Le journal porte la ligne
avec `origin = observed`, sans verdict.

#### Trou n° 2 — la lecture par un autre outil, et c'est le pire

Retirer `Read` **ne force pas** l'agent à passer par nous : `Grep`, `Glob` et `Bash` restent
disponibles. Un agent qui lit par l'un d'eux **n'entre pas dans le read-set**.

**Une lecture qui échappe est plus grave qu'une écriture qui échappe.** Une écriture manquante
laisse un trou dans le journal ; une lecture manquante supprime la **condition** d'un
`StaleRead` — le mécanisme central ne se déclenche pas, et rien ne l'indique.

Aucune atténuation **implémentée** aujourd'hui. La manche expérimentale a dû **fermer** `Grep`,
`Glob` et `Bash` pour mesurer quoi que ce soit : acceptable pour une expérience, **pas pour un
produit**. Un agent privé de recherche sur un vrai codebase est un agent dégradé.

**La voie de sortie est mesurée, elle n'est pas encore construite**
([sonde 3](sondes/2026-08-12-postooluse.md)). `PostToolUse` porte
`tool_response.filenames` — la liste des fichiers effectivement lus — en mode
`files_with_matches` et pour `Glob`, et ne se déclenche **ni** sur un appel refusé **ni** sur un
appel en échec : le read-set alimenté par là ne peut pas contenir de lecture fantôme. **Fermer
`Grep` et `Glob` n'est donc pas nécessaire.**

Deux décisions encadrent cette voie avant qu'elle soit construite. L'empreinte ne vient **que**
de `fs/read_text_file`, jamais du payload d'un hook — la CLI y injecte un `<system-reminder>`
([ADR 0020](adr/0020-empreinte-uniquement-depuis-fs-read-text-file.md), invariant n° 10). Et le
mode `content` de `Grep`, où les chemins n'existent que dans une chaîne de sortie, devient un
**troisième angle mort nommé et compté** plutôt qu'un cas à reconstruire
([ADR 0021](adr/0021-pas-d-analyse-de-la-sortie-de-grep.md)) — avec une atténuation réelle : un
agent qui veut le contexte autour des lignes trouvées **ouvre le fichier**, donc repasse par
`fs/read_text_file`.

Avec le refus des écritures `Bash` en `PreToolUse`, la même piste couvre aussi le trou n° 1 et
la dépendance à l'adaptateur déprécié — trois problèmes d'un coup.

#### Ce qu'aucun registre ne peut attraper

L'interférence sémantique **sans recouvrement de lecture** : A et B se contredisent sans avoir
lu le même fichier. Seul filet réel : le compilateur et les tests. Piste : détecteur de
quiescence.

### 6.8 VCS Layer

**Encore vide.** Deux constantes. Le cadrage tient :

- Répertoire de travail unique, jamais de worktree.
- **L'attribution est déterministe** : chaque écriture admise porte son `session_id`, donc sa
  branche. Ce n'est plus une heuristique, c'est une donnée.
- `ButBackend` en shell-out, `but ... --format json` systématiquement
  ([ADR 0003](adr/0003-gitbutler-en-shell-out.md), [ADR 0004](adr/0004-parsing-json-du-vcs.md)).
  Attention : `--format json`, **pas** `--json`, qui n'existe pas.

---

## 7. Cadrage macOS

Inchangé depuis la révision 2. Ce qu'on gagne : FSEvents (**désormais utilisé pour de vrai**,
§6.5), Keychain, launchd, notifications natives, item de barre de menus, APFS, une seule cible
CI. Ce que ça coûte : Apple Developer Program (~99 €/an), pas de Mac App Store, updater à
câbler, TCC à soigner. [ADR 0001](adr/0001-macos-uniquement.md).

---

## 8. Licence : open source, MIT OR Apache-2.0

**MIT OR Apache-2.0**, au choix de l'utilisateur — convention de l'écosystème Rust. Trame est
open source au sens OSI, sans précaution de vocabulaire. Pas de CLA : une contribution est
offerte sous les mêmes termes.

La révision 2 retenait FSL-1.1-MIT et interdisait le terme « open source ». Ce choix est
abandonné : la protection était théorique (une app desktop locale n'a pas de service à
concurrencer), le coût réel (licence non OSI, empaquetage compliqué, CLA sur chaque
contribution), et la protection ne vient pas de la licence mais de l'exécution et de la
marque. [ADR 0013](adr/0013-licence-open-source-mit-apache.md), qui remplace l'ADR 0009.

**Ce que ça ne règle pas** : la licence de Trame ne donne aucun droit sur le code de
GitButler. `but` reste un **prérequis externe installé par l'utilisateur, jamais vendorisé** —
c'est cette non-inclusion qui porte l'analyse.

---

## 9. Stack

| Domaine | Choix | État |
|---|---|---|
| Runtime | `tokio`, un acteur par domaine | ✅ registre, journal |
| Hash | `blake3`, à l'admission et à la lecture seulement | ✅ |
| Stockage | `rusqlite`, append-only | ✅ six tables |
| Transport agent | JSON-RPC sur stdio, ACP | ✅ `AcpBackend` |
| Watcher | `notify` (FSEvents) | ✅ avec filtre `ignore` (gitignore) |
| PTY | `portable-pty` | ⏳ squelette `todo!()` |
| Git | CLI `but` en shell-out | ⏳ constantes seulement |
| Keychain | `security-framework` | ⏳ pas commencé |
| UI v0 | `ratatui` | ✅ panneaux, flux, verdicts, dégradation |
| UI v1 | `gpui-ce` **épinglé** 0.3.3, importé sous le nom `gpui` | ✅ `apps/trame-gui` ([ADR 0023](adr/0023-gpui-ce-pour-la-gui.md)) |
| Sortie de secours UI | Tauri v2 + **Vue** — pas Nuxt : routing et SSR inutiles sur du mono-fenêtre | ⏳ si `gpui-ce` déçoit |

Aucun `unsafe`, `unsafe_code = "forbid"` au niveau du workspace.

---

## 10. Roadmap — corrigée

> **La roadmap d'origine plaçait le read-set en v0.5.** C'était faux : c'est le livrable de
> la v0.1, et c'est même la seule chose qui distingue Trame. Cette section est la version
> juste.

| Phase | Contenu | État |
|---|---|---|
| **0** | Outillage, frontières de crates, coutures, ADR, skills | ✅ |
| **1** | `trame-journal` + `trame-registry`. Scénario canonique testable sans agent | ✅ |
| **2** | `trame-agent`, ACP, interception validée en live | ✅ |
| **3.1** | Le registre écrit après admission | ✅ |
| **3.2** | Chaîne complète : `FileRead` → read-set, `FileWrite` → admission → avis | ✅ |
| **3.3** | Manche expérimentale sur la forme de l'avis | ✅ tranchée |
| **3.4** | Watcher FSEvents — **remonté avant la TUI** | ✅ |
| **3.5** | TUI ratatui minimal | ✅ |
| **4.0** | Sonde `gpui-ce` : fenêtre, `Receiver` tokio, liste qui défile | ✅ [sonde 4](sondes/2026-08-12-gpui-ce.md) |
| **4.1** | `apps/trame-gui` — même périmètre d'affichage que la TUI | ✅ |
| **v0.2** | Attribution → assignation des hunks aux branches virtuelles | ⏳ |
| **v0.3** | Multi-projet : Supervisor, toolchain, claims de ressources | ⏳ |
| **v0.4** | Hunks : `DisjointWrite` et `Overlap`, blocage du niveau 3 | ⏳ |
| **v1** | Signature, notarisation, cask Homebrew, updater | ⏳ |

> **Ne pas sauter au blocage.** Le risque produit n° 1 reste le taux de faux positifs. Rester
> en détection seule sur son propre workflow, mesurer, *puis* décider de ce qui mérite un
> blocage.

**118 tests**, déterministes, sans un `sleep` — sauf les trois tests FSEvents, qui attendent
une condition par interrogation bornée parce que le système notifie quand il notifie.

---

## 11. Non-objectifs

- ❌ Windows et Linux
- ❌ Un éditeur de code — ce n'est pas un IDE
- ❌ Un modèle ou un agent propriétaire
- ❌ Un SaaS, un compte, un backend, le multi-utilisateur
- ❌ 50 sessions dans un même projet / copy-on-write / scheduler distribué
- ❌ Un remplacement de git ou de la forge
- ❌ **Toute forme d'isolation**

---

## 12. Risques — mis à jour par la mesure

| Risque | Gravité | État |
|---|---|---|
| **Trou lecture** (`Grep`/`Glob`/`Bash`) | 🟡 Moyenne | **Ouvert, mais la sortie est mesurée** (sonde 3). `PostToolUse` rend les fichiers lus, sans lecture fantôme. Reste : l'implémenter, et arbitrer le mode `content` de `Grep`. Un `Bash` de lecture reste non couvert. |
| **Adaptateur ACP déprécié** | 🔴 Haute | **Épinglé à 0.16.2**, canari en place. Le successeur casse l'interception. Sursis, pas solution. |
| **Licence GitButler (FSL)** | 🔴 Haute | Ouvert. `but` non vendorisé ; la piste la plus solide reste la conversion FSL→MIT à deux ans. |
| **Scope creep** | 🔴 Haute | La section 11 existe pour ça. |
| **Faux positifs du registre** | 🟠 Moyenne | Pas encore mesuré sur usage réel. Deux cadrans avant de payer les hunks : filtre du read-set, TTL. |
| **Trou écriture par `Bash`** | 🟠 Moyenne | **Atténué** par le watcher : le registre ne devient plus faux. Non admis, non empêché. |
| **Dette de validation de la manche** | 🟠 Moyenne | 15/15 dans des conditions étroites. Rejeu nécessaire ; chaque limite est un déclencheur. |
| **Trous dans ACP** | 🟠 Moyenne | Trois comportements non documentés découverts. Canari + double vérification des tiers. |
| **Rétrofit du multi-projet** | 🟢 Réglé | Registre par projet depuis le premier commit. |
| **Interférence sémantique** | 🟡 Faible | Aucun registre ne peut l'attraper. Filet : compilateur + tests. |
| **Ressources inter-projets** | 🟡 Faible | Réservations globales au Supervisor. Pas encore écrit. |
| **Distribution macOS** | 🟡 Faible | À budgéter, pas à découvrir. |

---

## 13. Questions ouvertes

Les questions tranchées sont retirées d'ici et vivent dans leur ADR. Restent :

1. **Le trou lecture.** **Sondé trois fois** — contrat, session réelle, puis résultats d'outils :
   [`pretooluse`](sondes/2026-08-12-pretooluse.md),
   [`pretooluse-live`](sondes/2026-08-12-pretooluse-live.md),
   [`postooluse`](sondes/2026-08-12-postooluse.md).
   Établi : le hook se déclenche, un `deny` bloque réellement, le motif atteint l'agent qui se
   rabat **sur le chemin admis**, le fichier de réglages vit **hors du projet** via
   `extraArgs.settings`, et `PostToolUse` rend les fichiers lus sans jamais se déclencher sur un
   appel refusé ou en échec. Coût total ~11,7 ms par appel d'outil, deux processus.
   Direction retenue, **non implémentée** : **refuser** les commandes shell qui écrivent, ce qui
   ramène le trou dans le périmètre de l'admission au lieu de le modéliser — et **enregistrer**
   ce que `Grep` et `Glob` lisent plutôt que de les refuser.
   **Tranché depuis** : l'empreinte ne vient que de `fs/read_text_file`
   ([ADR 0020](adr/0020-empreinte-uniquement-depuis-fs-read-text-file.md)), et le mode `content`
   de `Grep` est un angle mort assumé, non reconstruit
   ([ADR 0021](adr/0021-pas-d-analyse-de-la-sortie-de-grep.md)).
   **Ce qui reste ouvert** : `head_limit` et sa troncature éventuellement silencieuse ; un `Bash`
   de **lecture** (`cat`, `head`), que rien ne couvre ; et le coût d'empreinter N fichiers
   rapportés par un seul `Grep`.
2. **Sortie de l'adaptateur déprécié** : contribuer en amont, adaptateur maintenu par Trame,
   hooks `PreToolUse`, ou accepter la dégradation ? [ADR 0017](adr/0017-adaptateur-acp-epingle.md)
   liste les quatre sans en engager aucune.
3. **`but` CLI ou `gix` natif** pour la v0.2 ?
4. **Un projet peut-il avoir plusieurs dépôts** (monorepo vs multi-repo lié) ?
5. **Positionnement** : outil perso, ou produit avec un angle auditabilité / souveraineté pour
   le marché européen ? La licence est tranchée, l'hébergement aussi
   ([ADR 0019](adr/0019-heberger-trame-sur-github.md) : GitHub, pour y trouver des
   contributeurs), le positionnement non.
6. **CI** : rester sur `.gitlab-ci.yml` ou passer à GitHub Actions une fois le dépôt créé ?
   Volontairement séparé du choix d'hébergement.

### Tranchées depuis la révision 2

| Question | Réponse | ADR |
|---|---|---|
| Quel harness en premier ? | Claude Code en ACP. L'interception avant disque **fonctionne**. | [0016](adr/0016-interception-avant-disque-validee.md) |
| Le niveau 1 informe-t-il l'agent automatiquement ? | **Oui**, et l'agent relit et s'adapte : 15/15. | [0018](adr/0018-pas-de-diff-dans-stalefile.md) |
| L'avis doit-il dire ce qui a changé ? | **Non.** Mesuré, réfuté. | [0018](adr/0018-pas-de-diff-dans-stalefile.md) |
| Fair Source ou open source ? | Open source, MIT OR Apache-2.0. | [0013](adr/0013-licence-open-source-mit-apache.md) |
| Le registre rend-il un verdict ou écrit-il ? | **Il écrit.** | [0014](adr/0014-le-registre-ecrit-sur-disque.md) |
