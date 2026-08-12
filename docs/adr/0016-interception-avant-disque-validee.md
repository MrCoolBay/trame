# 0016 — L'interception avant ecriture est possible, et voici ses trous

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

C'est **la** question dont depend l'existence du produit. Si les ecritures d'un agent ne
sont pas interceptables avant que le disque soit touche, le registre ne peut qu'observer
apres coup : quand FSEvents remonte l'evenement, le fichier est ecrit, le tool call est
termine, l'agent est passe a la suite. L'avis de lecture perimee arriverait trop tard, et
Trame se reduirait a un journal.

La question devait donc etre tranchee **avant** d'ecrire le transport, empiriquement, et
pas par lecture optimiste de la documentation.

## Ce qui a ete verifie, et comment

### 1. Le protocole — le client heberge le systeme de fichiers

Le schema JSON d'ACP 0.4.5 declare `fs/read_text_file` et `fs/write_text_file` comme des
methodes du **client**, annoncees par
`ClientCapabilities.fs.{readTextFile, writeTextFile}`.

L'inversion est la : **Trame est le client, l'agent est le serveur**. Ce n'est pas l'agent
qui ecrit puis nous previent — c'est l'agent qui *demande* a Trame d'ecrire. Le point
d'interception n'est pas un hook a installer, c'est le chemin normal du protocole.

### 2. L'adaptateur — il desactive les outils natifs

Verifie dans le code de `@zed-industries/claude-code-acp` 0.16.2,
`dist/acp-agent.js:844` :

```js
if (this.clientCapabilities?.fs?.writeTextFile) {
    disallowedTools.push("Write", "Edit");
}
```

C'est plus fort qu'espere. En annoncant la capacite, on n'obtient pas seulement une
notification : **les outils `Write` et `Edit` natifs de l'agent sont retires**. Il ne peut
plus ecrire lui-meme ; il n'a plus que le chemin qui passe par nous. `Edit` transite lui
aussi par `writeTextFile` apres relecture (`mcp-server.js:355`).

### 3. La poignee de main — verifiee en vrai

`initialize` lance contre l'adaptateur reel, cout nul, aucune requete au modele :

```
-> {"method":"initialize","params":{"protocolVersion":1,
    "clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true}}}}
<- {"result":{"protocolVersion":1,"agentInfo":{"name":"@zed-industries/claude-code-acp",
    "version":"0.16.2"},"authMethods":[...]}}
```

### 4. Le run live — ★ **confirme**

Execute le 2026-08-12 par l'utilisateur, hors session Claude Code, via
`cargo run -p trame-agent --example deux_sessions`. **Deux sessions Claude Code reelles**
dans un repertoire de travail partage, sans isolation.

```
[session-A] capacites : interception=true injection=true permission=true
[session-A] session ACP ouverte : 16d4a6f6-8628-4fe6-90c4-88cc49afcd02
[session-B] session ACP ouverte : 2ae3d69f-e980-4396-98a1-95315156803b
[session-A] LECTURE  …/trame-verif-phase2/auth.rs
[session-B] LECTURE  …/trame-verif-phase2/auth.rs
[session-A] ★ ECRITURE INTERCEPTEE  …/session_a.txt  (8 octets)
[session-A]   refus volontaire, rien n'est ecrit sur le disque
[session-B] ★ ECRITURE INTERCEPTEE  …/session_b.txt  (8 octets)
[session-B]   refus volontaire, rien n'est ecrit sur le disque
```

Etat du disque apres le run : `session_a.txt` **absent**, `session_b.txt` **absent**.

Les deux agents ont eux-memes rapporte l'echec — « l'ecriture du fichier a ete refusee
par un hook de verification » — puis ont resume leur travail sans planter. Le chemin du
refus fonctionne donc de bout en bout : l'agent recoit un outil en echec, ce qu'il sait
deja traiter.

**Conclusion : deux agents ont demande a ecrire, nous avons refuse, rien n'a atteint le
disque.** La piece porteuse de l'edifice tient.

## Decision

