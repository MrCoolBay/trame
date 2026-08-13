//! Ce que **toute interface Trame** partage, quel que soit son moteur de rendu.
//!
//! Deux choses, et rien d'autre :
//!
//! - [`state`] — l'state d'affichage. Pur, synchrone, sans moteur de rendu.
//! - [`source`] — l'ouverture d'un projet : journal, registre, watcher, et le feed
//!   d'observations qui en sort.
//!
//! # Pourquoi une crate et pas un module de chaque interface
//!
//! [`state`] ne contient pas de la mise en forme, il contient les **proprietes qu'une
//! interface Trame doit tenir** : un `StaleRead` est notable et un `Clean` ne l'est pas, une
//! ecriture observee ne se compte pas comme une ecriture admise, une seule session degradee
//! suffit a l'afficher, le feed est limit.
//!
//! Recopier ces regles dans la TUI et dans la GUI, ce serait se donner deux endroits ou elles
//! peuvent diverger — et ce sont precisement les regles dont l'ADR 0022 fait le contrat
//! d'affichage du produit. Un seul domicile.
//!
//! Ce qui reste propre a chaque interface : les couleurs, les caracteres, la disposition.
//! Ce qui est ici : **ce qu'on affiche, et pourquoi ca merite l'attention**.
//!
//! Position dans la chaine de dependances :
//! `core <- journal <- registry <- {agent, vcs} <- daemon <- view <- {tui, gui}`.

pub mod source;
pub mod state;

pub use state::{App, FEED_CAPACITY, Kind, Line, Panel};
