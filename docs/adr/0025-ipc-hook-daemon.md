# 0025 — L'IPC hook ↔ daemon : une socket unix par projet, et un échec bruyant

- **Statut** : Acceptée
- **Date** : 2026-08-13
- **Mesures d'origine** : [sonde 2](../sondes/2026-08-12-pretooluse-live.md) et
  [sonde 3](../sondes/2026-08-12-postooluse.md)

## Les objectifs du plan hooks, corrigés

À lire avant tout le reste, parce qu'un objectif retiré se réintroduit tout seul dans trois mois
si personne n'a écrit pourquoi il est parti.

| Objectif | État | Pourquoi |
|---|---|---|
| Fermer le **trou lecture** — `Grep`/`Glob` échappent au read-set | **retenu** | `PostToolUse` porte `filenames`, et ne se déclenche ni sur refus ni sur échec ([sonde 3](../sondes/2026-08-12-postooluse.md)) |
| Fermer le **trou écriture `Bash`** | **retenu** | `PreToolUse` voit la commande avant exécution, un `deny` bloque réellement, et l'agent se rabat sur le chemin admis ([sonde 2](../sondes/2026-08-12-pretooluse-live.md)) |
| **Lever l'épinglage** à l'adaptateur 0.16.2 | ~~**retiré**~~ | L'hypothèse était fausse : sur 0.66.0 il n'existe **aucun** outil ACP vers lequel se rabattre. Refuser les natifs ne redirige pas l'agent, ça le prive de tout chemin d'écriture ([sonde 5](../sondes/2026-08-13-write-edit-par-hook.md)) |

Le dernier a une piste de remplacement, **documentée et non retenue**, avec ses déclencheurs de
réexamen : [ADR 0024](0024-pas-de-serveur-mcp-maison.md). Les hooks n'y changent rien.

Ce qui reste vrai et ne dépendait pas de cette hypothèse : **les hooks sont la seule voie mesurée
pour fermer les deux trous nommés dans l'[ADR 0016](0016-interception-avant-disque-validee.md).**
C'est déjà ce qui justifie le travail.

## Contexte

Un hook de la CLI est un **processus** : la CLI lance une commande, lui passe le payload sur
`stdin`, lit sa décision sur `stdout`. Mesuré à ~5,7 ms par invocation, soit 0,13 % d'un tour de
30 s ([sonde 2](../sondes/2026-08-12-pretooluse-live.md)).

Or la décision qu'on veut rendre appartient au **registre**, qui vit dans le daemon : un refus
d'écriture `Bash` est une politique du produit, et une lecture `Grep` doit entrer dans le
read-set d'une session identifiée. Le hook doit donc joindre le daemon, et pour un refus,
**de façon synchrone** — l'agent attend.

C'est le **premier IPC du projet**. L'[ADR 0022](0022-decoupage-daemon-gui.md) notait qu'il
n'existait pas et que le canal d'observation le préfigurait sans le coûter. Il arrive ici.

## Décision

**Une socket unix de domaine par projet, le daemon écoute, `trame-hook` demande.**

```
CLI Claude Code ──stdin JSON──> trame-hook ──UDS──> daemon ──> registre
                <──stdout JSON──           <──verdict──
```

- **`trame-hook`**, un binaire du workspace. Il lit le payload sur `stdin`, l'envoie sur la
  socket, attend le verdict, l'écrit sur `stdout` au format attendu par la CLI. **Il ne décide
  rien** : toute la politique est dans le daemon. Un hook qui déciderait serait une seconde
  copie des règles d'admission.
- **Une socket par projet**, dans le répertoire de données — jamais dans le projet surveillé, qui
  est précisément ce qu'on observe. Un chemin par projet, pas un global : le registre est par
  projet (invariant 3), la socket suit.
- **Le fichier de réglages qui déclare le hook vit hors du projet**, atteint par
  `_meta.claudeCode.options.extraArgs.settings` — établi en sonde 2. Les deux exclusions posées
  tiennent : rien dans le projet surveillé, rien dans `~/.claude`.

### ★ Le point critique : un daemon absent doit faire échouer le hook **bruyamment**

C'est une frontière neuve, donc l'endroit privilégié pour une sortie plausible qui mente. Le mode
d'échec à refuser est précis :

