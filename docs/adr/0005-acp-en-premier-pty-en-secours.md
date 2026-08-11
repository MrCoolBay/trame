# 0005 — ACP en premier, PTY en secours

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Le registre d'admission n'a de sens que s'il voit les ecritures **avant** qu'elles
touchent le disque. Une detection *a posteriori* peut journaliser et alerter, mais
elle ne peut pas informer l'agent au bon moment : quand FSEvents remonte l'evenement,
le fichier est ecrit, le tool call est termine, et l'agent est deja passe a la suite.

Il y a deux facons de piloter un harness de code :

- **ACP** (Agent Client Protocol) — JSON-RPC sur stdio. Les tool calls transitent
  par le protocole, donc ils sont interceptables et le canal de retour vers l'agent
  est structure.
- **PTY** — pilotage de la CLI comme un terminal, via `portable-pty`. On voit du
  texte ; les ecritures se decouvrent apres coup.

## Decision

ACP est le chemin principal. `AcpBackend` d'abord, avec une seule cible en v0.1 :
**Claude Code**. `PtyBackend` existe comme squelette et repli degrade.

Les deux vivent derriere le trait `AgentBackend`, et la difference est declaree
explicitement dans `Capabilities` :

```rust
pub struct Capabilities {
    pub can_intercept_writes: bool,   // ACP: true, PTY: false
    pub can_inject_context: bool,
    pub can_request_permission: bool,
}
```

**L'UI doit afficher la degradation.** Un utilisateur en mode PTY qui croit avoir la
garantie d'admission est dans une situation pire que s'il n'avait pas d'outil du
tout : il fait confiance a un filet qui n'existe pas.

## Consequences

- Le repli PTY **n'est pas optionnel** : ACP est incomplet et inegal selon les
  harness. `AskUserQuestion` est indisponible en plan mode, par exemple. Sans repli,
  chaque trou du protocole devient un harness non supporte.
- En mode PTY, le registre journalise et attribue mais n'admet pas. La valeur du
  journal seul reste reelle — repondre a « qui a ecrit cette ligne, dans quelle
  session, en reponse a quel prompt » est deja utile — mais l'avis de lecture
  perimee, lui, disparait.
- La contribution aux trous d'ACP se fait **en amont**, dans le protocole. On ne
  forke pas ACP : un dialecte prive supprimerait la compatibilite avec les harness,
  qui est la seule raison d'utiliser un protocole standard.
- Point de verification bloquant de la phase 2 : si Claude Code en ACP ne permet
  pas d'intercepter `FileWrite` avant le disque, il faut le dire et s'arreter.
  C'est la piece porteuse de tout l'edifice, pas un detail d'implementation.

## Alternatives ecartees

- **PTY seulement.** Plus simple, marche avec tous les harness, et ne permet pas
  d'informer l'agent au bon moment. Supprime le produit.
- **Un hook sur le systeme de fichiers** (FUSE, interposition de `write`). Fragile,
  demande des privileges, casse a chaque mise a jour de macOS, et arrive de toute
  facon trop tard pour dialoguer avec l'agent.
- **Un wrapper d'outils cote harness** (remplacer l'outil `Write` par un proxy).
  Specifique a chaque harness, casse a chaque mise a jour, et suppose une
  cooperation qu'ACP fournit deja proprement.

## Ce qui invaliderait cette decision

Qu'ACP ne permette pas l'interception avant disque sur aucun harness majeur. Ce
serait un probleme de these produit, pas un choix de transport a revoir : sans
interception, il n'y a pas d'avis a injecter, et Trame se reduit a un journal.
