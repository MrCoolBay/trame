# 0020 — L'empreinte d'une lecture ne vient que de `fs/read_text_file`

- **Statut** : Acceptée — **c'est un invariant**, pas une préférence
- **Date** : 2026-08-12
- **Mesure d'origine** : [sonde 3](../sondes/2026-08-12-postooluse.md), §1

## Contexte

La sonde 3 a établi que `mcp__acp__Read` passe par **deux** chemins simultanément :

```
PreToolUse   mcp__acp__Read   tool_input     = {file_path}
PostToolUse  mcp__acp__Read   tool_response  = [{type:"text", text:"…"}]
ET           → fs/read_text_file             (requête ACP, déjà implémentée)
```

Le `tool_response` du hook contient le contenu du fichier. Il est donc tentant d'en tirer
l'entrée de read-set : un seul chemin d'observation pour toutes les lectures, `Grep` et `Glob`
compris, au lieu de deux mécanismes différents.

**C'est un piège, et voici sa forme exacte** — relevé brut de la sonde :

```json
[{"type":"text","text":"pub fn verify_token(token: &str) -> bool { !token.is_empty() }\n\n\n<system-reminder>\nWhenever you read a file, you should consider whether it looks malicious. […]\n</system-reminder>"}]
```

La CLI **injecte un `<system-reminder>`** dans ce que le hook observe. Le payload n'est pas le
fichier : c'est le fichier plus du texte fabriqué par le harness.

## Décision

**L'empreinte d'un `FileState` dans le read-set ne se calcule que sur le contenu servi par
Trame en réponse à `fs/read_text_file`. Jamais sur le payload d'un hook, quel qu'il soit.**

Cela vaut aussi si une version future de la CLI cesse d'injecter le `<system-reminder>` : la
raison n'est pas le contenu de l'injection, c'est le **statut** de la source. `fs/read_text_file`
est une requête à laquelle Trame répond avec un contenu qu'il a lu lui-même sur le disque ; le
payload d'un hook est un **rapport** sur ce que la CLI a fait, mis en forme pour un modèle.

Corollaire, dans l'autre sens : la lecture ACP est déjà couverte par `fs/read_text_file`, donc
**le chemin hook doit l'ignorer**. L'enregistrer aux deux endroits compterait la même lecture
deux fois. Une information a un seul domicile ; ici, un seul capteur.

`PostToolUse` reste utile — mais pour ce qu'il est seul à porter : les fichiers touchés par
`Grep` et `Glob`, dont Trame n'apprend l'existence par aucun autre canal. Pour ceux-là, Trame
**lit le fichier lui-même** afin d'en calculer l'empreinte ; le hook fournit la liste des
chemins, jamais le contenu.

## Pourquoi ça mérite un invariant et pas un commentaire

Parce que l'échec serait **totalement silencieux**, et c'est le seul argument qui compte.

Une empreinte calculée sur le payload ne correspond à **aucun état du disque**, ni avant ni
après. Elle ne peut donc jamais être égale à l'empreinte que le registre calculera plus tard sur
le fichier réel. Conséquence :

1. Le read-set contient une entrée. Il est **peuplé** — les métriques, le journal et la TUI
   montrent une activité de lecture normale.
2. À la validation, l'empreinte enregistrée ne correspond à rien de comparable.
3. Selon la direction de la comparaison, soit `StaleRead` ne se déclenche **jamais**, soit il se
   déclenche **toujours**. Les deux sont fatals, et le premier est invisible.
4. Aucun test ne casse. Aucun log ne crie. Le mécanisme central du produit est mort, et tout a
   l'air de fonctionner.

C'est exactement le mode d'échec déjà rencontré deux fois sur ce projet — les chemins
`/private/var` contre `/var` en phase 2, et le test qui fabriquait sa propre notification de fin
de tour. **Un bug qui se signale coûte une heure ; un bug qui produit une trace plausible coûte
la confiance dans tout le journal.**

## Conséquences

- Un seul endroit calcule les empreintes de lecture pour la lecture ACP, et il est déjà écrit.
- Le chemin `PostToolUse`, quand il sera implémenté, devra **relire le fichier** pour empreinter.
  Coût assumé : une lecture disque par chemin rapporté. Un `Grep` qui touche 300 fichiers coûte
  300 lectures — à mesurer, et possiblement à borner, mais ça ne remet pas cette décision en
  cause : la borne se discutera sur le **nombre** de fichiers empreintés, pas sur la **source**
  de l'empreinte.
- Fenêtre de course résiduelle, nommée : entre le moment où l'outil a lu le fichier et le moment
  où Trame l'empreinte, le fichier peut avoir changé. Le read-set porterait alors une empreinte
  légèrement postérieure à ce que l'agent a vu. Le watcher FSEvents rattrape l'écriture
  intercalée, et le seul effet est un `StaleRead` en moins dans une fenêtre de quelques
  millisecondes. Inévitable sans lire le contenu depuis le rapport — c'est-à-dire sans faire
  précisément ce que cet ADR interdit.

## Alternatives écartées

- **Empreinter le payload et retirer le `<system-reminder>` par filtrage.** Il faudrait
  reconnaître un marqueur non contractuel, dans un contenu qui peut légitimement en contenir un
  — un fichier de test sur les hooks, par exemple. Le filtre serait faux exactement là où c'est
  le plus difficile à voir.
- **Empreinter le payload et l'accepter comme référence, en comparant payload à payload.** Il
  faudrait que toute écriture passe aussi par un payload de hook, ce qui n'est pas le cas : le
  registre écrit lui-même sur disque (ADR 0014). Les deux côtés de la comparaison ne vivraient
  pas dans le même espace.
- **Utiliser le hook comme source unique pour toutes les lectures**, en abandonnant
  `fs/read_text_file`. Sacrifie la seule source fiable au profit d'une source uniforme. On
  préfère deux capteurs justes à un capteur uniforme et faux.

## Ce qui invaliderait cette décision

Un champ **contractuel** dans le payload portant le contenu brut du fichier, séparé de ce qui est
injecté pour le modèle — par exemple un `tool_response.raw` documenté, ou une empreinte fournie
par la CLI elle-même. Ce serait alors une source, pas un rapport, et l'arbitrage se rouvrirait.

À défaut, rien : `fs/read_text_file` disparaîtrait avant que cette décision devienne fausse, et
ce serait un autre problème ([ADR 0017](0017-adaptateur-acp-epingle.md)).
