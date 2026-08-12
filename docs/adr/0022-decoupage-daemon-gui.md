# 0022 — Découpage daemon / GUI : la GUI observe, elle ne pilote pas

- **Statut** : Acceptée
- **Date** : 2026-08-12
- **Généralise** ce que la TUI applique déjà depuis la phase 3.5, et le rend opposable à
  toute interface future.

## Le point de fond, avant la technique

**Le daemon est le produit. La GUI est interchangeable.**

Ce n'est pas une figure de style, c'est la condition qui autorise le reste. Parier sur un
framework d'interface pré-1.0 ([ADR 0023](0023-gpui-ce-pour-la-gui.md)) serait déraisonnable
si l'interface portait de la logique produit. Ça devient raisonnable dès lors que la remplacer
coûte le prix de son propre code, et **rien d'autre** : pas un verdict à réimplémenter, pas
une règle d'admission à retrouver, pas une décision qui vivrait dans un gestionnaire de clic.

La TUI le démontre déjà : `apps/trame-tui` fait ~600 lignes d'état et de rendu, sur un
mécanisme d'admission qui n'y a pas une ligne.

## Contexte

Deux interfaces vont coexister — la TUI reste utile en CI et en débogage, la GUI devient la
cible utilisateur. Rien n'empêche techniquement une interface de tenir un
`trame_registry::RegistryHandle` : il est `Clone`, et `admit` est public parce que le pilote
de session en a besoin.

Une convention de revue — « on ne pilote pas depuis l'UI » — tiendrait le temps qu'un
raccourci soit pratique. Il le sera : « ce serait tellement simple de forcer l'écriture depuis
le panneau ».

## Décision

**Une interface reçoit un `tokio::sync::mpsc::Receiver<Observation>` et rien d'autre du
daemon.** Pas de `RegistryHandle`, pas de `JournalHandle`, pas de `&Project` mutable.

C'est une contrainte de **typage**, pas de discipline : un `Receiver` n'expose que `recv`.
`admit` n'est pas accessible depuis l'interface, donc la question « et si on pilotait un peu »
ne se pose pas au moment de la revue — elle ne se pose pas du tout.

Le canal est celui de `trame_daemon::observe`, déjà en place :

```rust
let (observer, observations) = observe_channel();   // le daemon garde l'Observer
// l'interface ne recoit que `observations`
```

Et il porte **exactement** ce que l'interface affiche : `SessionOpened`, `StateChanged`,
`Read`, `Write`, `Refused`, `Notice`, `ExternalWrite`, `Lost`. Chaque variante ajoutée est une
promesse faite à l'utilisateur ; l'énumération est volontairement pauvre.

### Périmètre d'affichage v0.1, identique pour toute interface

- un panneau par session, avec son état — `Idle` / `Thinking` / `Writing`
- le flux d'événements en direct, verdicts mis en évidence
- `StaleRead` distinct de `Clean` **sur deux axes** : couleur **et** marqueur textuel. La
  couleur seule disparaît en niveaux de gris, dans une capture, et pour une partie des
  utilisateurs.
- la distinction **admis / observé** visible. Le watcher constate après coup ; une interface
  qui laisse croire l'inverse promet une garantie qu'elle n'a pas.
- un indicateur de dégradation quand `can_intercept_writes` est faux, qui **nomme ce qui
  manque**. « Mode dégradé » ne dit rien à personne.

Hors périmètre, à refuser : multi-projet, gestion de branches, diffs, configuration.

### Ce que ça impose au daemon

**Toute action utilisateur future passera par un message explicite du daemon**, pas par un
accès direct. Le jour où l'interface devra lancer une session ou annuler un tour, la réponse
n'est pas « donnons-lui le handle » mais « ajoutons un canal de commande », typé, avec sa
propre énumération d'actions permises. La v0.1 n'en a aucune : elle observe.

## Conséquences

- **Deux interfaces, une seule source.** TUI et GUI consomment le même type. Ce qui est vrai
  dans l'une l'est dans l'autre, et la TUI reste le banc d'essai le moins cher — elle se
  rend dans un `TestBackend` et se relit cellule par cellule.
- **La GUI est jetable sans négociation.** Si `gpui-ce` déçoit, on réécrit `apps/trame-gui`
  et le reste du dépôt ne bouge pas.
- **Perdre une observation est acceptable, et déjà traité.** `Observer::emit` ne bloque
  jamais et déclare ses pertes via `Observation::Lost` : une interface lente ne doit pas
  pouvoir ralentir une écriture d'agent (voir le module `observe`).
- Une interface ne peut pas afficher ce que le canal ne porte pas. C'est le but : ajouter un
  affichage demande d'ajouter une variante, donc de décider explicitement qu'on la promet.
- Coût réel : un aller-retour quand l'interface a besoin d'un état complet plutôt que d'un
  flux — au démarrage, par exemple. En v0.1 l'interface démarre vide et se remplit ; si ça
  devient gênant, la réponse est une variante `Snapshot` dans `Observation`, pas un handle.

## Alternatives écartées

- **Donner un `RegistryHandle` en lecture seule** — via un trait qui n'expose que
  `snapshot()`. Plus souple, et ça règle le démarrage à froid. Mais un snapshot donne l'état,
  pas les événements : il ne dit pas qu'un `StaleRead` a été rendu, seulement qu'un fichier a
  un dernier écrivain. Il faudrait les deux, et le trait deviendrait une porte qu'on
  élargirait.
- **Un canal bidirectionnel dès maintenant**, en prévision des actions. Spéculatif : on ne
  sait pas quelles actions, et une énumération de commandes inventée avant l'usage sera
  fausse. Le jour où une action existe, elle arrive avec son cas d'usage.
- **Mettre l'état d'affichage dans le daemon**, l'interface n'étant qu'un rendu. Ça
  déplacerait des décisions d'affichage — quelles lignes garder, quelle borne — dans le
  produit, et ferait dépendre le daemon des besoins de l'UI. Exactement l'inverse du but.
- **Un IPC (socket, JSON-RPC) entre daemon et GUI dès la v0.1.** C'est le découpage d'un
  produit multi-processus, et il viendra peut-être. Aujourd'hui les deux vivent dans le même
  binaire : payer la sérialisation maintenant, c'est payer pour une frontière qui n'existe
  pas. Le `Receiver` la préfigure sans la coûter — un canal se remplace par un socket sans
  changer la forme du code de l'interface.

## Ce qui invaliderait cette décision

- L'interface a besoin d'une action utilisateur en v0.1 — et alors il faut un canal de
  commande, pas l'abandon de celui-ci.
- Le démarrage à froid devient inacceptable : l'utilisateur ouvre l'app et attend une minute
  qu'un flux se remplisse. La réponse serait une variante `Snapshot`, pas un handle.
- Trame devient multi-processus, et le canal devient un IPC. La décision **survit** : elle
  dit que la GUI n'obtient que des observations, quel que soit le tuyau.
