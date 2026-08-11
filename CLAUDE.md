# Trame — instructions Claude Code

Le cadrage du projet — these, decisions, invariants, structure, commandes, regle de
controle de version — vit dans **[`AGENTS.md`](AGENTS.md)**, importe ici :

@AGENTS.md

**Lis `AGENTS.md` en entier avant d'ecrire une ligne de code.** Si l'import ci-dessus
n'a pas ete resolu dans ton contexte, ouvre le fichier a la main.

Pourquoi ce decoupage : Trame orchestre Claude Code, Codex et Gemini CLI. Le jour ou
une session Trame lance Codex sur le depot Trame lui-meme, elle ne verra que
`AGENTS.md`. Le cadrage y est donc neutre, et ce fichier-ci ne contient que ce qui est
specifique a Claude Code.

> **Ne recopie rien de `AGENTS.md` ici.** Une information a un seul domicile ; deux
> copies divergent a deux vitesses differentes. Si `but agent setup` reinjecte son bloc
> `<!-- gitbutler-agent-setup -->` dans ce fichier, supprime-le : son domicile est
> `AGENTS.md`.

## Skills du projet

Prescriptives et courtes, chacune avec un exemple correct et un contre-exemple.
**A lire avant d'agir dans leur domaine**, pas apres.

| Skill | Quand |
|---|---|
| `rust-conventions` | Avant d'ecrire ou de modifier du Rust ici |
| `actor-pattern` | Avant de creer un acteur, ou des qu'on est tente d'ecrire `Arc<Mutex<_>>` |
| `acp-integration` | Avant de toucher a `trame-agent` ou de brancher un harness |
| `journal-schema` | Avant de toucher au schema SQLite ou d'ecrire une requete |
| `concurrency-testing` | Avant d'ecrire un test qui touche a un acteur ou au temps |
| `adr-format` | Avant de creer un ADR, ou quand on hesite a documenter un choix |
| `gitbutler` | Fournie par `but agent setup`. Fait autorite sur la syntaxe `but`. |

## Subagents

| Subagent | Perimetre |
|---|---|
| `architect` | Confronte toute decision au concept et aux invariants. Redige les ADR. **Autorite pour dire « ca viole un invariant »**, et ca arrete le travail. |
| `rust-reviewer` | Revue d'idiomatisme : erreurs, durees de vie, allocations, chemins de panique |
| `test-writer` | Les tests, en particulier les tests de concurrence deterministes |
| `acp-specialist` | Le protocole ACP et l'integration des harnesses |
| `doc-keeper` | Empeche `AGENTS.md`, les ADR et les skills de mentir |

### Discipline d'execution

Sur ce projet greenfield, les subagents se lancent **en sequence, pas en parallele** :
les crates n'ont pas encore de frontieres stables et des agents paralleles se
marcheraient dessus. L'ironie n'echappera a personne — c'est exactement le probleme
que Trame resout, et Trame n'existe pas encore.

La parallelisation viendra quand les crates seront reellement independantes.

## Rappels qui coutent cher a oublier

- **`but`, jamais `git`** pour toute mutation. Detail dans `AGENTS.md`.
- **Une phase a la fois**, arret a chaque point de controle. Ne pas enchainer sur la
  phase suivante sans validation humaine explicite.
- Si une decision du tableau de `AGENTS.md` te semble mauvaise : **dis-le et
  argumente**, mais ne devie pas seul.
