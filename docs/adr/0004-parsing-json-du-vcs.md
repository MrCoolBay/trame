# 0004 — Sortie JSON du VCS systematiquement, jamais de scraping

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Puisque le backend VCS est un shell-out (ADR 0003), il faut lire ce que la CLI
repond. Une CLI a deux sorties : celle destinee a l'oeil humain, alignee, coloree,
localisee, tronquee selon la largeur du terminal ; et celle destinee a une machine.

Parser la premiere est un classique du bricolage qui tient six mois puis casse
silencieusement a une mise a jour mineure — et casse en produisant des donnees
plausibles mais fausses, ce qui est le pire mode d'echec pour un outil dont
l'argument est l'auditabilite.

## Decision

Tout appel a `but` demande la sortie machine, sans exception, et celle-ci est
desserialisee dans des types explicites via `serde`. Aucun `split`, aucune regex,
aucun parsing de sortie humaine, nulle part.

En `but` 0.21, le drapeau est **`--format json`** — verifie sur la CLI installee.
`--json` n'existe pas et echoue. La variable d'environnement `BUT_OUTPUT_FORMAT`
fait la meme chose de facon globale ; on prefere le drapeau explicite a chaque
appel, pour qu'une lecture du code montre le contrat sans dependre de
l'environnement.

Corollaire operationnel, inscrit dans `CLAUDE.md` : `but status --format json`
**avant chaque mutation**, pour recuperer les IDs courants. Les IDs de branche et
de fichier sont volatils — ils changent a chaque mutation de l'arbre — donc un ID
lu il y a trois commandes est un ID perime.

## Consequences

- Une commande `but` sans sortie machine est un blocage dur, a remonter en amont
  plutot qu'a contourner par du scraping.
- Les types de desserialisation sont du code a maintenir, et ils documentent au
  passage la surface exacte que Trame utilise — ce qui rend visible le jour ou elle
  grossit sans raison.
- Un changement de schema JSON casse a la compilation ou a la desserialisation, de
  facon bruyante et localisee. C'est le comportement voulu.
- Champs inconnus ignores plutot que refuses : `but` peut enrichir sa sortie sans
  casser Trame. En revanche, un champ attendu et absent est une erreur.

## Alternatives ecartees

- **Parser la sortie humaine.** Casse silencieusement, produit des attributions
  fausses.
- **La sortie machine quand elle existe, scraping en repli.** Le repli deviendrait le
  chemin normal des la premiere commande recalcitrante, et personne ne le saurait.
- **Lire directement les fichiers d'etat de GitButler sur le disque.** Couplage a
  un format interne non documente, sans le contrat de stabilite d'une CLI.

## Ce qui invaliderait cette decision

Rien de previsible. Le passage a `GixBackend` (ADR 0003) rendrait cet ADR sans
objet plutot que faux : il n'y aurait plus de sous-process a parser.
