# Sonde 2 — `PreToolUse` en session réelle

- **Date** : 2026-08-12
- **Statut** : **sonde, aucune décision, rien d'implémenté dans le produit**
- **Suite de** : [`2026-08-12-pretooluse.md`](2026-08-12-pretooluse.md), qui établissait le
  contrat sans l'observer
- **Autorisation** : décision explicite de l'utilisateur — authentification et contournement
  du garde-fou anti-imbrication, contre la levée de toutes les incertitudes
- **Coût** : trois tours d'agent réels, courts. Ma session a survécu aux trois.

**Toutes les incertitudes de la sonde 1 sont levées.** Et le résultat le plus utile n'était
pas dans les questions posées.

---

## 0. Le verrou : un fichier de réglages hors du projet

**Franchi, par une voie que ni la sonde 1 ni moi n'avions identifiée.**

`--settings` ne passe **pas** par l'option `settings` : le SDK 0.2.44 n'en a aucune. La ligne
que la sonde 1 citait venait de la version **0.66.0**, pas de la version épinglée — erreur de
lecture de ma part, corrigée ici.

La voie qui fonctionne :

```json
"_meta": { "claudeCode": { "options": {
    "extraArgs": { "settings": "/chemin/hors/projet/settings.json" } } } }
```

`extraArgs?: Record<string, string | null>` existe dans le SDK 0.2.44 (`sdk.d.ts:560`) et
l'adaptateur spread `...userProvidedOptions`. **C'est du JSON pur, donc ça traverse
JSON-RPC** — contrairement aux hooks callback, qui sont des fonctions et ne passent pas.

Vérifié par capture d'`argv` (coût nul), le fichier apparaît bien sur la ligne de commande :

```
--setting-sources user,project,local … --settings /tmp/sonde-hooks/settings.json
```

Les deux exclusions posées sont respectées : rien dans `.claude/` du projet surveillé, rien
dans `~/.claude/`.

### Une voie écartée, et pourquoi

`CLAUDE_CONFIG_DIR` déplace le répertoire de configuration
(`O8() = process.env.CLAUDE_CONFIG_DIR ?? join(homedir(), ".claude")`) et semblait une
troisième option propre. **Elle casse l'authentification** : le nom du service Keychain dérive
d'un sha256 du répertoire de configuration.

```js
function Kg(A=""){ let q=O8(),
  Y = !process.env.CLAUDE_CONFIG_DIR ? "" : `-${sha256(q).digest("hex").substring(0,8)}`;
  return `Claude Code${OAUTH_FILE_SUFFIX}${A}${Y}` }
```

Un répertoire déplacé n'a donc pas d'identifiants. À ne pas retenter.

---

## 1. Le hook se déclenche-t-il ?

**Oui. 7 invocations sur un tour de trois outils.**

Réglages utilisés — hook `command`, **sans `matcher`** :

```json
{ "hooks": { "PreToolUse": [
    { "hooks": [ { "type": "command", "command": "/tmp/sonde-hooks/hook-observe.sh" } ] } ] } }
```

Le hook n'écrit rien sur sa sortie standard : **observation pure, aucune décision.**

L'absence de `matcher` signifie bien « tous les outils » : sur 7 invocations, **4 étaient des
`TodoWrite`** — un outil qui ne nous intéresse pas du tout. À retenir pour le dimensionnement :
le hook est appelé bien plus souvent que le nombre d'outils qui nous concernent.

---

## 2. Les clés de `tool_input` — **observées**, pas attendues

Enveloppe, identique aux 16 invocations des deux runs :

```
cwd · hook_event_name · permission_mode · session_id · tool_input · tool_name
· tool_use_id · transcript_path
```

`permission_mode` n'était **pas** dans le type `PreToolUseHookInput` que la sonde 1 citait : il
vient de `BaseHookInput`. Le typage ne suffisait donc pas, l'observation était nécessaire.

| `tool_name` observé | `tool_input` observé |
|---|---|
| `Grep` | `pattern`, `path`, `output_mode` — et **`path` est absent** quand l'agent cherche depuis la racine |
| `mcp__acp__Read` | `file_path` |
| `mcp__acp__Edit` | `file_path`, `old_string`, `new_string` |
| `Bash` | `command`, `description` |
| `TodoWrite` | `todos` |

