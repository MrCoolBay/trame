# 0021 — Le mode `content` de `Grep` est un angle mort assumé, pas un cas à reconstruire

- **Statut** : Acceptée
- **Date** : 2026-08-12
- **Mesure d'origine** : [sonde 3](../sondes/2026-08-12-postooluse.md), §1 et §5

## Contexte

`PostToolUse` rend les fichiers lus par `Grep` — mais pas dans les trois modes :

| `output_mode` | `filenames` | `numFiles` | où sont les chemins |
|---|---|---|---|
| `files_with_matches` (défaut) | **peuplé** | correct | ✅ clé structurée |
| `count` | **vide** | correct | chaîne `content`, `chemin:compte` |
| `content` | **vide** | **0, faux** | chaîne `content`, `chemin:ligne:texte` |

En mode `content`, les chemins existent, mais dans une chaîne de sortie :

```
sub/deep.rs:1:use crate::auth::verify_token;
sub/deep.rs:2:pub fn deep(t: &str) -> bool { verify_token(t) && verify_token("x") }
```

Les extraire est faisable. Ce ne serait pas rejouer le motif nous-mêmes — on ne devinerait
aucune correspondance, on lirait celles que `Grep` déclare avoir trouvées. La question n'est
donc pas la justesse du principe, c'est le coût de la dépendance.

## Décision

**Trame n'analyse pas la chaîne `content`.** Le read-set est alimenté par les clés structurées
uniquement : `filenames` de `Grep` en mode `files_with_matches`, et `filenames` de `Glob`.

Un `Grep` en mode `content` ou `count` est un **angle mort nommé, affiché et compté** — pas un
cas à reconstruire. Il rejoint les deux trous déjà inventoriés dans l'ADR 0016 : nommé, mesuré,
visible dans l'interface.

**La réécriture de `output_mode` en `PreToolUse` est écartée** pour la même raison de fond, plus
une qui lui est propre. `updatedInput` permettrait de forcer `files_with_matches` sur tout
`Grep`, et la couverture deviendrait totale. Mais l'agent recevrait des noms de fichiers là où il
a demandé des lignes : le remède **dégraderait l'agent**, c'est-à-dire produirait exactement le
dommage que toute cette piste sert à éviter. Et il le ferait en silence, dans un mode de sortie
dont l'agent n'a pas demandé le changement.

## Pourquoi refuser une couverture qu'on sait atteignable

**Un format de sortie n'est pas un contrat.** `filenames` est une clé nommée dans une structure
de données ; `"sub/deep.rs:1:use crate…"` est une mise en forme pour un modèle de langage. La
première a une chance de survivre à une version mineure ; la seconde peut changer parce que
quelqu'un a trouvé l'affichage plus lisible autrement. Rien ne le signalerait : le parseur
rendrait une liste vide ou fausse, le read-set serait silencieusement incomplet, et **aucun test
ne casserait**.

**Et le budget de dépendances non spécifiées est déjà consommé.** Trame en porte deux, toutes
deux sur une version dépréciée :

1. le retrait de `Write` et `Edit` quand le client annonce `fs.writeTextFile`
   ([ADR 0017](0017-adaptateur-acp-epingle.md)) — le mécanisme central du produit ;
2. les clés de `tool_input` et `tool_response` des hooks, observées et non documentées
   ([sondes 2 et 3](../sondes/2026-08-12-postooluse.md)).

Une troisième dépendance à un comportement tiers non spécifié se paierait en probabilité de
casse silencieuse, pas en lignes de code. Le canari de l'ADR 0017 surveille la première ; on ne
va pas multiplier les canaris pour élargir la couverture d'un cas déjà atténué.

## L'atténuation réelle, qui change la taille du trou

Elle n'est pas dans le protocole, elle est dans le comportement de l'agent.

Un `Grep` en mode `content` rend **les lignes correspondantes** à l'agent. Ce qu'il en tire est
souvent suffisant pour répondre — et si ça ne l'est pas, **il ouvre le fichier**. C'est ce que la
sonde a observé sans le chercher : l'agent grep, puis lit `auth.rs` par son outil de fichier,
donc par `fs/read_text_file`, donc **par le read-set**.

L'angle mort ne couvre donc pas « les fichiers que l'agent connaît » mais un ensemble plus petit :
**les fichiers dont quelques lignes lui ont suffi et qu'il n'a jamais ouverts**. Un renommage
dans un de ces fichiers reste dangereux — c'est précisément le scénario canonique — mais la
probabilité qu'un fichier soit à la fois central au raisonnement de l'agent et jamais ouvert par
lui est nettement plus faible que le taux brut d'appels en mode `content`.

C'est une atténuation, pas une fermeture, et elle est **non mesurée**. Elle réduit l'urgence, pas
le trou.

## Conséquences

- L'implémentation du chemin lecture est plus simple : deux clés à lire, aucun parseur, aucun
  garde-fou de résolution de chemin, aucun cas de chemin contenant un `:`.
- Un compteur devient nécessaire : **le nombre d'appels `Grep` non enregistrés, par session**.
  Sans lui, l'angle mort n'est pas « nommé et compté », il est juste toléré. Il alimentera
  l'affichage de dégradation de l'interface, au même titre que les écritures observées.
- La couverture annoncée à l'utilisateur ne doit **jamais** être présentée comme totale. Une
  interface qui laisse croire à une garantie qu'elle n'a pas est pire qu'une interface qui
  affiche son trou (principe n° 10 du concept : un trou nommé vaut mieux qu'un trou ignoré).
- Si le compteur montre que le mode `content` domine largement, cette décision se rouvre — mais
  par la mesure, pas par l'intuition.

## Alternatives écartées

- **Analyser la chaîne, avec un garde-fou de résolution** : ne retenir un candidat que s'il
  résout sur un fichier existant sous `ProjectRoot`. Techniquement solide, et ça règle le cas du
  `:` dans un chemin. Ça ne règle pas le problème réel : le jour où le format change, le garde-fou
  rejette tout, et le silence est le même.
- **Réécrire `output_mode` via `updatedInput`** — écartée ci-dessus : dégrade l'agent en silence.
- **Refuser `Grep` en mode `content`** et renvoyer l'agent vers `files_with_matches` ou vers son
  outil de lecture, comme on le fera pour les écritures `Bash`. Séduisant par symétrie, mais la
  symétrie est trompeuse : refuser une **écriture** renvoie vers un chemin équivalent, alors que
  refuser cette **lecture** retire une capacité sans équivalent. Et la sonde 2 a mesuré ce que
  coûte un refus — un tour multiplié par 2,2, parce que l'agent replanifie. Payer ça sur un
  `Grep`, l'outil le plus fréquent d'un agent sur un vrai codebase, est hors de question.
- **Ne rien enregistrer du tout tant que la couverture n'est pas totale.** C'est l'état actuel, et
  c'est le pire : le mode `files_with_matches` est le **défaut** de l'outil, donc refuser une
  couverture partielle revient à refuser la majorité des cas pour ne pas admettre une minorité.

## Ce qui invaliderait cette décision

- Le compteur d'appels non enregistrés montre que le mode `content` est **majoritaire** en usage
  réel, et le journal montre des `StaleRead` manqués attribuables à ces appels.
- Ou : la CLI expose `filenames` dans **tous** les modes. La décision devient sans objet, et c'est
  la sortie souhaitable — elle est à demander en amont plutôt qu'à contourner en aval.
