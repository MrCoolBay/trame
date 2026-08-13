---
name: actor-pattern
description: Comment ecrire un acteur tokio dans Trame — mpsc plus oneshot, propriete de l'etat, poignee clonable, arret propre, backpressure. A lire avant de creer un acteur, d'ajouter un message a un acteur existant, ou des qu'on est tente d'ecrire Arc<Mutex<...>>.
---

# Le pattern acteur — Trame

## L'invariant

> **Un acteur possede son etat. Jamais de `Arc<Mutex<_>>` pour de l'etat metier.**

Ce n'est pas une preference de style. Le registre doit repondre a « ce fichier a-t-il
change **depuis** que cette session l'a lu », ce qui suppose un **ordre total** sur
les lectures et les ecritures d'un projet. Un `Mutex` donne l'exclusion mutuelle, pas
l'ordre total. Un acteur qui traite ses messages un par un donne l'ordre total
gratuitement, et il n'y a aucun interleaving a raisonner.

Un `Arc` sur une valeur **immuable** — une horloge, une configuration — n'est pas
concerne. Il n'y a pas de mutation, donc pas d'ordre a garantir.

## Les cinq pieces

1. Un enum `...Msg`, une variante par operation, chacune portant son `oneshot::Sender`
   de reponse.
2. Une struct d'etat **privee**, jamais exposee, qui possede tout.
3. Une boucle `while let Some(msg) = rx.recv().await`.
4. Une struct `...Handle` publique et clonable, qui encapsule le `mpsc::Sender` et
   expose des methodes `async` typees. **L'appelant ne construit jamais un message a
   la main.**
5. Un `spawn` qui renvoie le handle et le `JoinHandle`.

## Exemple complet

Les reservations de ressources : le port 3000 est machine-wide, donc cet acteur est
**global** au workspace, contrairement au registre qui est par projet (ADR 0010).

```rust
use std::collections::HashMap;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Une reservation refusee dit **par qui** elle est detenue, sinon l'utilisateur
/// n'a aucun moyen d'agir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Granted,
    Held { by: String },
}

/// Un message adresse a l'acteur. Chaque variante porte son canal de retour.
enum ClaimMsg {
    Claim { resource: String, owner: String, reply: oneshot::Sender<ClaimOutcome> },
    Release { resource: String, owner: String, reply: oneshot::Sender<bool> },
    Snapshot { reply: oneshot::Sender<Vec<(String, String)>> },
}

/// L'etat. Prive, possede par l'acteur, jamais partage.
struct ClaimActor {
    held: HashMap<String, String>,
    rx: mpsc::Receiver<ClaimMsg>,
}

impl ClaimActor {
    /// La boucle. Un message a la fois : l'ordre total est structurel.
    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                ClaimMsg::Claim { resource, owner, reply } => {
                    let outcome = match self.held.get(&resource) {
                        Some(existing) if *existing != owner => {
                            ClaimOutcome::Held { by: existing.clone() }
                        }
                        _ => {
                            self.held.insert(resource.clone(), owner.clone());
                            ClaimOutcome::Granted
                        }
                    };
                    tracing::debug!(%resource, %owner, ?outcome, "reservation");
                    // L'appelant a pu abandonner : son `oneshot` est ferme. Ce n'est
                    // pas une erreur, on l'ignore. `let _ =` et pas `.unwrap()`.
                    let _ = reply.send(outcome);
                }
                ClaimMsg::Release { resource, owner, reply } => {
                    let released = self.held.get(&resource) == Some(&owner);
                    if released {
                        self.held.remove(&resource);
                    }
                    let _ = reply.send(released);
                }
                ClaimMsg::Snapshot { reply } => {
                    let snapshot =
                        self.held.iter().map(|(r, o)| (r.clone(), o.clone())).collect();
                    let _ = reply.send(snapshot);
                }
            }
        }
        // Sortie de boucle = tous les handles sont tombes. Arret propre, sans
        // signal dedie, sans `select!`, sans `CancellationToken`.
        tracing::info!("acteur de reservations arrete");
    }
}

/// La poignee. Clonable, c'est le seul acces a l'acteur.
#[derive(Debug, Clone)]
pub struct ClaimHandle {
    tx: mpsc::Sender<ClaimMsg>,
}

impl ClaimHandle {
    /// Erreur unique : l'acteur est mort. Rien d'autre ne peut echouer.
    pub async fn claim(&self, resource: &str, owner: &str) -> Result<ClaimOutcome, ActorGone> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ClaimMsg::Claim {
                resource: resource.to_owned(),
                owner: owner.to_owned(),
                reply,
            })
            .await
            .map_err(|_| ActorGone)?;
        rx.await.map_err(|_| ActorGone)
    }

    pub async fn release(&self, resource: &str, owner: &str) -> Result<bool, ActorGone> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ClaimMsg::Release {
                resource: resource.to_owned(),
                owner: owner.to_owned(),
                reply,
            })
            .await
            .map_err(|_| ActorGone)?;
        rx.await.map_err(|_| ActorGone)
    }

    pub async fn snapshot(&self) -> Result<Vec<(String, String)>, ActorGone> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(ClaimMsg::Snapshot { reply }).await.map_err(|_| ActorGone)?;
        rx.await.map_err(|_| ActorGone)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("l'acteur de reservations n'est plus joignable")]
pub struct ActorGone;

/// Demarre l'acteur. La borne du canal **est** la politique de backpressure.
pub fn spawn_claims() -> (ClaimHandle, JoinHandle<()>) {
    // Borne, jamais `unbounded_channel()` : une file non bornee transforme une
    // surcharge en fuite de memoire silencieuse. 64 messages en attente sur un
    // acteur qui traite en microsecondes, c'est deja beaucoup.
    let (tx, rx) = mpsc::channel(64);
    let join = tokio::spawn(ClaimActor { held: HashMap::new(), rx }.run());
    (ClaimHandle { tx }, join)
}
```

