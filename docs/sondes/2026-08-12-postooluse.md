# Sonde 3 — `PostToolUse` expose-t-il les résultats de `Grep` ?

- **Date** : 2026-08-12
- **Suite de** [`2026-08-12-pretooluse.md`](2026-08-12-pretooluse.md) (contrat) et
  [`2026-08-12-pretooluse-live.md`](2026-08-12-pretooluse-live.md) (session réelle)
- **Périmètre** : une seule question. **Rien n'est implémenté dans le produit** — aucun fichier
  de `crates/` ni de `apps/` n'est touché par cette sonde.
- **Coût** : 5 tours réels, dont un invalidé par un bug de méthode (§6). Aucun tour n'a modifié
  le dépôt Trame ; tout s'est passé dans `/tmp/sonde-hooks/projet`.

## Réponse en une ligne

**Oui, mais pas dans les trois modes de `Grep`.** `tool_response.filenames` porte la liste des
fichiers correspondants — **structurée, complète, et sans faux positif** — en mode
`files_with_matches` et pour `Glob`. En mode `content` et `count`, `filenames` est **vide** et
les chemins n'existent que dans la chaîne `content`.

Ce n'est donc **ni** la sortie 1 **ni** la sortie 2 de l'énoncé. C'est une troisième : le trou se
ferme, avec un arbitrage résiduel sur un mode et un seul.

---

## 1. Ce que `PostToolUse` porte, mesuré

Enveloppe observée, identique à `PreToolUse` **plus une clé** :

```
cwd, hook_event_name, permission_mode, session_id, transcript_path,
tool_input, tool_name, tool_use_id, tool_response          ← la nouvelle
```

Le `tool_use_id` est **le même** qu'en `PreToolUse` pour le même appel : l'appariement
avant/après est direct, sans heuristique.

Sondé statiquement d'abord, dans le bundle de la CLI épinglée — le schéma type `tool_response`
en `x.unknown()`, donc le typage ne dit **rien** de son contenu. Seule l'observation tranche.

### `Grep`, les trois modes

| `output_mode` | `filenames` | `numFiles` | où sont les chemins |
|---|---|---|---|
| `files_with_matches` (défaut) | **peuplé** | correct | ✅ clé structurée |
| `count` | **vide** | correct | chaîne `content`, format `chemin:compte` |
| `content` | **vide** | **0, faux** | chaîne `content`, format `chemin:ligne:texte` |

Relevés bruts :

```json
{"pattern":"verify_token","output_mode":"files_with_matches"}
→ {"mode":"files_with_matches","numFiles":4,
   "filenames":["sub/deep.rs","middleware.rs","handlers.rs","auth.rs"]}

{"pattern":"verify_token","output_mode":"count"}
→ {"mode":"count","numFiles":4,"filenames":[],"numMatches":7,
   "content":"middleware.rs:2\nsub/deep.rs:2\nauth.rs:1\nhandlers.rs:2"}

{"pattern":"verify_token","path":"sub","output_mode":"content"}
→ {"mode":"content","numFiles":0,"filenames":[],"numLines":2,
   "content":"sub/deep.rs:1:use crate::auth::verify_token;\nsub/deep.rs:2:pub fn deep(…"}
```

**`numFiles` n'est pas une source fiable** : il vaut `0` en mode `content` alors que trois
fichiers correspondaient. Ne pas s'en servir pour détecter une liste incomplète.

### `Glob`

```json
{"pattern":"**/*.rs"}
→ {"filenames":["/private/tmp/…/auth.rs","/private/tmp/…/handlers.rs",
                "/private/tmp/…/middleware.rs","/private/tmp/…/config.rs"],
   "numFiles":4,"durationMs":8,"truncated":false}
```

Structuré, complet, **et un champ `truncated`** — que `Grep` n'a pas.

### Forme des chemins — les deux outils ne s'accordent pas

- **`Grep`** rend des chemins **relatifs au `cwd`**, y compris quand l'appel porte un argument
  `path` : `path: "sub"` donne `sub/deep.rs`, pas `deep.rs`. Vérifié aussi avec un `path`
  absolu — les chemins rendus restent relatifs.
- **`Glob`** rend des chemins **absolus et résolus** : `/private/tmp/…`, pas `/tmp/…`.

`ProjectRoot` absorbe déjà les deux formes — c'est exactement ce pour quoi il existe depuis le
run live de la phase 2. Mais il faut les traiter tous les deux, et un test devra épingler les
deux formes, sinon la régression passera inaperçue.

