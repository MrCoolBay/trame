//! Couche VCS, une par projet. Working directory unique, jamais de worktree.
//!
//! # L'attribution est une donnee, pas une heuristique
//!
//! Chaque ecriture admise porte son `session_id`, donc sa branche virtuelle.
//! L'assignation hunk -> branche n'est pas devinee apres coup : elle est connue
//! au moment de l'ecriture. Trois agents terminent, trois branches sont deja
//! correctement remplies, zero tri manuel.
//!
//! # Shell-out vers `but`, assume
//!
//! `ButBackend` appelle la CLI GitButler en sous-process, avec `--format json`
//! systematiquement : API structuree, pas du scraping de sortie humaine. La
//! surface necessaire fait une petite dizaine de commandes. Reimplementer les
//! branches virtuelles serait six a dix-huit mois de travail sur ce qui est,
//! pour Trame, une commodite.
//!
//! `but` est traite comme une **dependance externe installee par l'utilisateur**,
//! jamais vendorisee — de la meme facon qu'un orchestrateur d'agents ne livre pas
//! Claude Code avec lui.
//!
//! `GixBackend`, une reimplementation native sur `gitoxide`, est une sortie
//! possible a long terme. Pas un objectif de la v0.1.
//!
//! Ce crate est vide en phase 0.

/// Le binaire attendu sur le `PATH`.
///
/// S'il est absent, Trame s'arrete et le dit. Il ne bascule **jamais** sur du
/// git nu : le modele de branches virtuelles n'a pas d'equivalent en git, et
/// simuler l'un avec l'autre produirait des attributions fausses.
pub const BUT_BINARY: &str = "but";

/// Version minimale de la CLI validee avec ce code.
pub const BUT_MIN_VERSION: &str = "0.21";
