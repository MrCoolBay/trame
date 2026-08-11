# 0001 — macOS uniquement

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Trame surveille des systemes de fichiers, stocke des credentials d'agents, et doit
survivre a la fermeture de son interface. Ces trois besoins ont des reponses
excellentes et specifiques sur macOS, et des reponses mediocres et divergentes
partout ailleurs. Ecrire une abstraction cross-platform sur ces trois points
couterait plus cher que le reste du produit reuni, pour un benefice nul a ce stade :
il n'y a pas d'utilisateur Linux qui attend Trame.

## Decision

macOS **uniquement**, sur Apple Silicon. Aucune abstraction cross-platform, aucun
`cfg(target_os)` defensif. On exploite directement FSEvents, le Keychain, launchd
et APFS.

## Consequences

Ce qu'on gagne :

- **FSEvents** — watching recursif natif et efficace, sans limite de watches type
  inotify. Ca compte quand on surveille cinq projets simultanement.
- **Keychain** (`security-framework`) — stockage propre des credentials, pas de
  fichier de conf en clair.
- **launchd LaunchAgent** — le daemon survit a la fermeture de l'UI. Les sessions
  continuent, on rouvre la fenetre plus tard. C'est structurant pour le
  multi-projet.
- **Notifications natives** et item de barre de menus, gratuitement.
- **APFS** — snapshots quasi gratuits si on veut de l'undo au-dela de l'oplog git.
- **Une seule cible** — pas de matrice CI, pas de conditionnels de plateforme, pas
  de bug Windows a diagnostiquer a distance.

Ce que ca coute, et qu'il faut budgeter plutot que decouvrir :

- **Apple Developer Program, ~99 €/an.** Sans signature ni notarisation, Gatekeeper
  bloque l'application. Non negociable des le premier testeur externe.
- **Pas de sandbox, donc pas de Mac App Store.** L'application a besoin d'un acces
  filesystem arbitraire et de spawner des process. Distribution directe en DMG plus
  cask Homebrew.
- **Mises a jour** a cabler soi-meme : Sparkle ou l'updater Tauri.
- **TCC** — l'acces aux dossiers utilisateur demande une autorisation. L'UX de
  premiere ouverture doit etre soignee.

## Alternatives ecartees

- **Cross-platform des le depart.** Triple le cout des trois modules qui touchent
  le systeme, pour zero utilisateur supplementaire aujourd'hui.
- **Un coeur portable avec des adaptateurs par plateforme.** C'est deja le cas de
  fait : `trame-core`, `trame-journal` et `trame-registry` ne touchent pas au
  systeme. Le formaliser en traits maintenant serait de la couture speculative — et
  contrairement aux coutures de l'ADR 0005, celle-ci n'a aucun usage en v0.1.

## Ce qui invaliderait cette decision

Une demande reelle et repetee d'utilisateurs Linux, une fois le produit valide.
Dans ce cas, le portage ne concernerait que le watcher, le stockage de credentials
et le mode daemon — le coeur est deja portable sans effort. Ce n'est pas un
argument pour abstraire aujourd'hui, c'est un argument pour ne pas s'inquieter.