**L'interception avant ecriture est retenue comme acquise pour ACP + Claude Code.** Le
transport est construit dessus.

Consequences appliquees dans `trame-agent` :

- `initialize` annonce **toujours** `fs.readTextFile` et `fs.writeTextFile`. Ce n'est pas
  une option de configuration : sans cette annonce, l'agent garde ses outils natifs et
  l'admission n'existe plus.
- `session/new` passe `_meta.claudeCode.options.disallowedTools`, qui est **fusionne** et
  non ecrase par l'adaptateur (`acp-agent.js:860`). C'est ce qui permet de fermer les
  trous restants.
- Une demande d'ecriture remonte comme `AgentEvent::FileWrite(request)` portant son canal
  de reponse. **Rien ne part vers l'agent avant la decision.** Une requete abandonnee
  **refuse** au lieu d'admettre : si le defaut etait « admis », une requete oubliee
  produirait une ecriture non admise — exactement ce que le produit existe pour empecher.

## Les trous, nommes

Un filet dont on ne connait pas les trous est pire qu'un filet dont on les connait.

| Trou | Etat | Consequence |
|---|---|---|
| `NotebookEdit` | **Ferme** par `disallowedTools` | L'adaptateur ne le retire pas de lui-meme : il ecrirait un `.ipynb` directement. |
| **La lecture par un autre outil** | **Ouvert**, et c'est le plus grave | Retirer `Read` ne force pas l'agent a passer par nous : `Grep`, `Glob` et `Bash` restent disponibles. Un agent qui lit par l'un d'eux **n'entre pas dans le read-set**, et sans read-set il n'y a *jamais* de `StaleRead`. Ce trou ne degrade pas le produit, il l'eteint — silencieusement. Constate en essayant de mesurer : les agents avaient travaille, aucune lecture n'etait remontee. |
| `Bash` et les ecritures par shell | **Ouvert**, mesure | Capture reelle : `--disallowedTools AskUserQuestion,Read,Write,Edit`. `Bash`, `BashOutput` et `KillShell` **restent disponibles** — ils ne sont retires que si le client annonce la capacite `terminal`, que Trame n'annonce pas. Un `echo > fichier` echappe donc a l'admission. |
| Ecritures hors-bande (`sed -i`, hooks git, formatters, build) | **Ouvert**, par nature | Aucun protocole ne les couvre. Rattrapees par FSEvents, jamais admises. Deja liste comme risque assume dans le concept. |
| `internalPath` de l'adaptateur | **Sans effet** | Une branche ecrit en direct sous `~/.claude/`, hors du repertoire de travail du projet. Ne concerne aucun fichier suivi. |
| Mode PTY | **Ouvert par construction** | `can_intercept_writes == false`. L'interface **doit** afficher la degradation. |
| Reponse a une demande de permission | **Ferme** — on ne choisit que du non persistant | **Trouve par le run live**, et non par relecture : choisir `allow_always` a fait ecrire `.claude/settings.local.json` **dans le repertoire de travail**, hors admission. En repondant a une permission, on peut se salir soi-meme l'arbre qu'on surveille. `PermissionRequest::allow_once` est desormais le seul chemin. |

### La portee reelle de l'invariant, enoncee precisement

Mesuree, pas supposee. La ligne de commande reellement produite en annoncant
`fs.writeTextFile` et en passant notre `_meta` :

```
--allowedTools    mcp__acp__Read
--disallowedTools NotebookEdit,AskUserQuestion,Read,Write,Edit
```

Donc :

> **Le registre est le point de passage unique des ecritures d'agent faites par les outils
> de fichiers — `Write`, `Edit`, `NotebookEdit`.** Les ecritures faites *par le shell* de
> l'agent (`Bash`) ne sont ni interceptees, ni admises, ni journalisees a l'admission.
>
> **Et le read-set ne contient que les lectures faites par l'outil de lecture ACP.** Une
> lecture par `Grep`, `Glob` ou `Bash` echappe entierement au registre.

