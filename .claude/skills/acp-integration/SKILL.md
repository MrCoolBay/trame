---
name: acp-integration
description: Agent Client Protocol dans Trame — JSON-RPC sur stdio, negociation de capacites, cycle de vie d'une session, interception des ecritures avant le disque, trous connus du protocole et strategie de repli PTY. A lire avant de toucher a trame-agent ou de brancher un harness.
---

# Integration ACP — Trame

## Pourquoi ACP et pas un PTY

Une seule raison, et elle porte tout le produit :

> **En ACP, les acces au systeme de fichiers passent par le protocole. Donc Trame
> les voit avant le disque, et peut les soumettre au registre.**

C'est l'inversion qui compte : dans ACP, **Trame est le client et l'agent est le
serveur**. Ce n'est pas l'agent qui ecrit puis qui nous previent — c'est l'agent qui
*demande* a Trame d'ecrire. Le point d'interception n'est donc pas un hook a
installer, c'est le chemin normal du protocole.

En PTY, on voit du texte. Les ecritures se decouvrent apres coup par FSEvents, quand
le tool call est termine et que l'agent est passe a la suite. Il reste possible de
journaliser et d'attribuer ; il devient impossible d'informer au bon moment.

## Cycle de vie

```
1. spawn du harness en sous-process, stdin/stdout en pipes
2. initialize            -> negociation de version et de capacites
3. session/new           -> un id de session ACP, associe a notre SessionId
4. session/prompt        -> on envoie le travail
5. session/update (n)    -> notifications entrantes : texte, tool calls, resultats
   dont les requetes fs/*  -> ★ LE POINT D'ADMISSION
6. fin de tour           -> l'agent rend la main
```

Les details de transport — JSON-RPC 2.0, une enveloppe par ligne sur stdio — sont
standard. **Les noms exacts des methodes et la forme des payloads sont a verifier
contre la specification** (`agentclientprotocol.com`) au moment de coder, pas a
recopier depuis cette skill : le protocole bouge.

## Le point d'admission

Une requete d'ecriture de l'agent arrive comme un appel entrant a traiter, pas comme
un evenement a observer. La reponse est differee jusqu'au verdict du registre.

✅ **Correct** — le registre decide, l'evenement normalise sort ensuite :

```rust
async fn on_write_request(&mut self, path: PathBuf, content: String) -> Result<()> {
    // 1. Admission AVANT le disque. C'est tout l'interet d'ACP.
    let verdict = self.registry.admit(self.session, &path, &content).await?;

    // 2. L'avis eventuel est prepare pour le prochain tour de l'agent.
    if verdict.needs_notice() {
        self.pending_notice = Some(verdict.clone());
    }

    // 3. Puis seulement l'ecriture.
    if verdict.is_admitted() {
        tokio::fs::write(&path, &content).await?;
    }

    self.emit(AgentEvent::FileWrite { path, content });
    Ok(())
}
```

❌ **Contre-exemple** — l'ordre est inverse, le registre est un spectateur :

```rust
async fn on_write_request(&mut self, path: PathBuf, content: String) -> Result<()> {
    tokio::fs::write(&path, &content).await?;              // deja sur le disque
    let _ = self.registry.admit(self.session, &path, &content).await;  // trop tard
    Ok(())
}
```

Le second compile, passe les tests, et supprime la raison d'exister du produit. C'est
le bug le plus important a ne pas ecrire dans ce depot.

## Les lectures comptent autant que les ecritures

Le read-set se remplit depuis les requetes de lecture de l'agent. Sans elles, il n'y
a pas de `StaleRead` possible, donc pas de produit.

**Filtrer** : seules les lectures substantielles — un fichier lu en entier. Pas les
hits de grep, pas les listings de repertoire. Les agents lisent enormement ; sans
filtre le read-set explose et tout devient niveau 1 (ADR 0007).

## Capacites : declarer la degradation, ne jamais la masquer

```rust
pub struct Capabilities {
    pub can_intercept_writes: bool,   // ACP: true, PTY: false
    pub can_inject_context: bool,
    pub can_request_permission: bool,
}
```

Un utilisateur en mode PTY qui croit avoir la garantie d'admission est dans une
situation **pire** que sans outil : il fait confiance a un filet qui n'existe pas.
L'interface doit afficher la degradation.

Corollaire de code : ne jamais deduire une capacite du type de backend au point
d'appel. On interroge `capabilities()`.

## Trous connus du protocole

- **`AskUserQuestion` indisponible en plan mode.** Cas documente et connu ; ce n'est
  pas le seul.
- Le support d'ACP est **inegal selon les harness**. Une capacite annoncee n'est pas
  toujours une capacite fonctionnelle.
- Le repli PTY **n'est pas optionnel** (ADR 0005). Sans lui, chaque trou du protocole
  devient un harness non supporte.

**On ne forke pas ACP.** Les manques se contribuent en amont : un dialecte prive
supprimerait la compatibilite avec les harness, qui est la seule raison d'utiliser un
protocole standard.

## Regles de detail

- **stdout du sous-process appartient au protocole.** Ses logs a lui vont sur stderr,
  les notres aussi. Un `println!` dans le chemin ACP corrompt la trame JSON-RPC —
  c'est aussi pourquoi `print_stdout` est en `deny`.
- Un sous-process qui meurt est un cas normal, pas une panique : `AgentEvent::Error`
  puis `SessionState::Failed`.
- Le `SessionId` de Trame et l'id de session ACP sont **deux choses differentes**. On
  garde la correspondance ; on ne reutilise pas l'un pour l'autre.
- Le mecanisme de permission ACP existe deja et l'agent sait attendre. Le niveau 3
  du registre s'y branchera plutot que d'inventer un canal (v0.4, pas v0.1).
- Timeouts : un agent peut reflechir plusieurs minutes. Ne jamais mettre de timeout
  court sur un tour ; en mettre un sur l'admission, qui doit repondre en
  millisecondes.

## Verification bloquante de la phase 2

Si Claude Code en ACP ne permet pas d'intercepter l'ecriture **avant** le disque :
s'arreter et le dire. C'est la piece porteuse de l'edifice, pas un detail
d'implementation a contourner.
