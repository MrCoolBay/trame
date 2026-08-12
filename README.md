# Trame

Plusieurs agents de code en parallele, dans **un seul repertoire de travail** par projet,
avec une coordination explicite et observable au lieu de silencieuse.

macOS. Rust. Local-first : un binaire, pas de compte, pas de serveur, pas de cloud.

> **Etat : en construction, utilisable pour observer, pas encore pour travailler.**
> Ce que ca fait aujourd'hui et ce que ca ne fait pas est detaille plus bas, sans arrondi.

## La these

Tous les outils concurrents isolent chaque agent — un worktree git ou une copie de
repertoire par session. L'isolation supprime les collisions, mais elle rend aussi la
coordination **impossible** : chaque agent est aveugle aux autres par construction, et aucune
couche ajoutee par-dessus n'y remedie.

Trame fait le pari inverse : **repertoire partage, coordination appliquee.**

> Quand l'agent A s'apprete a ecrire, si un fichier qu'il a **lu** a ete modifie depuis par
> une autre session, A raisonne sur un monde qui n'existe plus. Trame le detecte et
> **l'en informe**.

Appliquee, pas suggeree : ce n'est pas une consigne dans un prompt qu'un agent peut oublier,
c'est un **point de passage**. Les ecritures de l'agent transitent par le registre
d'admission, qui rend un verdict et effectue l'ecriture lui-meme.

Le mode d'echec que ca attrape ne produit **aucune collision d'ecriture** :

```
1. Session A lit auth.rs, memorise la signature de verify_token()
2. Session B modifie auth.rs, renomme verify_token() -> validate_token()
3. Session A ecrit handlers.rs, appelle verify_token()

Deux fichiers differents. Un verrou par fichier ne voit rien. L'arbre est casse.
```

C'est le mecanisme central, et le seul du produit qui n'existe nulle part ailleurs — trop
tard au niveau de la forge, ou le code est deja ecrit ; impossible au niveau du systeme de
fichiers, ou les agents sont isoles.

**Trame ne bloque pas, il informe.** L'agent averti relit et s'adapte. Ce comportement a ete
mesure sur de vraies sessions Claude Code, et la mesure est explicitement marquee comme
**non discriminante** : scenario court, outils de recherche fermes, changement tres lisible.
A rejouer sur un cas realiste avant d'en conclure quoi que ce soit
([ADR 0018](docs/adr/0018-pas-de-diff-dans-stalefile.md)).

## La portee reelle de l'invariant, et ses deux trous

Un filet dont on ignore les trous est pire qu'un filet dont on les connait. Les voici, tels
que mesures :

**Ce qui est couvert.** Les ecritures faites par les outils de fichiers de l'agent — `Write`,
`Edit`, `NotebookEdit` — passent par le registre **avant le disque**. Valide sur deux sessions
Claude Code reelles : elles ont demande a ecrire, nous avons refuse, rien n'a atteint le
disque ([ADR 0016](docs/adr/0016-interception-avant-disque-validee.md)).

**Trou n° 1 — l'ecriture par le shell.** Un `echo > fichier` ou un `sed -i` dans un `Bash`
echappe a l'admission. Le watcher FSEvents le **constate apres coup** : il n'empeche rien,
mais il empeche le registre de devenir *faux*. Ces ecritures sont journalisees comme
`observed`, **sans verdict**, et l'interface les affiche comme telles — jamais comme des
ecritures admises.

**Trou n° 2 — la lecture par un autre outil, et c'est le pire.** Retirer `Read` ne force pas
l'agent a passer par nous : `Grep`, `Glob` et `Bash` restent disponibles. Une lecture par l'un
d'eux **n'entre pas dans le read-set** — et une lecture manquante ne laisse pas un trou dans
le journal, elle supprime la **condition** d'un avis. Le mecanisme central ne se declenche
pas, et rien ne l'indique.

La voie de sortie est mesuree mais **pas encore construite** : les hooks `PreToolUse` et
`PostToolUse` de la CLI exposent ce qu'il faut
([sonde 3](docs/sondes/2026-08-12-postooluse.md)). En attendant, les manches de mesure
**ferment** ces outils, ce qui est acceptable pour une experience et **pas pour un produit** :
un agent prive de recherche sur un vrai codebase est un agent degrade.

**Ce qu'aucun registre ne peut attraper** : deux agents qui se contredisent sans avoir lu le
meme fichier. Seul filet reel, le compilateur et les tests.

## Etat

Phases 0 a 4.1 livrees. **146 tests.** Ce qui existe :

