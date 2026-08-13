# Architecture Decision Records

Un ADR enregistre une decision **structurante et couteuse a defaire**, avec le
contexte qui l'a rendue raisonnable. Il ne documente pas le code — le code se
documente lui-meme — il documente le *pourquoi*, qui disparait de la memoire en
trois mois et n'est reconstituable par personne.

## Index

| # | Decision | Statut |
|---|---|---|
| [0001](0001-macos-uniquement.md) | macOS uniquement, pas d'abstraction cross-platform | Acceptee |
| [0002](0002-aucune-isolation.md) | Aucune isolation : un repertoire de travail unique par projet | Acceptee |
| [0003](0003-gitbutler-en-shell-out.md) | GitButler via la CLI `but`, en shell-out | Acceptee |
| [0004](0004-parsing-json-du-vcs.md) | Sortie JSON du VCS systematiquement, jamais de scraping | Acceptee |
| [0005](0005-acp-en-premier-pty-en-secours.md) | ACP en premier, PTY en secours | Acceptee |
| [0006](0006-acteurs-tokio.md) | Acteurs tokio, un par domaine, aucun etat partage | Acceptee |
| [0007](0007-concurrence-optimiste-read-set.md) | Concurrence optimiste avec validation du read-set | Acceptee |
| [0008](0008-journal-sqlite-append-only.md) | Journal SQLite append-only, global au workspace | Acceptee |
| [0009](0009-licence-fsl-1-1-mit.md) | Licence FSL-1.1-MIT (Fair Source) | ~~Remplacee par [0013](0013-licence-open-source-mit-apache.md)~~ |
| [0010](0010-parallelisme-par-projets.md) | Le parallelisme se fait par projets, pas par sessions | Acceptee |
| [0011](0011-gitlab-self-hosted-en-premier.md) | GitLab self-hosted comme premiere cible de forge | Acceptee |
| [0012](0012-granularite-fichier-en-v0-1.md) | Granularite fichier entier en v0.1, pas de hunks | Acceptee |
| [0013](0013-licence-open-source-mit-apache.md) | Licence open source : MIT OR Apache-2.0 | Acceptee |
| [0014](0014-le-registre-ecrit-sur-disque.md) | Le registre effectue l'ecriture sur disque | Acceptee |
| [0015](0015-canal-admit-borne.md) | Le canal du registre est borne, a 64 | Acceptee |
| [0016](0016-interception-avant-disque-validee.md) | L'interception avant ecriture est possible, et voici ses trous | Acceptee |
| [0017](0017-adaptateur-acp-epingle.md) | L'adaptateur ACP est epingle, et le successeur est refuse | Acceptee |
| [0018](0018-pas-de-diff-dans-stalefile.md) | `StaleFile` ne portera pas de resume du changement | Acceptee |
| [0019](0019-heberger-trame-sur-github.md) | Heberger **Trame lui-meme** sur GitHub (l'ADR 0011 reste valide) | Acceptee |
| [0020](0020-empreinte-uniquement-depuis-fs-read-text-file.md) | L'empreinte d'une lecture ne vient que de `fs/read_text_file` | Acceptee — **invariant** |
| [0021](0021-pas-d-analyse-de-la-sortie-de-grep.md) | Le mode `content` de `Grep` est un angle mort assume, non reconstruit | Acceptee |
| [0022](0022-decoupage-daemon-gui.md) | La GUI observe, elle ne pilote pas : un `Receiver<Observation>` et rien d'autre | Acceptee |
| [0023](0023-gpui-amont-pour-la-gui.md) | `gpui` de l'amont Zed pour la GUI, epingle — `gpui-ce` en echappatoire | Acceptee |
| [0024](0024-pas-de-serveur-mcp-maison.md) | Pas de serveur MCP maison pour l'ecriture : piste documentee, non retenue | Acceptee |
| [0025](0025-ipc-hook-daemon.md) | L'IPC hook vers daemon : socket unix par projet, echec bruyant | Acceptee |

## Quand en ecrire un

Ecris un ADR si la reponse est oui a l'une de ces questions :

- Est-ce que revenir sur ce choix imposerait une reecriture plutot qu'un refactor ?
- Est-ce que ce choix ferme une porte — une plateforme, un modele, un protocole ?
- Est-ce qu'un developpeur competent proposerait spontanement le contraire ?
- Est-ce que le choix parait arbitraire sans son contexte ?

N'ecris **pas** d'ADR pour un choix de nommage, une signature de fonction, un
decoupage de module, ou une decision qui se defait en une apres-midi.

## Format

Voir [`.claude/skills/adr-format/SKILL.md`](../../.claude/skills/adr-format/SKILL.md).
Cinq sections, courtes : Contexte, Decision, Consequences, Alternatives ecartees,
Ce qui invaliderait cette decision.

La derniere section est celle qui compte. Un ADR sans condition de reexamen est un
dogme, pas une decision.

## Statuts

- **Proposee** — en discussion, pas encore appliquee dans le code.
- **Acceptee** — appliquee. Le code la respecte.
- **Remplacee par [XXXX]** — on a change d'avis. **On ne supprime jamais un ADR**,
  on le marque remplace et on ecrit le nouveau. L'historique des decisions a plus
  de valeur que la coherence apparente de l'index.