### `mcp__acp__Read` — il passe par **les deux** chemins

C'était la quatrième question, et la réponse n'est pas « l'un ou l'autre » :

```
PreToolUse   mcp__acp__Read   tool_input  = {file_path}
PostToolUse  mcp__acp__Read   tool_response = [{type:"text", text:"…"}]
ET  → fs/read_text_file /private/tmp/…/auth.rs     (le chemin déjà implémenté)
```

**Conséquence de conception : garder le chemin `fs/read_text_file` et ignorer les hooks pour la
lecture ACP.** Deux raisons, dont une non évidente :

1. `fs/read_text_file` est une **requête** à laquelle Trame répond avec le contenu qu'il a lu
   lui-même — donc l'empreinte est calculée sur ce que Trame a servi, pas sur un rapport.
2. Le `tool_response` du hook contient le contenu **plus un `<system-reminder>` injecté par la
   CLI**. Empreinter le payload du hook au lieu du fichier produirait une empreinte qui ne
   correspond à aucun état du disque. Le piège est silencieux : ça « marche » et le
   `StaleRead` ne se déclenche jamais.

Enregistrer les deux compterait la même lecture deux fois. Un seul domicile, ici comme ailleurs.

---

## 2. Le résultat qui rend l'enregistrement sûr

Un `PostToolUse` qui se déclencherait sur un appel refusé ou en échec ferait enregistrer des
lectures qui n'ont pas eu lieu. **Ce n'est pas le cas**, dans les deux sens :

| situation | `PreToolUse` | `PostToolUse` |
|---|---|---|
| appel réussi | ✅ | ✅ |
| appel **refusé** par un `deny` en `PreToolUse` | ✅ | **aucun** |
| appel **en échec** (`Grep` sur un répertoire inexistant) | ✅ | **aucun** |

Mesuré : sur un tour à deux `Grep` dont un refusé, `pre.jsonl` contient les deux et
`post.jsonl` **un seul**. Sur un tour à trois `Grep` dont un sur `absent/`, deux `PostToolUse`.

C'est la garantie qui manquait : `PostToolUse` ne rapporte que ce qui s'est réellement produit.
Le read-set alimenté par ce chemin ne peut pas contenir de lecture fantôme.

---

## 3. Troncature — la liste est-elle complète ?

Fixture de 300 fichiers contenant le même motif, un `Grep` en `files_with_matches` :

```
numFiles=300   len(filenames)=300   truncated=(clé absente)
```

**Aucune troncature à 300 fichiers**, et le payload du hook porte la liste entière. `Grep` n'a
pas de champ `truncated` ; `Glob` en a un, à surveiller.

**Non sondé** : le paramètre `head_limit` que l'agent peut passer. Si `head_limit` tronque aussi
le payload, la liste serait incomplète **sans champ pour le dire**. À vérifier avant de dépendre
de l'exhaustivité.

---

## 4. Coût