Test correspondant — **aucun agent, aucun `sleep`, deterministe** :

```rust
#[tokio::test]
async fn an_already_held_resource_names_its_holder() {
    let (claims, _join) = spawn_claims();

    assert_eq!(claims.claim("port:3000", "portailfcd").await.unwrap(), ClaimOutcome::Granted);
    assert_eq!(
        claims.claim("port:3000", "lyra-rp").await.unwrap(),
        ClaimOutcome::Held { by: "portailfcd".into() },
        "un refus doit dire par qui la ressource est tenue"
    );

    assert!(claims.release("port:3000", "portailfcd").await.unwrap());
    assert_eq!(claims.claim("port:3000", "lyra-rp").await.unwrap(), ClaimOutcome::Granted);
}
```

## Contre-exemple

```rust
// ❌ TOUT est faux ici.
#[derive(Clone)]
pub struct Claims {
    held: Arc<Mutex<HashMap<String, String>>>,   // etat metier partage
}

impl Claims {
    pub fn claim(&self, resource: &str, owner: &str) -> bool {
        let mut held = self.held.lock().unwrap();   // unwrap : deny en CI
        if held.contains_key(resource) {
            return false;                          // booleen : on perd « par qui »
        }
        held.insert(resource.to_owned(), owner.to_owned());
        true
    }
}
```

Quatre problemes : etat metier sous `Mutex` (invariant 1 de `AGENTS.md`), `unwrap()`
sur un lock empoisonne, un booleen la ou il faut un resultat exploitable, et un
`std::sync::Mutex` tenu a travers un point d'await des que la fonction deviendra
`async`.

## Regles de detail

- **`let _ = reply.send(...)`, jamais `.unwrap()`.** Un `oneshot` ferme signifie que
  l'appelant a abandonne. C'est normal, pas une erreur.
- **Un `mpsc::channel` borne.** Jamais `unbounded_channel()`.
- **Pas de signal d'arret dedie.** L'acteur s'arrete quand le dernier handle tombe.
  Un `CancellationToken` en plus est une deuxieme facon de mourir, donc un bug de plus.
- **Le handle expose des methodes, pas l'enum.** L'enum `...Msg` reste prive au module.
- **Un acteur n'appelle jamais un autre acteur en attendant sa reponse dans sa propre
  boucle** — c'est un interblocage en attente d'arriver. S'il faut le faire, on spawn
  la sous-tache et on lui passe le `oneshot::Sender` du demandeur.
- **Six champs maximum par variante de message.** Au-dela, l'acteur est mal decoupe ;
  `clippy.toml` applique le seuil.
- **Une operation par variante.** Pas de `Msg::Do { op: Operation }` — ca deplace le
  dispatch dans un deuxieme `match` et supprime le typage des reponses.
