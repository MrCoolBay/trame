# 0003 — GitButler via la CLI `but`, en shell-out

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Trame a besoin de branches virtuelles : un repertoire de travail unique ou les
changements sont **etiquetes** plutot qu'isoles. Ce modele est conceptuellement
superieur au worktree pour notre cas, parce qu'il n'y a pas de divergence dans le
temps — donc **le conflit n'a pas d'endroit ou naitre** — et parce que l'assignation
au hunk se fait en continu, pendant que le contexte est frais.

GitButler implemente exactement ca, avec le hunk locking, le restack automatique
sur les piles de branches, les conflits *first-class* et un oplog. C'est plusieurs
annees de travail, dont un portage de Git en Rust qui passe la suite de tests du
Git C.

La surface dont Trame a reellement besoin fait environ sept commandes.

## Decision

`ButBackend` appelle la CLI `but` en sous-process, derriere un trait `VcsBackend`.
`but` est traite comme une **dependance externe installee par l'utilisateur**,
jamais vendorisee — de la meme facon qu'un orchestrateur d'agents ne livre pas
Claude Code avec lui.

Si la CLI est absente, Trame s'arrete et le dit. Il ne bascule **jamais** sur du
git nu : le modele de branches virtuelles n'a pas d'equivalent en git, et simuler
l'un avec l'autre produirait des attributions fausses — donc un journal qui ment,
ce qui est pire que pas de journal.

## Consequences

- On recupere l'oplog et `but undo` gratuitement.
- Le cout d'integration est de l'ordre de la journee, contre six a dix-huit mois
  pour une reimplementation, sur ce qui est pour Trame une **commodite** : la valeur
  du produit est dans le registre d'admission, pas dans le VCS.
- Un sous-process par mutation. Negligeable : les mutations sont rares a l'echelle
  d'une session d'agent, et l'admission d'ecriture ne passe pas par `but`.
- Dependance a la stabilite de la CLI et de sa sortie JSON (ADR 0004). On
  epingle une version minimale et on echoue proprement en dessous.
- **Question de licence non reglee.** GitButler est sous FSL-1.1-MIT, avec une
  clause de non-concurrence sur les usages commerciaux. Trois pistes, a faire
  valider par quelqu'un qui lit vraiment les licences : traiter `but` comme
  dependance externe non vendorisee (ce que fait cet ADR) ; le fait que la clause
  visee les produits *commerciaux* ; et surtout la **conversion FSL vers MIT a deux
  ans**, qui rend les versions 2023-2024 du coeur reutilisables sans restriction.
  La troisieme piste est la plus solide et la moins exploree.

  La licence de Trame **ne regle pas** ce point et ne l'a jamais regle : ce sont deux
  questions independantes. C'etait vrai sous FSL, ca reste vrai sous MIT/Apache
  (ADR 0013). La non-vendorisation est ce qui porte l'analyse.

  Point neuf apporte par l'ADR 0013 : sous licence permissive, un tiers peut
  redistribuer Trame commercialement. S'il empaquetait `but` avec, c'est **lui** qui
  se confronterait a la clause de GitButler. Raison de plus pour que `but` reste un
  prerequis documente et jamais un binaire embarque.

## Alternatives ecartees

- **`gix` (gitoxide) natif.** L'ecosysteme est mur et ce serait la sortie propre
  cote licence, mais reimplementer les branches virtuelles, le hunk locking et le
  restack est un projet en soi. Reste une option long terme : le trait
  `VcsBackend` existe pour que ce soit un ajout et non une reecriture.
- **Lier la bibliotheque GitButler en dependance Rust.** Aggrave la question de
  licence au lieu de la reduire, et couple Trame a des API internes non publiques.
- **git nu plus une convention de branches.** Ramene la divergence dans le temps,
  donc les conflits, donc exactement ce que le modele virtual branches supprime.

## Ce qui invaliderait cette decision

Une instabilite repetee du schema JSON entre versions, ou une reponse juridique
qui ferme les trois pistes ci-dessus. Dans les deux cas, `GixBackend` derriere le
meme trait.