| | médiane | p90 | max |
|---|---|---|---|
| payload 326 o (`Grep files_with_matches`) | 5,81 ms | 6,02 ms | 6,91 ms |
| payload 35 ko (lecture d'un gros fichier) | 5,90 ms | 6,12 ms | 6,80 ms |

60 invocations chacun. **La taille du payload ne coûte rien** — 100× plus de données pour 0,1 ms
de plus. Le coût est celui du `fork`/`exec`, pas du transport.

Donc **Pre + Post ≈ 11,7 ms par appel d'outil**, contre 5,7 ms avec `PreToolUse` seul. Sur un
tour de 30 s à huit outils : 94 ms, soit 0,3 %. Le doublement du nombre de processus ne change
pas l'ordre de grandeur.

---

## 5. Ce que ça décide, et ce qui reste à décider

### Le trou lecture se ferme

La couverture devient réelle, avec la répartition suivante :

| ce que l'agent fait | par où Trame le voit | état |
|---|---|---|
| lecture par l'outil de fichier | `fs/read_text_file` | **implémenté** |
| `Glob` | `PostToolUse.tool_response.filenames` (absolus) | mesuré, non implémenté |
| `Grep` en `files_with_matches` | `PostToolUse.tool_response.filenames` (relatifs) | mesuré, non implémenté |
| `Grep` en `content` / `count` | chaîne `content` uniquement | **arbitrage ouvert** |
| écriture par `Bash` | refus en `PreToolUse` | mesuré, non implémenté |

Aucune de ces voies n'exige de fermer `Grep` ni `Glob`. **La sortie 2 de l'énoncé — dégrader
l'agent en le privant de recherche — n'est pas nécessaire.**

### L'arbitrage résiduel, et pourquoi il vous revient

En mode `content` et `count`, les chemins existent, mais dans une chaîne. Les extraire est
**une lecture du rapport de l'outil**, pas un rejeu du motif : on n'invente aucune
correspondance, on lit celles que `Grep` déclare avoir trouvées. C'est une différence de nature
avec ce que vous avez exclu, et je ne la présente pas comme équivalente.

Ça reste du découpage de chaîne sur `:`, donc faillible sur un chemin contenant un `:`. Un
garde-fou existe — ne retenir un candidat que s'il résout sur un fichier existant sous
`ProjectRoot` — mais c'est de la conception, hors périmètre de cette sonde.

Trois options, **non tranchées ici** :

1. **Analyser la chaîne `content`.** Couverture complète. Introduit un analyseur de format de
   sortie, c'est-à-dire une dépendance à un détail non contractuel de la CLI.
2. **N'enregistrer que `files_with_matches` et `Glob`.** Aucun analyseur, aucun risque, mais le
   mode `content` reste un trou — et c'est le mode où l'agent en apprend le **plus**, puisqu'il
   voit les lignes.
3. **Réécrire `output_mode` en `PreToolUse` via `updatedInput`.** Techniquement possible.
   **À écarter** : ça changerait le résultat que l'agent reçoit — il demanderait des lignes et
   obtiendrait des noms de fichiers. Le remède dégrade l'agent, exactement ce qu'on cherchait à
   éviter.

Une donnée pour arbitrer : sur 11 appels `Grep` observés, 5 en `content`, 5 en
`files_with_matches`, 1 en `count`. **Cette proportion ne mesure rien** — mes énoncés
orientaient le mode dans plusieurs tours. Elle dit seulement que `content` n'est pas marginal.

---

## 6. Un bug de méthode, et ce qu'il a coûté

Mon premier hook de refus était écrit ainsi :

```sh
/usr/bin/python3 - "$@" <<'PY'      # ← le heredoc EST le stdin de python
raw = sys.stdin.read()              # ← lit donc EOF, jamais le payload
```

Le programme arrivant par stdin, `sys.stdin.read()` rendait une chaîne vide. Le hook **n'a rien
observé et n'a rien refusé**, et le tour est parti quand même : il a produit une trace
plausible, avec un `Grep` qui « n'a rien trouvé » — parce que le motif choisi était absent de la
fixture, pas parce qu'il avait été refusé.

Sans le contrôle « `pre.jsonl` est vide, or il devrait contenir une ligne par appel d'outil »,
j'aurais conclu que
`PostToolUse` se déclenche après un refus. **La conclusion aurait été fausse et inversée.**
Corrigé en sortant le script python dans un fichier, puis vérifié à vide sur les deux branches
avant de relancer un tour.

C'est la leçon déjà inscrite dans la skill `concurrency-testing`, dans sa version sonde : un
harnais qui fabrique lui-même la condition qu'il observe ne l'observe pas.

---

## 7. Reporté, à ne pas explorer maintenant

- **Motifs `Bash` conservateurs.** Le refus a doublé le tour en sonde 2 — 68 s contre 30 —
  parce que l'agent replanifie. Mieux vaut laisser passer un cas que payer une replanification
  sur un faux positif. À écrire dans l'ADR de l'implémentation.
- **`head_limit`** et la troncature silencieuse du payload (§3).
- Le garde-fou de résolution pour l'option 1 (§5).
- `PostToolUse` sur `Bash` : non sondé. Sans intérêt si `Bash` est refusé en amont pour les
  écritures, mais un `Bash` de **lecture** (`cat`, `head`) reste un trou non couvert par cette
  sonde.

## 8. Reproduire

```sh
/tmp/sonde-hooks/settings-lecture.json      # hooks Pre + Post, observation pure
/tmp/sonde-hooks/runner-lecture.mjs         # Grep content, Glob, lecture ACP
/tmp/sonde-hooks/runner-modes.mjs           # les trois output_mode
/tmp/sonde-hooks/runner-refus2.mjs          # deny en Pre, on regarde si Post suit
/tmp/sonde-hooks/runner-masse.mjs           # 300 fichiers, troncature
env -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT node runner-lecture.mjs lecture
```

Le fichier de réglages vit **hors du projet observé**, atteint par
`_meta.claudeCode.options.extraArgs.settings` — établi en sonde 2, inchangé ici.
