# Sonde — les hooks `PreToolUse` couvriraient-ils le trou lecture ?

- **Date** : 2026-08-12
- **Statut** : **sonde, aucune décision, aucune implémentation**
- **Question** : les hooks `PreToolUse` permettent-ils de voir, refuser et inspecter les
  appels à `Grep`, `Glob` et `Bash` ?
- **Enjeu** : c'est la seule piste qui couvrirait les **trois** problèmes ouverts d'un coup —
  trou lecture (§6.7 du concept), trou écriture par `Bash`, et dépendance à un comportement
  non spécifié de l'adaptateur 0.16.2 déprécié ([ADR 0017](../adr/0017-adaptateur-acp-epingle.md)).

---

## Méthode, et niveaux de preuve

Même discipline que la sonde de migration : **mécanique, sans authentification, sans jeton
consommé.** Aucune session d'agent n'a été lancée.

Trois niveaux de preuve, distingués parce qu'ils ne valent pas la même chose :

| Niveau | Nature | Ce que ça vaut |
|---|---|---|
| **A — mesuré** | capture de l'`argv` réellement produit par l'adaptateur | fort : c'est le comportement, pas sa description |
| **B — typé** | définitions de types du SDK (`sdk.d.ts`), source de l'adaptateur | fort sur le contrat, muet sur l'exécution |
| **C — inspecté** | chaînes et fragments du bundle `cli.js` (11 Mo, minifié) | indicatif : présence ≠ chemin d'exécution atteint |

Ce qui exige un tour d'agent réel est marqué **non vérifié**, et n'est pas supposé.

Versions : adaptateur `@zed-industries/claude-code-acp` 0.16.2 ·
SDK `@anthropic-ai/claude-agent-sdk` 0.2.44.

---

## Question 1 — `PreToolUse` voit-il `Grep`, `Glob` et `Bash` avant exécution ?

**Réponse : oui pour « avant exécution », très probablement oui pour ces trois outils —
mais je ne l'ai pas observé.**

### Ce qui est établi

**Niveau B.** L'adaptateur enregistre déjà un hook `PreToolUse`, et il le fait **sans
`matcher`** (`dist/acp-agent.js:799`) :

```js
hooks: {
  ...userProvidedOptions?.hooks,
  PreToolUse: [
    ...(userProvidedOptions?.hooks?.PreToolUse || []),
    { hooks: [createPreToolUseHook(settingsManager, this.logger)] },   // ← pas de matcher
  ],
```

Le type du SDK rend `matcher` optionnel :

```ts
export declare interface HookCallbackMatcher {
    matcher?: string;
    hooks: HookCallback[];
    timeout?: number;   // en secondes
}
```

Un `matcher` absent signifie « tous les outils ». Le hook de l'adaptateur reçoit donc **chaque
appel d'outil**, `Grep`, `Glob` et `Bash` compris — c'est structurel, pas une option.

**Pour « avant exécution »** : le nom de l'événement, et le commentaire de l'adaptateur, qui
est explicite sur l'ordre :

> *This runs before the SDK's built-in permission rules, allowing us to enforce our own
> permission settings for ACP-prefixed tools.*

Le hook s'exécute donc **avant la résolution des permissions**, elle-même avant l'exécution.

### Ce qui reste incertain

- **Je n'ai pas observé le hook se déclencher**, pour aucun outil. Tout ce qui précède est du
  contrat et du code, pas de l'exécution.
- Un tour d'agent réel pourrait révéler des outils traités hors du chemin des hooks. La liste
  `HOOK_EVENTS` compte quinze événements, dont `PermissionRequest` et `Setup` : il existe
  plusieurs chemins, et je n'ai pas cartographié lequel prime dans quel cas.

**Ce qui manque pour trancher** : un tour réel avec un hook instrumenté, donc authentification,
jetons, et contournement du garde-fou anti-imbrication.

---

## Question 2 — peut-il refuser, ou seulement observer ?

**Réponse : il peut refuser. C'est la réponse la plus solide des trois.**

### Ce qui est établi

**Niveau B, et par deux voies indépendantes.**

Le type de sortie du SDK autorise explicitement le refus :

```ts
export declare type PreToolUseHookSpecificOutput = {
    hookEventName: 'PreToolUse';
    permissionDecision?: 'allow' | 'deny' | 'ask';
    permissionDecisionReason?: string;
    updatedInput?: Record<string, unknown>;
    additionalContext?: string;
};
```

Et surtout : **l'adaptateur s'en sert déjà en production** (`dist/tools.js:588`). Ce n'est pas
une capacité théorique du type, c'est un chemin de code utilisé :

