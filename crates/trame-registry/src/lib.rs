//! ★ **Le coeur du produit.** Le controleur d'admission en ecriture.
//!
//! Un acteur tokio **par projet**. Il possede son etat ; personne ne le partage.
//!
//! # Ce n'est pas un systeme de verrous
//!
//! Le locking pessimiste est inadapte : les agents tiennent leur transaction
//! pendant des minutes, ne declarent pas leur intention a l'avance, et bloquer un
//! tool call en vol declenche des timeouts cote harness. Le modele est celui des
//! bases de donnees — **controle de concurrence optimiste avec validation du
//! read-set**.
//!
//! # Pourquoi valider les lectures et pas seulement les ecritures
//!
//! Le mode d'echec le plus frequent a trois agents ne produit **aucune collision
//! d'ecriture** :
//!
//! ```text
//! 1. Session A lit auth.rs, memorise la signature de verify_token()
//! 2. Session B ecrit auth.rs, renomme verify_token() -> validate_token()
//! 3. Session A ecrit handlers.rs, appelle verify_token()
//!
//! -> Deux fichiers differents. Un verrou par fichier ne voit rien.
//! -> L'arbre est casse.
//! ```
//!
//! On ne sait pas *si* ca casse. On sait que **A raisonne sur un monde qui
//! n'existe plus**, et c'est le seul invariant qui compte.
//!
//! # Regles de la v0.1 (phase 1)
//!
//! - **Granularite fichier entier.** Pas de suivi de hunks : fichier + fenetre
//!   temporelle donne 90 % de la valeur pour 5 % du travail.
//! - **Read-set filtre** : seules les lectures substantielles, pas les hits de
//!   grep ni les listings de repertoire. Sinon le read-set explose et tout
//!   devient `StaleRead`.
//! - **Decroissance a 10 minutes.** Au-dela, le contexte de l'agent a tourne de
//!   toute facon.
//! - Compteur de sequence **par projet**.
//! - blake3 a l'admission et a la lecture. Jamais l'arbre entier.
//! - `DisjointWrite` et `Overlap` renvoient `Clean` : les variantes existent,
//!   la logique attend la v0.4.
//!
//! **Rien n'est bloque en v0.1.** Le registre observe, journalise et informe.
//! Le blocage viendra apres mesure du taux reel de faux positifs.
//!
//! Ce crate est vide en phase 0.

use std::time::Duration;

/// Duree de vie d'une entree du read-set.
///
/// Au-dela, on considere que le contexte de l'agent a suffisamment tourne pour
/// que l'avertissement soit du bruit. C'est le premier cadran a tourner si le
/// taux de faux positifs mesure est trop haut.
pub const READ_SET_TTL: Duration = Duration::from_secs(10 * 60);
