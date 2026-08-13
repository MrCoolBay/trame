# 0017 — L'adaptateur ACP est epingle, et le successeur est refuse

- **Statut** : Acceptee
- **Date** : 2026-08-12

## Contexte

`@zed-industries/claude-code-acp` 0.16.2 est marque **deprecie** par npm, au profit de
`@agentclientprotocol/claude-agent-acp`. La migration a donc ete demandee, avec
re-verification du comportement dont depend l'invariant d'interception.

Rappel de ce dont il s'agit (ADR 0016) : l'interception avant disque **ne repose pas sur
une garantie du protocole ACP**. Elle repose sur un choix d'implementation, non specifie,
de l'adaptateur — quand le client annonce `fs.writeTextFile`, l'adaptateur retire `Write`
et `Edit` des outils de l'agent, qui n'a alors plus que le chemin passant par nous.

## La mesure

Sonde identique sur les deux versions : on oriente `CLAUDE_CODE_EXECUTABLE` vers un faux
`claude` qui n'ecrit que son `argv`, on fait la vraie negociation en annoncant
`fs.writeTextFile`, et on lit la ligne de commande que l'adaptateur *aurait* donnee au
vrai binaire. Aucune authentification, aucun jeton consomme.

**`@zed-industries/claude-code-acp` 0.16.2** — la version en place :

```
--allowedTools mcp__acp__Read
--disallowedTools AskUserQuestion,Read,Write,Edit
--mcp-config {"mcpServers":{"acp":{"type":"sdk","name":"acp"}}}
--tools default
```

**`@agentclientprotocol/claude-agent-acp` 0.66.0** — le successeur :

```
--disallowedTools AskUserQuestion
--tools default
```

Le successeur ne retire **ni `Write` ni `Edit`**, et n'installe plus de serveur MCP de
remplacement. L'inspection du code confirme la mesure : la branche
`if (clientCapabilities?.fs?.writeTextFile) { disallowedTools.push("Write", "Edit"); }` a
disparu, les dependances `@modelcontextprotocol/sdk` et `diff` ne sont plus la, et les
methodes `readTextFile` / `writeTextFile` de l'adaptateur n'ont plus **aucun appelant
interne** — elles ne subsistent que comme surface d'API cliente. `Write` et `Edit` y sont
desormais traites par des hooks `PostToolUse`, c'est-a-dire **apres** l'ecriture, pour
afficher un diff.

## Decision

**On ne migre pas.** `@zed-industries/claude-code-acp` reste la cible, epinglee a la
version **0.16.2**, malgre sa depreciation.

Sur le successeur, l'agent ecrirait directement sur le disque et le registre ne pourrait
plus qu'observer apres coup. Ce n'est pas une regression cosmetique : c'est la suppression
du seul mecanisme du produit qui n'existe nulle part ailleurs. Migrer aurait supprime la
raison d'exister de Trame **en silence**, sans qu'aucun test existant ne bronche.

Deux garde-fous en consequence :

- **Un canari automatique**, `crates/trame-agent/tests/interception_canary.rs`, joue la
  sonde ci-dessus a chaque `just ci` et dans un job de CI dedie. Il echoue bruyamment si
  `Write` ou `Edit` restent disponibles, et un second test verifie qu'il **sait detecter**
  la rupture, en rejouant son analyse sur la capture reelle du successeur. Un canari qui
  ne peut pas echouer ne garde rien.
- `TRAME_ACP_COMMAND` permet de viser une version candidate **avant** de s'y engager.
  Toute mise a jour de l'adaptateur passe par la.

## Consequences

- **On depend d'un paquet deprecie pour le mecanisme central du produit.** C'est le vrai
  cout de cette decision, et il faut le regarder en face plutot que le repartir dans des
  notes de bas de page. Un paquet deprecie ne recoit plus de correctifs, y compris de
  securite, et le protocole ACP continuera d'evoluer sans lui.
- L'invariant d'interception a une dependance **hors de notre controle et non
  contractuelle**. Le canari ne la supprime pas : il garantit seulement qu'on
  l'apprendra par un echec de test plutot que par un journal qui mentait depuis trois
  semaines.
- La fenetre se refermera. Cette decision est un sursis, pas une solution.

### Les sorties possibles, et laquelle est a l'utilisateur

