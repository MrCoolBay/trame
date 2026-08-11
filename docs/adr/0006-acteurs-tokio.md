# 0006 — Acteurs tokio, un par domaine, aucun etat partage

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Le registre d'admission doit rendre des verdicts qui dependent de l'ordre des
evenements : « ce fichier a-t-il change **depuis** que cette session l'a lu ». Une
reponse correcte suppose un ordre total sur les lectures et les ecritures d'un
projet.

Deux facons d'obtenir cet ordre : le prendre par un verrou autour d'un etat partage,
ou l'obtenir par construction en confiant l'etat a une seule tache qui traite ses
messages sequentiellement.

## Decision

Un acteur tokio par domaine, **un registre par projet**. Chaque acteur possede son
etat ; personne d'autre n'y touche. La communication se fait par `mpsc` en entree et
`oneshot` en retour.

> **Invariant : jamais de `Arc<Mutex<_>>` pour de l'etat metier.**

Un `Arc` sur une valeur immuable — une horloge, une configuration — n'est pas
concerne : il n'y a pas de mutation, donc pas d'ordre a garantir.

## Consequences

- **La serialisation et l'ordre total sont gratuits**, par projet, sans verrou. Il
  n'y a rien a raisonner sur les interleavings : l'acteur traite un message a la
  fois.
- Pas de deadlock possible entre registres : ils ne se parlent pas. Deux projets
  sont independants par construction (ADR 0010).
- Un acteur est testable **sans le moindre agent** : on lui envoie une suite de
  messages et on verifie les verdicts. C'est ce qui rend la phase 1 testable avant
  que la phase 2 existe.
- Le `oneshot` de retour donne le backpressure gratuitement : un appelant qui attend
  sa reponse n'inonde pas la file.
- Cout : chaque operation devient un message avec son type. C'est verbeux, et c'est
  ce qui rend la surface d'un domaine explicite plutot que diffuse.
- Un message a plus de six champs est le signe d'un acteur mal decoupe. Le seuil est
  applique par `clippy.toml`.

## Alternatives ecartees

- **`Arc<Mutex<RegistryState>>`.** Marche, jusqu'au jour ou un chemin de code prend
  deux verrous dans un ordre different. Surtout : le verrou ne donne pas l'ordre
  total, il donne l'exclusion mutuelle. Deux lectures concurrentes pourraient
  s'observer dans un ordre incoherent avec leur ordre reel, ce qui est exactement le
  genre de bug qu'on ne reproduit jamais.
- **Un registre global unique.** Contention entre projets qui, par construction, ne
  peuvent pas entrer en collision. Et le compteur de sequence deviendrait global, ce
  que l'ADR 0010 interdit.
- **`async_channel` / `flume` a la place de tokio.** Aucun gain, une dependance de
  plus, et on est deja sur tokio pour le reste.

## Alternative retenue partiellement

Le pattern *handle* : chaque acteur expose une structure `Handle` clonable qui
encapsule le `mpsc::Sender` et offre des methodes `async` typees. L'appelant ne
construit jamais un message a la main. Voir
[`.claude/skills/actor-pattern/SKILL.md`](../../.claude/skills/actor-pattern/SKILL.md)
pour l'exemple complet.

## Ce qui invaliderait cette decision

Un profil de charge ou la sequentialisation d'un acteur devient le goulot. A deux a
cinq sessions par projet, avec un hash blake3 par admission, c'est hors de portee de
plusieurs ordres de grandeur.
