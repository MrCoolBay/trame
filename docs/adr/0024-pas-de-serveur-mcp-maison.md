# 0024 — Pas de serveur MCP maison pour l'écriture. Piste documentée, non retenue

- **Statut** : Acceptée — **décision de ne pas faire**, avec ses déclencheurs de réexamen
- **Date** : 2026-08-13
- **Mesure d'origine** : [sonde 5](../sondes/2026-08-13-write-edit-par-hook.md)
- **Ne remplace pas** l'[ADR 0017](0017-adaptateur-acp-epingle.md) : l'épinglage tient.

## Contexte

La [sonde 5](../sondes/2026-08-13-write-edit-par-hook.md) a démoli l'hypothèse qui portait la
sortie de notre plus vieille dette. Refuser `Write` et `Edit` par hook sur l'adaptateur 0.66.0
ne fait pas basculer l'agent sur les outils ACP : **il n'y en a pas.** L'`argv` passé à la CLI
n'a ni `--mcp-config` ni `--allowedTools`, donc aucun serveur MCP `acp`, donc aucun
`mcp__acp__Write`. Refuser les natifs ne redirige pas l'agent, ça le prive de tout chemin
d'écriture.

Mais la même sonde a mesuré autre chose, sans le chercher : **les deux versions transmettent à
la CLI un serveur MCP déclaré par le client** dans `session/new`. Sur 0.16.2, le nôtre coexiste
même avec celui de l'adaptateur :

```
--mcp-config {"mcpServers":{"trame":{"type":"stdio",…},"acp":{"type":"sdk","name":"acp"}}}
```

D'où une piste réelle, et séduisante : **Trame porte son propre outil d'écriture, et refuse les
outils natifs par hook.** L'agent n'aurait qu'un seul chemin d'écriture, le nôtre, quelle que
soit la version de l'adaptateur. L'invariant ne reposerait plus sur un détail non spécifié d'un
paquet tiers mais sur une capacité **documentée** du protocole.

## Décision

**On ne construit pas ce serveur MCP.** Pas maintenant, et pas comme réponse à l'épinglage.
La piste est enregistrée ici avec ses deux objections, pour qu'elle soit reprise sur ses mérites
le jour où un déclencheur tombe — et pas reprise par enthousiasme dans trois mois.

## Les deux objections, et elles sont dirimantes aujourd'hui

### 1. Ça doublerait la surface du chemin d'écriture au lieu de la déplacer

`fs/write_text_file` ne disparaîtrait pas : c'est le chemin ACP, il fonctionne, et il est déjà
testé de bout en bout. Notre outil MCP viendrait **s'ajouter**. Deux voies d'admission, donc :

- deux chemins à maintenir en cohérence — même normalisation par `ProjectRoot`, même ordre
  « écrire puis acquitter », même traitement du `Drop` qui refuse ;
- deux chemins à tester, y compris dans leurs interactions : que se passe-t-il si l'agent
  utilise les deux dans le même tour ?
- deux chemins à surveiller, alors que l'invariant 2 tire sa force d'être un **point de passage
  unique**.

Un invariant qui a deux portes n'est pas deux fois plus solide. Il est aussi solide que la porte
la moins bien gardée, et il coûte deux fois plus cher à garder.

S'ajoute le coût de forme : un serveur MCP `stdio` est un **sous-processus**. Trame en
lancerait un par session, qui devrait parler au daemon par la même IPC que les hooks
([ADR 0025](0025-ipc-hook-daemon.md)). Ça fait un processus de plus dans la chaîne d'écriture —
le chemin dont on veut précisément qu'il soit court et vérifiable.

### 2. Rien ne dit que l'agent choisirait notre outil

C'est l'objection qui tue la piste dans sa forme naïve. Un agent qui connaît `Write` depuis son
entraînement n'a aucune raison de préférer `mcp__trame__write`, dont le nom ne lui dit rien.
La sonde 5 n'a mesuré que la **plomberie** — la déclaration arrive à la CLI — et rien du
**choix** de l'agent.