> Le daemon n'écoute pas — pas démarré, planté, socket périmée. `trame-hook` ne peut pas
> demander. S'il **sort 0 sans rien dire**, la CLI comprend « pas d'objection » et l'écriture
> passe. L'invariant est mort, et l'agent travaille normalement : **aucun symptôme.**

Donc : **en cas d'impossibilité de joindre le daemon, `trame-hook` refuse et le dit.** Sortie non
nulle et message explicite sur `stderr`. Un hook qui ne peut pas consulter la politique ne
présume pas la permission.

C'est le même raisonnement que le `Drop` de `FileWriteRequest`, qui refuse par défaut
([ADR 0016](0016-interception-avant-disque-validee.md)) : sur le chemin d'admission, l'absence de
réponse n'est jamais un oui.

**Le test de ce comportement s'écrit avant le chemin nominal.** Pas après, parce qu'un chemin
nominal qui fonctionne rend le cas dégradé abstrait, et un cas dégradé abstrait ne se teste
jamais vraiment.

Conséquence assumée, et à afficher : **si le daemon est absent, l'agent est bloqué en écriture
shell.** C'est bruyant, donc réparable. L'inverse est silencieux, donc pas.

## Conséquences

- Un binaire de plus dans le workspace, volontairement minuscule. Il doit démarrer vite : le
  budget est les ~5,7 ms mesurés pour un script shell, et un binaire Rust fait mieux. Aucune
  dépendance lourde, pas de runtime tokio — une socket bloquante suffit.
- **Le daemon devient un serveur.** Il doit gérer une socket périmée au démarrage (fichier
  restant d'un processus mort) et la retirer à l'arrêt.
- Le protocole sur la socket est un JSON par ligne, comme ACP. Le payload du hook y transite tel
  quel plus le contexte que le hook ne connaît pas — quel projet, quelle session.
- **La correspondance hook → session est un problème ouvert.** Le payload porte un `session_id`
  de la CLI, pas notre `SessionId`. Il faudra une table, alimentée à l'ouverture de session. Tant
  qu'elle n'existe pas, une lecture `Grep` ne peut pas être attribuée — et une lecture attribuée
  à la mauvaise session serait pire qu'une lecture manquante.
- Un timeout côté hook, court, pour ne pas suspendre l'agent si le daemon est vivant mais bloqué.
  Un timeout dépassé est traité comme un daemon absent : **refus**.

## Alternatives écartées

- **Le hook décide seul**, avec les motifs en dur dans le binaire. Plus simple, aucune IPC, aucun
  cas dégradé. Mais ça duplique la politique d'admission hors du registre, et une politique en
  deux exemplaires divergera. Surtout, un hook autonome ne peut pas alimenter le read-set : il
  n'a ni le `SessionId`, ni l'accès au registre.
- **Le hook écrit dans un fichier ou une file que le daemon lit** (asynchrone). Ça marche pour
  l'enregistrement d'une lecture `Grep`, ça ne marche **pas** pour un refus — la CLI attend une
  décision sur `stdout`. Deux mécanismes selon le hook serait deux fois la surface.
- **Un socket TCP sur localhost.** Fonctionne, et expose la politique d'admission à tout
  processus de la machine. Une socket unix se protège par les droits du système de fichiers.
- **Le hook parle au registre en l'important comme bibliothèque.** Impossible : le registre
  possède son état dans un acteur du daemon, et un second processus qui l'instancierait aurait
  un read-set vide (invariant 1).
- **En cas de daemon absent, laisser passer** en journalisant un avertissement. C'est le mode
  d'échec décrit plus haut. Un avertissement dans un log que personne ne lit pendant que
  l'invariant est mort, c'est exactement la classe de bug que ce projet a payée six fois.

## Ce qui invaliderait cette décision

- Le coût mesuré du hook devient visible pour l'utilisateur — un aller-retour UDS qui pousse le
  total bien au-delà des 5,7 ms observés. Il faudrait alors un mécanisme sans processus par appel,
  ce que la CLI ne propose pas aujourd'hui.
- La CLI cesse de garantir qu'un `deny` bloque — surveillé par la même famille de canaris que
  l'adaptateur ACP.
- Trame devient multi-processus pour d'autres raisons et se dote d'un IPC général. Cette socket
  deviendrait un cas particulier à absorber, et **la règle du refus par défaut devrait suivre.**