Trois choses à noter, dont deux qui n'étaient pas prévues.

**La lecture n'est pas `Read`, c'est `mcp__acp__Read`.** Annoncer `fs.readTextFile` fait
retirer `Read` et le remplacer par l'outil ACP — donc le hook voit un nom *préfixé*. Un filtre
écrit sur `"Read"` ne verrait rien. Et cette lecture **passe déjà par nous** via
`fs/read_text_file` : la traiter aussi dans le hook, c'est la compter deux fois.

**`Grep` n'expose que sa requête, jamais son résultat.** `pattern` et `path` disent *où l'agent
a cherché*, pas *quels fichiers il a lus*. `path` est en plus optionnel — absent dans un des
deux runs. Ça a une conséquence directe sur la direction retenue pour `Grep`, traitée en §5.

**`Bash` expose la commande entière**, plus une `description` en langage naturel rédigée par
l'agent — utile pour un message d'erreur, pas pour une décision.

---

## 3. Un deny depuis un hook commande bloque-t-il réellement ?

**Oui.** Hook de contrôle refusant **uniquement** `Bash` :

```sh
printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse",
  "permissionDecision":"deny","permissionDecisionReason":"Trame: ecriture par shell refusee,
  utilise tes outils de fichiers"}}\n'
```

La commande `echo … >> notes.txt` **n'a pas été exécutée**. L'agent l'a constaté et l'a dit :

> *Le hook bloque l'écriture via shell. Je vais utiliser l'outil de fichier à la place pour
> respecter la contrainte du hook.*

Puis, dans son résumé final :

> *la commande shell a été bloquée par un hook, j'ai donc utilisé l'outil d'édition de fichier*

**Le `permissionDecisionReason` remonte jusqu'à l'agent et il agit dessus.** C'est la
confirmation que le canal porte non seulement un refus, mais un refus *explicable*.

### ★ Le résultat que je ne cherchais pas

L'agent n'a pas abandonné : il a **contourné le refus par le bon chemin**.

```
46 484 ms  Bash  `echo … >> notes.txt`     ← REFUSÉ par le hook
53 279 ms  mcp__acp__Read  notes.txt        ← il relit
61 166 ms  mcp__acp__Edit  notes.txt        ← il bascule sur son outil de fichier
61 185 ms  → fs/write_text_file             ← ET ÇA PASSE PAR L'ADMISSION
```

Le fichier a bien été modifié — **mais par le chemin admis**, avec verdict, journalisation et
provenance.

C'est exactement la direction de conception retenue pour `Bash` : *ramener le trou dans le
périmètre de l'admission au lieu de le modéliser.* Validé empiriquement avant d'être
implémenté, et sans qu'on l'ait demandé — l'agent, invité à utiliser ses outils de fichiers,
utilise ses outils de fichiers.

---

## 4. La latence

Deux coûts très différents, et confondre les deux mènerait à la mauvaise conclusion.

### Le coût du hook : négligeable

| Mesure | Valeur |
|---|---|
| Une invocation du hook, 60 tirages | **médiane 5,7 ms** · p90 6,4 ms |
| Tour sans hook | 30 667 ms |
| Tour avec hook d'observation (7 invocations) | 29 968 ms |

7 × 5,7 ms ≈ **40 ms sur un tour de 30 s, soit 0,13 %**. La différence entre les deux tours
(−700 ms) est du bruit : la réflexion de l'agent domine de trois ordres de grandeur.

Réserve honnête : ce hook est un plancher — `sh` + `cat` + une écriture. Un hook réel doit
joindre Trame, donc ajouter un aller-retour IPC. Même à 20 ms, l'ordre de grandeur reste
négligeable devant les 7 s que l'agent met entre deux outils.

### Le coût d'un refus : substantiel

| Tour | Durée |
|---|---|
| sans hook | 30 667 ms |
| hook d'observation | 29 968 ms |
| **hook avec un deny** | **67 830 ms** |

**Un seul refus a plus que doublé le tour.** Ce n'est pas le hook qui coûte, c'est la
replanification : l'agent constate l'échec, relit le fichier, choisit une autre voie, refait le
travail. Trois outils de plus.