```js
case "deny":
    return {
        continue: true,
        hookSpecificOutput: {
            hookEventName: "PreToolUse",
            permissionDecision: "deny",
            permissionDecisionReason: `Denied by settings rule: ${permissionCheck.rule}`,
        },
    };
```

### Deux capacités que je ne cherchais pas, et qui changent la valeur de la piste

Le type de sortie porte aussi :

- **`updatedInput`** — le hook peut **réécrire les paramètres** de l'appel avant exécution. On
  pourrait donc, par exemple, restreindre un `Grep` plutôt que le refuser.
- **`additionalContext`** — le hook peut **injecter du contexte** au moment de l'appel d'outil.

Ce second point mérite d'être noté : aujourd'hui l'avis de lecture périmée est retenu et posé
devant le *message suivant*, parce qu'au moment du verdict l'agent est au milieu d'un tool call
et qu'il n'y a pas de canal pour lui parler. `additionalContext` serait précisément ce canal.

Je ne propose rien — c'est une observation, et elle demanderait sa propre mesure. La manche
[ADR 0018](../adr/0018-pas-de-diff-dans-stalefile.md) a montré que l'avis fonctionne à
l'emplacement actuel ; le déplacer serait un changement à mesurer, pas une amélioration
évidente.

### Ce qui reste incertain

- Le refus est prouvé pour un hook **de type callback**, celui que l'adaptateur enregistre. Si
  Trame passe par un hook **de type commande** (voir question 4), il reste à vérifier que le
  même `permissionDecision: "deny"` est honoré depuis la sortie standard d'un process. Le
  protocole JSON paraît partagé, je ne l'ai pas prouvé.
- Comportement en cas de refus **concurrent** — deux hooks, décisions contradictoires — non
  exploré.

---

## Question 3 — expose-t-il les paramètres de l'appel ?

**Réponse : oui, l'objet d'entrée complet. Les clés exactes par outil ne sont pas vérifiées.**

### Ce qui est établi

**Niveau B.** Le type d'entrée :

```ts
export declare type PreToolUseHookInput = BaseHookInput & {
    hook_event_name: 'PreToolUse';
    tool_name: string;
    tool_input: unknown;
    tool_use_id: string;
};
```

Et l'adaptateur les consomme tous les deux (`dist/tools.js:592`) :

```js
const toolName  = input.tool_name;
const toolInput = input.tool_input;
const permissionCheck = settingsManager.checkPermission(toolName, toolInput);
```

Donc **le nom de l'outil et son objet de paramètres sont disponibles**, et l'adaptateur prend
déjà des décisions de permission à partir des deux.

### Ce qui reste incertain

`tool_input` est typé `unknown`. Son contenu est le schéma propre de chaque outil, et **je ne
l'ai pas observé** :

- pour `Grep`, on s'attend à `pattern` et `path` — non vérifié ;
- pour `Bash`, on s'attend à `command` — non vérifié ;
- pour `Glob`, on s'attend à `pattern` — non vérifié.

Ces noms sont des attentes raisonnables, pas des faits établis. Le seul moyen de les confirmer
est d'observer un `tool_input` réel.

**Conséquence pratique à ne pas sous-estimer** : pour `Bash`, obtenir la commande n'est que le
début. Décider si `sed -i s/a/b/ src/*.rs` écrit — et quoi — demande d'analyser une ligne de
shell. C'est un problème ouvert, et il ne se règle pas en lisant un champ.

---

## Question 4 — la question qu'il fallait poser : Trame peut-il enregistrer un tel hook ?

Les trois réponses ci-dessus décrivent un mécanisme qui existe. Elles ne disent pas que **nous**
pouvons y accéder. C'est le vrai point de blocage, et il n'était pas dans la liste.

### Le chemin évident ne fonctionne pas

L'adaptateur fusionne les hooks fournis par l'appelant :
`...(userProvidedOptions?.hooks?.PreToolUse || [])`, où `userProvidedOptions` vient de
`_meta.claudeCode.options` — donc **du JSON transporté par JSON-RPC**.

Or `HookCallbackMatcher.hooks` est typé `HookCallback[]` : des **fonctions JavaScript**.

> **Un hook de type callback ne peut pas traverser JSON-RPC.** JSON ne transporte pas de
> fonction. Ce chemin est fermé, et aucune contorsion de `_meta` ne l'ouvre.

C'est établi au niveau B, et c'est net.

### Un autre chemin existe : les hooks de type commande

**Niveau C.** Le bundle `cli.js` connaît quatre formes de hooks :