La seconde phrase est plus lourde de consequences que la premiere. Une ecriture qui echappe
laisse une ligne manquante dans le journal. Une **lecture** qui echappe supprime la
condition d'un `StaleRead` : le mecanisme central ne se declenche pas, et rien ne l'indique.
Le produit a l'air de fonctionner et ne fait rien.

C'est pour ca que le harnais experimental ferme `Grep`, `Glob` et `Bash` : sans ce controle,
il mesurerait du vide. Un canari verifie que ces fermetures atteignent bien la ligne de
commande de l'agent, fusionnees et non ecrasees.

C'est cette phrase-la qui doit etre affichee a l'utilisateur, pas une garantie plus large.
Un agent a qui on demande d'ecrire un fichier utilise `Write` — c'est le chemin normal, et
c'est celui qu'on couvre. Un agent qui lance `sed -i` dans un `Bash` passe a cote, comme y
passent le build, les hooks git et les formatters.

Deux facons de fermer ce trou, aucune gratuite, et le choix est produit :

1. **Ajouter `Bash` a `disallowedTools`.** Ferme le trou completement, et prive l'agent de
   shell : plus de tests, plus de build, plus de git. Inacceptable pour la plupart des
   usages.
2. **Annoncer la capacite `terminal`.** Les outils shell sont alors remplaces par les
   methodes `terminal/*` de l'ACP, donc on **voit les commandes**. On ne voit toujours pas
   leurs effets sur les fichiers — une commande n'est pas une ecriture structuree — mais on
   passe de l'aveuglement a l'observation, et un `sed -i` deviendrait detectable par
   analyse de la commande. C'est la piste la plus prometteuse, et elle est hors v0.1.

## Alternatives ecartees

- **Detection a posteriori par FSEvents seule.** Arrive apres l'ecriture, donc trop tard
  pour informer l'agent. Reste utile comme filet pour le hors-bande.
- **Annoncer la capacite `terminal` pour fermer le trou `Bash`.** Fermerait un trou et en
  ouvrirait un autre : les ecritures d'un shell ne sont pas des evenements structures, on
  verrait la commande sans voir ses effets. A reconsiderer quand le terminal aura une
  raison d'etre integree.
- **Un hook systeme sur les appels d'ecriture** (FUSE, interposition). Fragile, demande
  des privileges, casse a chaque mise a jour de macOS, et arrive trop tard pour dialoguer.
- **`_meta.disableBuiltInTools`.** Fermerait tout d'un coup, y compris `Glob`, `Grep`,
  `Task`, `WebSearch`. On supprimerait les trous en supprimant l'agent.

## Ce qui invaliderait cette decision

Que l'adaptateur cesse de retirer `Write` et `Edit` quand la capacite est annoncee. C'est
verifiable en une ligne, et la verification tient dans un test : `initialize` doit
annoncer `fs.writeTextFile`, et une requete `fs/write_text_file` doit remonter avant toute
ecriture. Les deux sont couverts par
`crates/trame-agent/tests/interception.rs`, contre un agent scenarise.

Le run live etant passe, il n'y a plus de second declencheur en attente. Si un jour un
harness ecrivait malgre tout, ce serait un probleme de these produit et pas de transport a
corriger : il faudrait s'arreter et le dire, pas contourner par un watcher.

## Ce que le run a appris en plus

Deux constats qu'aucune relecture de code n'aurait donnes, et qui ont chacun produit du
code :

1. **Les chemins arrivent absolus et resolus.** La racine passee etait
   `/var/folders/…/projet`, l'agent a repondu `/private/var/folders/…/projet/auth.rs`.
   Sans normalisation, une lecture et une ecriture du meme fichier deviennent deux cles
   differentes, et `StaleRead` **cesse de se declencher sans que rien ne casse** — le
   registre se tait exactement quand il devrait parler, et tous les tests passent parce
   qu'ils utilisent des chemins relatifs. D'ou `trame_core::ProjectRoot`, par lequel toute
   cle de fichier doit passer.
2. **Repondre a une permission peut ecrire dans le projet.** Voir le tableau des trous.
