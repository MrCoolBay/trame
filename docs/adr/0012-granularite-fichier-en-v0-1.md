# 0012 — Granularite fichier entier en v0.1, pas de hunks

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Le registre doit decider si deux acces au meme fichier se recouvrent. Trois
granularites possibles :

- **Fichier entier** — trop grossier en theorie : on signale un recouvrement des que
  deux sessions touchent le meme fichier, meme a mille lignes d'ecart.
- **Ligne** — trop fragile : les numeros derivent des qu'une edition a lieu au-dessus.
- **Hunks plus quelques lignes de contexte** — le bon niveau conceptuel, et de loin
  le plus cher. La difficulte n'est pas de decouper en hunks, c'est de **projeter les
  anciennes plages a travers les diffs successifs** pour comparer dans un referentiel
  commun. C'est un probleme en soi.

Le risque produit numero un du projet est le taux de faux positifs. Or on ne connait
pas ce taux : personne n'a jamais mesure a quelle frequence deux agents touchent le
meme fichier a des endroits sans rapport, sur un vrai depot.

## Decision

**v0.1 : fichier entier**, plus une fenetre temporelle. Pas de suivi de hunks.

Corollaire : `Verdict::DisjointWrite` et `Verdict::Overlap` ne sont **pas implementes**.
Les variantes existent dans le type, la logique renvoie `Clean`, avec un `TODO`
explicite. Les deux distinctions n'ont de sens qu'a une granularite sous-fichier — les
definir maintenant fait que les ajouter en v0.4 sera un `match` a completer et non un
changement de type public qui casse le journal, l'interface et les tests.

Le raisonnement en une ligne : **fichier plus fenetre temporelle donne 90 % de la
valeur pour 5 % du travail**. On raffine apres avoir mesure son propre taux de faux
positifs, pas avant.

## Consequences

- Le mecanisme qui compte — la validation du read-set (ADR 0007) — **ne depend pas de
  la granularite**. Le scenario canonique (A lit `auth.rs`, B ecrit `auth.rs`, A ecrit
  `handlers.rs`) se detecte parfaitement au fichier entier. C'est ce qui rend ce
  compromis acceptable : il ne degrade pas la these, il degrade un raffinement.
- Les faux positifs attendus sont du type « B a touche `auth.rs` a la ligne 1200, ce
  qui n'a aucun rapport avec ce que A avait lu ». Comme rien n'est bloque en v0.1,
  le cout d'un faux positif est un avis inutile dans le contexte de l'agent — pas une
  session cassee.
- Il faut **instrumenter** le taux de faux positifs des le depart. Sans mesure, la
  decision de passer aux hunks se prendra a l'intuition, ce qui est exactement ce que
  cet ADR cherche a eviter.
- La granularite fichier rend le hash blake3 trivial : un hash par fichier, calcule a
  la lecture et a l'admission. Jamais l'arbre entier.
- Deux cadrans sont disponibles avant de payer les hunks : le filtre du read-set
  (seules les lectures substantielles) et la fenetre de decroissance (dix minutes).
  Les tourner coute des heures ; les hunks coutent des semaines.

## Alternatives ecartees

- **Hunks des la v0.1.** Des semaines de travail sur la projection de plages a travers
  les diffs, pour resoudre un probleme dont on n'a pas mesure l'ampleur. Et le risque
  de ne jamais livrer la v0.1.
- **Granularite ligne.** Fragile a l'edition, et faussement precise : deux
  modifications a trois lignes d'ecart sont probablement liees.
- **Ne pas definir `DisjointWrite` et `Overlap` du tout.** Les ajouter plus tard
  changerait un enum public que le journal persiste, l'interface affiche et les tests
  couvrent. Le cout de les declarer aujourd'hui est nul.

## Ce qui invaliderait cette decision

Un taux de faux positifs mesure trop eleve **apres** avoir tourne les deux cadrans
disponibles. C'est le declencheur explicite du passage aux hunks, prevu en v0.4. La
decision est donc datee par construction : elle attend une mesure, pas une opinion.