| Composant | Etat |
|---|---|
| `trame-journal` — journal SQLite append-only, six tables | ✅ ecritures reelles, provenance et origine |
| `trame-registry` — l'acteur d'admission, **le coeur** | ✅ verdicts `Clean` / `StaleRead`, ecrit sur disque |
| `trame-agent` — ACP sur stdio, interception avant disque | ✅ valide en live ; `PtyBackend` en squelette honnete |
| `trame-daemon` — pilote de session, watcher FSEvents | ✅ la chaine complete, jusqu'a l'avis pose devant le prompt |
| `trame-tui` — interface terminal | ✅ panneaux, flux, verdicts, degradation |
| `trame-gui` — application desktop `gpui` | ✅ meme perimetre d'affichage |
| `trame-vcs` — GitButler en shell-out | ⏳ frontiere posee, contenu a venir |
| Supervisor multi-projet | ⏳ cadre, pas ecrit |

Ce qui **n'est pas** fait : l'attribution des modifications aux branches virtuelles, le
multi-projet reel, la fermeture des deux trous ci-dessus, et tout ce qui touche a la
distribution — signature, notarisation, mise a jour.

Le cadrage complet est dans [`docs/concept.md`](docs/concept.md), les decisions et leurs
raisons dans [`docs/adr/`](docs/adr/), et ce qui a ete mesure plutot que suppose dans
[`docs/sondes/`](docs/sondes/).

## Non-objectifs

Refuses explicitement, pour que personne n'ouvre une change request qui sera fermee :
Windows et Linux, un editeur de code embarque, un modele ou un agent propietaire, un mode
SaaS, et **toute forme d'isolation** — worktrees, conteneurs, copy-on-write, microVMs. La
derniere est la condition de possibilite du produit, pas une preference.

## Essayer

```sh
just tui-scenario /tmp/trame-demo    # le scenario canonique, en terminal
just gui-scenario /tmp/trame-demo    # le meme, en application desktop
```

Les deux jouent le scenario canonique par le **vrai** registre, sans agent : les verdicts
affiches sont ceux qu'il rend. Pendant que ca tourne, depuis un autre terminal :

```sh
echo '// ajoute a la main' >> /tmp/trame-demo/notes.txt
```

La ligne apparait en **hors-bande, sans verdict** — le watcher l'a constatee apres coup,
personne ne l'a admise.

## Developper

```sh
just check     # compile tout, tests inclus
just test      # la suite complete
just lint      # fmt + clippy, zero warning tolere
just ci        # ce que la CI verifie, en local
just canari    # ★ verifie que l'adaptateur ACP retire toujours les outils d'ecriture
just fumee     # ★ ouvre la GUI et exige qu'une image soit reellement produite
```

Prerequis : macOS sur Apple Silicon, une toolchain Rust stable, la CLI
[GitButler](https://gitbutler.com) (`but`) sur le `PATH`, et Node pour l'adaptateur ACP
(`npm install -g @zed-industries/claude-code-acp@0.16.2` — version **epinglee**, voir
l'[ADR 0017](docs/adr/0017-adaptateur-acp-epingle.md)).

Xcode complet n'est **pas** requis : la GUI compile ses shaders au lancement plutot qu'au
build ([ADR 0023](docs/adr/0023-gpui-amont-pour-la-gui.md)).

Trame appelle `but` comme dependance externe ; il ne l'embarque pas. Ce depot est en
**workspace mode GitButler** : on n'utilise ni `git commit`, ni `git add`, ni `git push` —
voir [`AGENTS.md`](AGENTS.md).

## Contribuer

Les regles du projet, les invariants d'architecture et les decisions prises sont dans
[`AGENTS.md`](AGENTS.md). Les lire avant d'ouvrir une change request fait gagner un
aller-retour — en particulier les non-objectifs ci-dessus et les dix invariants.

Deux choses sont non negociables : `just lint` sans le moindre warning, et `just test` vert.
Le reste se discute.

Une regle de methode, nee de quatre fois le meme bug : **tout mecanisme qui traverse une
frontiere — protocole tiers, systeme de fichiers, terminal — doit avoir ete vu tourner pour
de vrai avant d'etre considere comme acquis.** Un test etablit la coherence avec ce qu'on
croit de la frontiere, jamais ce que la frontiere fait. Detail dans `AGENTS.md`.

## Licence

Trame est **open source**, sous double licence au choix :

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

Usage, modification, fork, redistribution y compris commerciale : tout est permis. C'est la
convention de l'ecosysteme Rust — MIT pour la concision, Apache-2.0 pour la clause de brevet
explicite que MIT n'a pas.

Sauf mention contraire de votre part, toute contribution que vous soumettez intentionnellement
pour inclusion dans Trame est offerte sous ces memes deux licences, sans condition
supplementaire. **Il n'y a pas de CLA.**

Le raisonnement derriere ce choix — et pourquoi le projet a quitte la FSL-1.1-MIT — est dans
l'[ADR 0013](docs/adr/0013-licence-open-source-mit-apache.md).

> Note : la CLI GitButler (`but`) est un logiciel distinct, sous FSL-1.1-MIT. Trame l'appelle
> comme prerequis externe et ne l'embarque **jamais**. Si vous redistribuez Trame,
> n'empaquetez pas `but` avec.