Aucune n'est engagee ici : cet ADR constate et epingle. **Le choix de la strategie est une
decision produit**, pas technique.

1. **Contribuer en amont.** Proposer au successeur une option qui retire les outils
   d'ecriture natifs quand le client annonce `fs.writeTextFile`. C'est la sortie propre,
   elle beneficie a tous les clients ACP, et elle ne depend pas de nous seuls.
2. **Un adaptateur maintenu par Trame.** Un fork minimal, ou un adaptateur ecrit
   directement sur `@anthropic-ai/claude-agent-sdk`, qui expose ce qu'il nous faut. Cout
   reel, controle reel.
3. **Passer par les hooks `PreToolUse` du SDK.** **Sondee** le 2026-08-12 :
   [`docs/sondes/2026-08-12-pretooluse.md`](../sondes/2026-08-12-pretooluse.md). Le hook voit
   tous les outils avant execution, peut **refuser** (`permissionDecision: "deny"`, deja
   utilise par l'adaptateur lui-meme) et expose `tool_name` et `tool_input`.
   La difficulte n'est aucune de ces trois questions : c'est **par ou enregistrer le hook**.
   Un hook callback ne traverse pas JSON-RPC — JSON ne porte pas de fonction — donc il faut
   passer par un hook de type **commande** dans un fichier de reglages, ce qui pose la
   question d'ecrire dans le projet qu'on surveille. La piste tient, elle n'est pas gratuite,
   et rien n'est engage.
4. **Accepter la degradation.** Detection a posteriori par FSEvents, et Trame se reduit a
   un journal. Le journal a de la valeur seul, mais ce n'est plus le meme produit.

## Alternatives ecartees

- **Migrer quand meme et compenser par un watcher.** C'est exactement le contournement
  interdit : FSEvents arrive apres l'ecriture, donc trop tard pour informer l'agent. On
  garderait le nom du produit en perdant son mecanisme.
- **Migrer et annoncer la capacite `terminal` pour reprendre du controle.** Sans rapport :
  `terminal` concerne les outils shell, pas `Write` et `Edit`.
- **Rester sur 0.16.2 sans canari.** C'est l'etat de fait avant cet ADR, et il n'est
  tenable qu'aussi longtemps que personne ne met a jour sans regarder. Le canari est ce
  qui transforme une chance en garantie.

## Revision du 2026-08-13 — la sortie esperee n'existe pas

Le plan des hooks `PreToolUse` comptait cette dette parmi ses benefices : si un `deny` sur
`Write` et `Edit` faisait basculer l'agent sur les outils ACP, l'epinglage tombait.
**[Sonde 5](../sondes/2026-08-13-write-edit-par-hook.md) : non.**

Sur 0.66.0, l'`argv` passe a la CLI ne contient **ni `--mcp-config` ni `--allowedTools`** :
aucun serveur MCP `acp`, donc aucun outil `mcp__acp__Write` vers lequel se rabattre. Refuser
les outils natifs ne redirige pas l'agent, **ca le prive de tout chemin d'ecriture**.

L'hypothese etait fausse pour une raison qu'aucun raisonnement sur les hooks ne pouvait
reveler : elle portait sur la presence d'outils, pas sur le comportement des hooks. Mesuree en
une capture d'argv, sans un jeton.

**La piste qui reste**, et son premier maillon est mesure : les deux versions transmettent a la
CLI un serveur MCP declare par le client dans `session/new`. Trame pourrait donc **porter son
propre outil d'ecriture** et refuser les natifs par hook — une independance plus forte que
l'epinglage, parce qu'elle reposerait sur une capacite documentee du protocole et non sur un
detail non specifie d'un paquet tiers. Non verifie au-dela de la plomberie : rien ne dit encore
que l'agent choisira cet outil, ni que lancer un sous-processus MCP par session soit un bon
echange.

**En attendant, cette decision reste inchangee, et la dette reste ouverte.**

## Ce qui invaliderait cette decision

Que le successeur retire a nouveau `Write` et `Edit` — verifiable en une commande :
`TRAME_ACP_COMMAND=…/claude-agent-acp just canary`. Ce jour-la, la migration devient un
changement de constante.

Ou qu'une des sorties ci-dessus soit engagee, ce qui rendrait cet ADR remplace plutot que
faux.
