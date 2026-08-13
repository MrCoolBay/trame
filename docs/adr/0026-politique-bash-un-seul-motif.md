# 0026 — La politique `Bash` : un seul motif, et on ne l'élargit pas

- **Statut** : Acceptée
- **Date** : 2026-08-13
- **Mesures d'origine** : [sonde 2](../sondes/2026-08-12-pretooluse-live.md) pour le coût d'un
  refus, et la mesure du 2026-08-13 rapportée ici
- **S'applique par** `trame_hook::bash`, appelé par le daemon
  ([ADR 0025](0025-ipc-hook-daemon.md))

## Décision

**Un seul motif :** `> fichier`, quand la cible **n'est pas** sous `/dev/` et **tombe dans
l'arbre du projet**. Rien d'autre.

Et une seconde décision, qui en est le pendant : **on ne l'élargit pas.** Ni `>>`, ni `tee`, ni
`sed -i`, ni `mv`, ni `cp`. Pas avant qu'une mesure le justifie.

## La portée du registre n'était pas une exception à ajouter

Le motif a d'abord été écrit comme « toute redirection vers un fichier, sauf `/dev/*` ». Passé
sur 601 chaînes extraites du dépôt, il refusait **18,8 %** — dont `just tui 2>/tmp/tui.log`.

Le réflexe aurait été d'ajouter `/tmp` à une liste d'exceptions. Le vrai diagnostic est
ailleurs : **le registre ne suit que son arbre.** Une redirection vers `/tmp`, vers `~/` ou vers
`../` ne menace aucun invariant — le watcher FSEvents ne regarde même pas là, et le read-set ne
contient rien de ces fichiers. La règle était **mal cadrée**, pas incomplète.

Après avoir aligné le motif sur la portée réelle du registre — et exclu les gabarits de
documentation, où le `>` de `but uncommit <commit-id>` ferme un paramètre — le corpus tombe à
**2,3 %**.

Ce chiffre-là ne vaut pas grand-chose : le corpus sur-représente la documentation. **Sa valeur a
été de révéler le mauvais cadrage**, pas de mesurer un taux. C'est une correction qui tient dans
le temps, parce qu'elle découle de ce que le registre *est* et non d'une liste de cas.

## Ne pas élargir est une décision, et voici sa donnée

**Session réelle, neuf commandes `Bash` émises par un agent Claude Code, mode ombre** — on
enregistre, on ne refuse pas :

```
ls -la …                      find … -type f | head -50
grep -rn "verify_token" …     wc -l …
grep -rni "TODO\|FIXME\|…" … ; echo "Exit code: $?"
ls …/Cargo.toml 2>/dev/null && cat … || echo "…"
grep -rn "mod handlers" …     grep -rn "mod auth" …
tree … 2>/dev/null || find …
```

**Zéro refus sur neuf, soit 0 %.** Deux commandes portaient `2>/dev/null` : exactement
l'exclusion qui décide.

**Et le fait qui pèse plus que le taux.** Chargé d'écrire un `rapport.txt` à la racine du
projet, l'agent a pris son **outil de fichier**, pas une redirection. L'écriture est passée par
`fs/write_text_file`, donc par l'admission, avec verdict et provenance — **spontanément**, sans
qu'on lui refuse quoi que ce soit.

> **Le trou `Bash` est une capacité, pas une propension.** La sonde de la phase 3 a prouvé qu'un
> agent *peut* écrire par le shell quand on le lui demande. Cette mesure montre qu'il ne le
> *choisit* pas naturellement.

D'où la conclusion sur le périmètre : **le refus est un garde-fou, pas la correction d'un
comportement fréquent.** Un garde-fou se dimensionne au risque résiduel, pas à l'exhaustivité —
et chaque élargissement se paie au prix mesuré en sonde 2 : **un refus a fait passer un tour de
30 s à 68 s**, parce que l'agent replanifie. Un faux positif coûte plus du double d'un tour.

## Ce qui reste dehors, nommé

- `>>` en ajout, `tee`, `sed -i`
- **`mv` et `cp`** — fréquents en usage légitime, et déjà rattrapés par le watcher
- les heredocs, `>|`, les redirections de descripteurs (`2>&1`)
- un `Bash` de **lecture** (`cat`, `head`), qui échappe au read-set et n'est pas couvert ici

Ce sont des trous nommés, pas des oublis. Le watcher FSEvents les constate après coup : le
registre ne devient pas **faux**, il apprend juste plus tard, et le journal porte la ligne avec
`origin = observed`, sans verdict.

## L'analyse reste lexicale, et le restera

On ne détermine **jamais** ce qu'une commande shell écrit. Analyser une ligne de shell est
indécidable en général, et un interpréteur partiel serait faux précisément là où ça compte. On
ramène le trou dans le périmètre de l'admission au lieu de le modéliser : on refuse la commande,
avec un message qui renvoie l'agent vers ses outils de fichiers — ce qu'il fait, mesuré en
sonde 2.

Le suivi des guillemets existe pour le faux positif le plus facile à produire : `echo "a > b"`
ne redirige rien.

## Conséquences

- Le coût du motif est nul sur un usage réaliste, et son bénéfice est un garde-fou sur un cas
  que l'agent n'emprunte pas de lui-même. C'est un bon échange **tant que le motif reste
  étroit** ; il cesse de l'être dès qu'il produit des faux positifs.
- Un `Bash` de lecture reste le trou le plus gênant de la famille, et il n'est pas adressable
  par un refus : refuser `cat` dégraderait l'agent sans rien protéger.
- La décision de ne pas élargir doit être **relue avec sa donnée**, pas comme une préférence.
  D'où cet ADR.

## Alternatives écartées

- **Couvrir toutes les formes d'écriture shell** — `>>`, `tee`, `sed -i`, `mv`, `cp`,
  `truncate`, `dd`… Exhaustif, donc rassurant, et non mesuré. Chaque motif ajouté est un risque
  de replanification à 38 s l'unité, pour un comportement dont on vient de mesurer qu'il est
  rare. On ajoutera quand une mesure montrera un agent qui écrit par le shell malgré son outil
  de fichier disponible.
- **Ne rien refuser du tout**, et se reposer sur le watcher. Défendable : le watcher rattrape
  déjà, et le registre ne devient pas faux. Mais une écriture rattrapée après coup n'a **pas de
  verdict** — elle ne peut pas déclencher un avis, seulement corriger l'état. Le refus, lui,
  ramène l'écriture dans l'admission.
- **Refuser toute commande shell** dès qu'un doute existe. C'est ce que faisaient les manches de
  mesure en fermant `Bash`, et c'est acceptable pour une expérience, pas pour un produit : un
  agent privé de shell sur un vrai codebase est un agent dégradé.

## Ce qui invaliderait cette décision

- **Une mesure montrant un agent qui écrit par le shell alors que son outil de fichier est
  disponible.** C'est le déclencheur d'élargissement, et il est chiffrable : le journal porte les
  écritures `origin = observed` avec leur chemin. Si elles deviennent fréquentes et qu'elles
  proviennent de commandes shell de l'agent, le motif est trop étroit.
- **Un faux positif observé en usage réel.** Symétriquement : le motif est trop large, et le
  tour perdu se voit dans le flux.
- Un harnais dont l'outil de fichier est absent ou pénible, ce qui pousserait l'agent vers le
  shell par défaut. Le motif devrait alors être repensé pour ce harnais, pas élargi pour tous.
