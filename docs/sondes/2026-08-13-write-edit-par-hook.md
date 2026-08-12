# Sonde 5 — refuser `Write`/`Edit` par hook fait-il tomber l'épinglage à 0.16.2 ?

- **Date** : 2026-08-13
- **Question** : sur `@agentclientprotocol/claude-agent-acp` 0.66.0, qui ne retire plus les
  outils d'écriture natifs, est-ce qu'un `deny` en `PreToolUse` sur `Write` et `Edit` fait
  basculer l'agent sur les outils ACP — et donc restaure l'invariant d'admission ?
- **Réponse : non.** Et c'est établi **sans un seul jeton**, en une capture d'argv.
- **Mais une autre voie existe**, dont le premier maillon est mesuré.

## 1. Pourquoi la bascule espérée est impossible

`argv` réellement passé au binaire `claude` par 0.66.0, capturé aujourd'hui avec un faux
`claude` qui n'écrit que ses arguments :

```
--output-format stream-json --verbose --input-format stream-json
--permission-prompt-tool stdio
--disallowedTools AskUserQuestion          ← Write et Edit ne sont PAS retires
--tools default
--setting-sources=user,project,local
--permission-mode default --allow-dangerously-skip-permissions
--include-partial-messages --session-id=… --replay-user-messages
```

**Il n'y a ni `--mcp-config` ni `--allowedTools`.** Donc **aucun serveur MCP `acp`**, donc
**aucun outil `mcp__acp__Read` / `Write` / `Edit` n'existe** dans cette session.

À comparer avec 0.16.2, épinglée, capturée dans les mêmes conditions :

```
--allowedTools mcp__acp__Read
--disallowedTools AskUserQuestion,Read,Write,Edit
--mcp-config {"mcpServers":{"acp":{"type":"sdk","name":"acp"}}}
```

La conséquence est nette : sur 0.66.0, les **seuls** outils de fichiers de l'agent sont les
natifs, qui écrivent directement sur le disque. Les refuser par hook ne le fait pas basculer
sur autre chose — **ça le laisse sans aucun chemin d'écriture.** Un agent qui ne peut plus
écrire du tout n'est pas un agent dont on a restauré l'invariant, c'est un agent cassé.

L'hypothèse du plan était donc fausse, et elle l'était pour une raison qu'aucun raisonnement
sur les hooks ne pouvait révéler : elle portait sur la présence d'outils, pas sur le
comportement des hooks.

**Le contrôle qui rend la mesure fiable** : la même capture, sur la même machine, avec le même
harnais, rend l'argv attendu pour 0.16.2 — dont `--mcp-config` et le retrait de
`Read,Write,Edit`. Le dispositif sait donc voir un `--mcp-config` quand il y en a un. Sans ce
contrôle, « pas de `--mcp-config` » aurait pu être un défaut du harnais.

> Note de méthode : `session/new` rend une erreur `-32603 Query closed before response
> received`, parce que le faux `claude` sort immédiatement. C'est attendu et sans effet — la
> ligne de commande est construite **avant** que le binaire soit lancé, et c'est elle qu'on
> mesure.

## 2. La voie qui reste, et son premier maillon mesuré

`session/new` accepte un champ `mcpServers`. **Les deux versions le transmettent à la CLI.**

Sur 0.66.0, en déclarant un serveur nommé `trame` :

```
--mcp-config {"mcpServers":{"trame":{"type":"stdio","command":"…","args":[],"env":{}}}}
```

Sur 0.16.2, le nôtre **coexiste** avec celui de l'adaptateur :

```
--mcp-config {"mcpServers":{"trame":{"type":"stdio",…},"acp":{"type":"sdk","name":"acp"}}}
```

D'où le candidat, qui n'est **pas** celui du plan :

> **Trame déclare son propre serveur MCP portant un outil d'écriture, et refuse les outils
> natifs par hook.** L'agent n'a alors qu'un seul chemin d'écriture, et c'est le nôtre — quelle
> que soit la version de l'adaptateur, et sans dépendre de son comportement de retrait d'outils.

Ce serait une indépendance plus forte que l'épinglage actuel : elle ne repose plus sur un
détail non spécifié d'un paquet tiers, mais sur une capacité **documentée** du protocole.

### Ce qui n'est pas établi, et qu'il ne faut pas supposer

- **Que l'agent utilise effectivement l'outil déclaré.** Le `--mcp-config` prouve que la
  déclaration arrive à la CLI, rien de plus. Il faut une session réelle pour voir l'agent
  choisir `mcp__trame__write` plutôt que d'abandonner.
- **Que ce serveur MCP soit viable dans notre architecture.** Un serveur `stdio` est un
  sous-processus : Trame en lancerait un par session, qui devrait parler au daemon par la même
  IPC que les hooks. Ça double la surface du chemin d'écriture au lieu de la déplacer.
- **Le coût sur 0.16.2.** Y ajouter notre serveur alors que `acp` fait déjà le travail
  n'apporte rien tant que l'épinglage tient.

## 3. Ce que ça change au plan des hooks

Rien sur les deux objets qui comptent, et tout sur le troisième :

| Objectif | État |
|---|---|
| Fermer le trou lecture (`Grep`/`Glob`) | **Inchangé** — `PostToolUse` reste la voie, sur 0.16.2 |
| Fermer le trou écriture `Bash` | **Inchangé** — refus en `PreToolUse`, motif conservateur |
| Lever l'épinglage à 0.16.2 | **Retiré des bénéfices attendus.** La voie hook seule ne le fait pas |

L'épinglage reste donc ce qu'il est : un sursis, pas une solution
([ADR 0017](../adr/0017-adaptateur-acp-epingle.md)). La dette n'est pas payée ; elle est
mieux comprise, et la piste qui pourrait la payer est nommée.

**Ce qui aurait coûté cher** : implémenter les hooks en croyant qu'ils lèvent l'épinglage, puis
découvrir à la migration que l'agent n'a plus de chemin d'écriture. Une heure de capture d'argv
contre cette découverte-là.

## 4. Reproduire

```sh
npm install -g @agentclientprotocol/claude-agent-acp   # 0.66.0
/tmp/sonde-066/capture.mjs    # argv de 0.66.0, mcpServers vide
/tmp/sonde-066/capture2.mjs   # 0.66.0 avec un serveur MCP declare
/tmp/sonde-066/capture3.mjs   # le meme, contre 0.16.2 (claude-code-acp)
```

Aucune authentification, aucun jeton : `CLAUDE_CODE_EXECUTABLE` pointe sur un faux `claude` qui
n'écrit que son `argv`. Le garde-fou anti-imbrication ne s'applique pas, puisque le vrai binaire
n'est jamais lancé.