```js
if (q.type === "command")  return { type: "command", command: q.command };
else if (q.type === "prompt")   … 
else if (q.type === "function") …
else if (q.type === "callback") …
```

Et il attribue une **source** à chaque hook, dont trois qui sont des fichiers de réglages :

```
source:"projectSettings"   source:"localSettings"   source:"userSettings"
source:"pluginHook"        source:"sessionHook"     source:"flagSettings"
```

**Niveau B**, la documentation du SDK dit exactement quels fichiers :

```
* Control which filesystem settings to load.
* - 'user'    - Global user settings (~/.claude/settings.json)
* - 'project' - Project settings (.claude/settings.json)
* - 'local'   - Local settings (.claude/settings.local.json)
* When omitted or empty, no filesystem settings are loaded (SDK isolation mode).
```

**Niveau A**, la capture d'`argv` déjà réalisée montre que l'adaptateur les active tous les
trois :

```
--setting-sources user,project,local
```

Donc : un hook `PreToolUse` de type **commande**, écrit dans un fichier de réglages, **serait
chargé**. C'est le chemin praticable, et il n'est pas celui qu'on aurait deviné.

### Ce que ça coûte, et l'ironie à ne pas manquer

Trois points à peser, aucun n'est bloquant, aucun n'est gratuit :

1. **Où écrire le hook ?** `.claude/settings.json` du projet est **dans le répertoire
   surveillé**. Y écrire est exactement l'écriture hors-bande à l'intérieur du projet qui nous
   a déjà mordus avec `allow_always` ([ADR 0016](../adr/0016-interception-avant-disque-validee.md)).
   Le remède emprunterait le véhicule de la maladie.
   `~/.claude/settings.json` évite le projet mais devient **global à toutes les sessions Claude
   Code de l'utilisateur**, y compris celles qui n'ont rien à voir avec Trame. Il existe une
   piste `flagSettings` / `--settings` qui laisserait passer un fichier hors du projet : **non
   vérifiée**, et c'est celle qu'il faudrait explorer en premier.
2. **Un aller-retour par appel d'outil.** Un hook commande est un process lancé à chaque appel,
   qui doit joindre Trame. Sur un agent qui grep beaucoup, la latence s'additionne. `timeout`
   existe dans le type, mais sa valeur par défaut et le comportement à l'expiration sont **non
   vérifiés**.
3. **Ça ne supprime pas la dépendance, ça la déplace.** On dépendrait des hooks de la CLI
   plutôt que du retrait d'outils par l'adaptateur. C'est plus documenté et plus stable — mais
   ça reste un comportement tiers, et ça mériterait son propre canari.

---

## Synthèse

| Question | Réponse | Niveau | Ce qui manque |
|---|---|---|---|
| Voit `Grep`/`Glob`/`Bash` avant exécution ? | **Oui**, hook sans `matcher` donc tous les outils, et avant la résolution des permissions | B | l'observer se déclencher |
| Peut refuser ? | **Oui**, `permissionDecision: "deny"`, déjà utilisé par l'adaptateur | B ×2 | le refus depuis un hook *commande* |
| Expose les paramètres ? | **Oui**, `tool_name` + `tool_input` | B | les clés réelles par outil |
| **Trame peut-il enregistrer un hook ?** | **Pas par `_meta`** — JSON ne porte pas de fonction. **Oui par un fichier de réglages**, en type commande | B / C | `--settings` hors projet, latence, timeout |

**La piste tient.** Elle couvrirait le trou lecture, le trou `Bash` — au moins pour voir les
commandes — et réduirait la dépendance au retrait d'outils de l'adaptateur déprécié.

Elle n'est pas gratuite, et le point le plus délicat n'est aucune des trois questions posées :
c'est **par où enregistrer le hook sans écrire dans le projet qu'on surveille**.

## Ce que je n'ai pas fait, et pourquoi

**Rien d'implémenté**, comme demandé.

**Aucun tour d'agent réel.** Toutes les incertitudes listées se lèvent de la même façon : un
hook instrumenté, une session réelle, et l'observation de ce qui arrive sur son entrée standard.
Ça demande une authentification, des jetons, et le contournement du garde-fou anti-imbrication —
donc une décision qui n'est pas la mienne.

Le pas suivant, s'il est décidé, tient en une manipulation : écrire un `PreToolUse` de type
commande dans un fichier de réglages, le faire vider son entrée standard dans un fichier, lancer
**un** tour qui grep, et lire. Ça répond d'un coup à ce qui reste ouvert : déclenchement réel,
clés de `tool_input` pour les trois outils, et effet d'un `deny` depuis une commande.
