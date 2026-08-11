# 0014 — Le registre effectue l'ecriture sur disque

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

L'invariant 2 dit : *le registre est le point de passage unique des ecritures. Rien
n'ecrit a cote.* Jusqu'ici, le registre ne faisait que **rendre un verdict** ; il
restait a l'appelant d'ecrire. C'est insuffisant, pour une raison qui n'est pas
theorique :

- Un appelant peut oublier d'ecrire, ecrire un contenu different de celui admis, ou
  ecrire avant d'appeler `admit`. Le journal dirait alors quelque chose de faux, ce qui
  est **pire que pas de journal** pour un outil dont l'argument est l'auditabilite.
- La sequence, l'empreinte enregistree et le contenu reel du disque peuvent divergerment
  silencieusement. Rien ne le detecterait.
- Un invariant qui repose sur la discipline de l'appelant n'est pas un invariant, c'est
  une convention.

La validation ACP rend en plus cette forme **naturelle** : l'agent ne s'ecrit pas
lui-meme, il demande au client d'ecrire (`fs/write_text_file`). Le registre est donc
deja, protocolairement, celui a qui l'ecriture est deleguee.

## Decision

**L'admission inclut l'ecriture.** Le registre hashe le contenu, rend son verdict,
ecrit le fichier, met son etat a jour et journalise — dans cet ordre, dans le meme
acteur, pour un seul message `Admit`.

Consequences sur l'API, a appliquer au cablage (phase 3) :

- Le registre est construit avec la **racine absolue du projet**. Les chemins des
  messages restent relatifs ; le registre les resout et **refuse tout chemin qui sort
  de la racine** (`CoreError::PathOutsideProject`) — il ne peut rien garantir sur ce
  qu'il ne voit pas.
- `admit` devient faillible : `Result<Verdict, RegistryError>`, avec une variante
  d'echec d'ecriture. Un verdict rendu sans que l'ecriture ait eu lieu serait un
  mensonge.
- **L'etat n'est mis a jour que si l'ecriture a reussi.** Sinon le registre croirait le
  fichier modifie et perimerait a tort les lectures des autres sessions.
- Le journal n'enregistre l'ecriture qu'apres son succes. Une ecriture echouee est
  tracee via `tracing`, pas comme une ligne de `writes`.

## Consequences

- L'invariant devient **structurel** : il n'existe plus de chemin par lequel une
  ecriture admise ne serait pas effectuee, ni d'ecriture effectuee sans admission.
- **L'acteur fait de l'I/O sur son chemin critique.** C'est le cout reel de cette
  decision. Il est accepte pour deux raisons : l'ecriture d'un fichier source est de
  l'ordre de la centaine de microsecondes, et la serialisation est de toute facon ce
  qu'on veut — deux ecritures du meme fichier dans un ordre indetermine seraient un
  bug. `tokio::fs` evite de bloquer le runtime.
- Si le profil de latence devenait un probleme, la sortie n'est pas de rendre
  l'ecriture a l'appelant : c'est de deplacer l'I/O dans une tache dediee **par
  projet**, qui conserverait l'ordre.
- Les ecritures **hors-bande** ne sont pas couvertes et ne peuvent pas l'etre :
  `sed -i`, hooks git, formatters, build. Elles sont rattrapees par FSEvents, jamais
  admises. Le registre reste le point de passage unique des ecritures **d'agents**, ce
  qui est la portee reelle de l'invariant. A afficher comme tel, pas a laisser croire
  autre chose.
- En mode PTY (`can_intercept_writes == false`), l'agent ecrit lui-meme et le registre
  ne peut qu'observer. **L'interface doit afficher cette degradation** : un utilisateur
  qui croit avoir la garantie est dans une situation pire que sans outil.

## Alternatives ecartees

- **Le registre rend un verdict, l'appelant ecrit.** L'etat actuel. L'invariant repose
  sur la discipline de chaque site d'appel, et rien ne detecte une divergence entre le
  contenu admis et le contenu ecrit.
- **Le registre rend un jeton que l'appelant doit presenter pour ecrire.** Deplace le
  probleme sans le resoudre : il faudrait un second gardien pour verifier les jetons.
- **Verifier apres coup, par relecture du disque, que le contenu correspond.** Une
  lecture par ecriture sur le chemin chaud, pour detecter tard un probleme qu'on peut
  rendre impossible.

## Ce qui invaliderait cette decision

Un profil ou l'I/O sequentialisee par projet devient le goulot mesure — pas suppose. A
deux a cinq sessions par projet, chacune ecrivant quelques fichiers par minute, c'est
plusieurs ordres de grandeur en dessous. La reponse serait alors une tache d'ecriture
dediee par projet, jamais un retour de l'ecriture a l'appelant.