C'est une donnée de conception, pas un défaut : refuser a un prix, donc on ne refuse que ce
qu'on tient vraiment à refuser. Ça renforce l'asymétrie déjà retenue — refuser sur `Bash`,
observer sur `Grep`.

---

## 5. Ce que ça implique pour la direction retenue

Rappel de la direction, qui n'est pas à explorer maintenant : **refuser** les commandes shell
qui écrivent, **enregistrer** ce que `Grep` lit.

### Sur `Bash` : faisable, et le mécanisme est validé

La commande complète est disponible, le refus fonctionne, le motif atteint l'agent, et l'agent
se rabat sur le chemin admis. Reste à décider **quel** motif de refus déclencher — `>`, `>>`,
`sed -i`, `tee`, `mv`, `cp`. C'est une reconnaissance de motifs syntaxiques, volontairement
pas une analyse sémantique.

Un point que la sonde éclaire : les faux positifs seront coûteux. Un refus double la durée du
tour. Un motif trop large — refuser tout `>` alors que `cmd > /dev/null` n'écrit rien d'utile —
se paiera en secondes à chaque occurrence.

### Sur `Grep` : le mécanisme ne suffit pas, et il faut le dire

**`PreToolUse` ne donne pas les fichiers lus par un `Grep`.** Il donne `pattern` et un `path`
optionnel — l'endroit où l'agent a cherché, pas ce qu'il a trouvé. Or le read-set a besoin des
**fichiers** et de leur **empreinte**.

Trois options, toutes hors périmètre ici, aucune évidente :

1. **`PostToolUse`** verrait le résultat du `Grep`, donc les fichiers correspondants. Mais
   après exécution — acceptable pour un read-set, qui n'a pas besoin d'être préventif.
2. **Rejouer le `Grep`** côté Trame depuis `pattern` et `path` pour connaître les fichiers
   touchés. Double le travail et peut divergier de ce que l'agent a réellement vu.
3. **Traiter un `Grep` comme une lecture non substantielle** — donc ne rien enregistrer, ce qui
   est la position actuelle du `ReadKind`. Cohérent, mais laisse le trou ouvert.

**Le trou lecture n'est donc pas fermé par cette sonde.** `PreToolUse` seul ne le ferme pas.
La piste reste la meilleure disponible, mais elle demande `PostToolUse` en complément, et ça
n'a pas été sondé.

---

## Synthèse

| Question | Réponse | Preuve |
|---|---|---|
| Fichier de réglages hors projet ? | **Oui**, par `extraArgs.settings` — pas par `settings` | argv capturé |
| Le hook se déclenche ? | **Oui**, 7 invocations, tous outils, `TodoWrite` compris | 2 runs live |
| Clés de `tool_input` ? | **Observées** — `Grep`: `pattern`/`path?`/`output_mode` · `mcp__acp__Read`: `file_path` · `Bash`: `command`/`description` | 16 captures |
| Un deny bloque ? | **Oui**, et le motif atteint l'agent, qui se rabat sur le chemin admis | run live + diff disque |
| Latence du hook ? | **~5,7 ms par appel, 0,13 % du tour.** Négligeable | 60 tirages + 3 runs |
| Latence d'un refus ? | **Tour ×2,2.** Substantiel | 3 runs |
| Trou lecture fermé ? | **Non.** `PreToolUse` ne donne pas les fichiers lus par `Grep` | §5 |

**Deux corrections à la sonde 1** : l'option `settings` que je citais venait de 0.66.0 et
n'existe pas sur la version épinglée ; et `permission_mode` est dans l'enveloppe alors que le
typage cité ne le mentionnait pas. Dans les deux cas, l'observation a corrigé le contrat — ce
qui est exactement la leçon inscrite dans la skill `concurrency-testing`.

## Ce que je n'ai pas fait

**Rien d'implémenté dans le produit.** Aucun fichier de `crates/` touché.

Non sondé, et nécessaire avant de s'engager : **`PostToolUse`** — voit-il le résultat d'un
`Grep`, et sous quelle forme ? C'est ce qui décide si le trou lecture se ferme.

Évidence brute conservée dans `/tmp/sonde-hooks/` — `captures.jsonl`, `captures-deny.jsonl`,
`run-*.log`, `msg-deny.txt` — jusqu'au prochain nettoyage du système.