Et la parade évidente reconstruit le problème : forcer la main en refusant les natifs par hook,
c'est remplacer une dépendance comportementale par une autre. Aujourd'hui nous dépendons du fait
que l'adaptateur retire `Write` et `Edit` ; nous dépendrions du fait que l'agent, privé de ses
outils habituels, **trouve et adopte** le nôtre plutôt que de renoncer, de contourner par `Bash`,
ou de tourner en rond.

La sonde 2 a montré un agent obéissant : refusé sur `Bash`, il est passé à son outil de fichier.
C'est encourageant et ça ne prouve rien ici — il basculait vers un outil qu'il **connaissait**.
Basculer vers un outil inventé par nous est une autre affaire, et ça se mesure sur plusieurs
sessions avant d'y adosser un invariant.

**Le même piège qu'on vient d'éviter, une porte plus loin** : une hypothèse sur le comportement
d'un tiers, plausible, non mesurée, sur laquelle on adosserait le mécanisme central.

## Conséquences

- L'épinglage à 0.16.2 reste ce qu'il est : **un sursis, pas une solution**. La dette est ouverte
  et mieux comprise.
- Le plan des hooks perd un objectif et garde les deux autres — voir le tableau corrigé dans
  l'[ADR 0025](0025-ipc-hook-daemon.md). C'est là qu'il faut regarder avant de réintroduire
  « lever l'épinglage » dans une liste de bénéfices.
- Le canari de l'ADR 0017 devient plus important, pas moins : c'est lui qui dira que la version
  épinglée a cessé de faire ce qu'on attend.

## Alternatives écartées

- **Construire le serveur MCP et abandonner `fs/write_text_file`.** Ça répondrait à
  l'objection n° 1 en supprimant une des deux portes. Mais ça remplacerait un chemin mesuré, avec
  un run live à l'appui ([ADR 0016](0016-interception-avant-disque-validee.md)), par un chemin
  dont on ne sait pas si l'agent l'emprunte. On ne troque pas du mesuré contre du supposé.
- **Le construire pour un autre harnais que Claude Code** — Codex, Gemini CLI. C'est l'argument
  le plus solide en sa faveur, et il n'est pas d'actualité : aucun autre harnais n'est branché.
  Le jour où l'un l'est et n'expose pas d'outils de fichiers côté client, c'est un déclencheur
  (ci-dessous), pas une anticipation.
- **Le construire « au cas où », en parallèle du chemin ACP.** C'est l'objection n° 1 assumée
  comme prix d'une assurance. Refusé : deux chemins d'admission dont un jamais emprunté, c'est
  du code non exercé sur le trajet le plus critique du produit.

## Déclencheurs de réexamen

**L'épinglage tient tant qu'aucun de ces trois n'est déclenché.** Chacun est observable ; aucun
ne demande de jugement.

1. **Une faille dans 0.16.2** — sécurité, ou un bug qui casse l'interception — qui rende la
   version épinglée intenable. On perd alors le sursis, et il faut un autre mécanisme.
2. **Un harnais qu'on veut supporter et qui n'expose pas d'outils de fichiers côté client.**
   Codex ou Gemini CLI sans équivalent de `fs/write_text_file` : notre propre outil devient la
   seule voie possible, et l'objection n° 1 tombe — il n'y aurait plus deux portes, une seule.
3. **Une rupture du canari** (`crates/trame-agent/tests/interception_canary.rs`) : l'adaptateur
   épinglé cesse de retirer `Write` et `Edit`. Le sursis est fini.

Dans les trois cas, la réouverture commence par **mesurer l'objection n° 2** : une manche où
l'agent, privé de ses outils natifs, se voit offrir le nôtre. Tant que ce chiffre n'existe pas,
la piste reste une piste.

## Ce qui invaliderait cette décision

Une mesure qui répond à l'objection n° 2 : plusieurs sessions réelles où l'agent adopte un outil
d'écriture déclaré par le client sans dégradation observable — pas de renoncement, pas de
contournement par `Bash`, pas de tours perdus. Ce serait la première fois que le comportement
serait établi plutôt que supposé, et l'arbitrage se reposerait alors sur l'objection n° 1 seule,
qui est un coût et non une inconnue.
