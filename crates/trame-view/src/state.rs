//! L'state d'affichage. **Pur, synchrone, sans moteur de rendu.**
//!
//! Les proprietes garanties ici — un `StaleRead` distinct d'un `Clean`, une ecriture observee
//! jamais presentee comme admise — sont des proprietes de l'state autant que du dessin. Ici
//! elles se testent sans terminal et sans fenetre ; chaque interface verifie ensuite qu'elles
//! arrivent bien a l'ecran.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use trame_core::clock::{Clock, Timestamp};
use trame_core::{SessionId, SessionState, Verdict};
use trame_daemon::{Observation, Transport};

/// Nombre de lines de feed conservees.
///
/// Borne, comme tout le reste : une session longue produirait sinon une croissance sans
/// fin, et personne ne remonte de mille lines dans un terminal.
pub const FEED_CAPACITY: usize = 500;

/// La nature d'une line de feed. **C'est elle qui porte la distinction visuelle.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Une lecture entree dans le read-set.
    Read,
    /// Une ecriture admise sans rien de notable. ~95 % du trafic.
    Clean,
    /// ★ Une ecriture admise alors qu'une lecture de la session est perimee.
    Stale,
    /// Une ecriture refusee.
    Refused,
    /// Un avis pose devant le prochain prompt.
    Notice,
    /// Une ecriture **hors-bande**, constatee apres coup. Sans verdict.
    Observed,
    /// Des observations perdues.
    Lost,
    /// ★ Un cumul d'avis **potential** — ce que les lectures `Grep` auraient produit.
    ///
    /// Volontairement une nature a part, et volontairement **non notable** : ce n'est pas un
    /// avis, personne n'a ete averti. Le confondre avec un avis reel annoncerait une couverture
    /// qui n'existe pas (ADR 0027).
    Shadow,
}

impl Kind {
    /// Vrai si cette line merite l'attention de l'utilisateur.
    ///
    /// Sert au rendu : ce qui est propre reste discret. ~95 % du trafic doit passer sans
    /// un mot, sinon l'outil est desactive en une semaine.
    #[must_use]
    pub const fn is_notable(self) -> bool {
        matches!(self, Self::Stale | Self::Refused | Self::Lost)
    }

    /// Le verbe affiche en tete de line.
    #[must_use]
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Read => "read",
            // La casse porte la gravite : `WROTE` en capitales pour l'ecriture qui merite
            // l'attention, `wrote` pour celle qui ne la merite pas.
            Self::Clean => "wrote",
            Self::Stale => "WROTE",
            Self::Refused => "denied",
            Self::Notice => "notice",
            Self::Observed => "observed",
            Self::Lost => "lost",
            Self::Shadow => "shadow",
        }
    }
}

/// Une line du feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Quand.
    pub at: Timestamp,
    /// Le nom de la session, ou `None` pour le hors-bande — **que personne n'a demande**.
    pub session: Option<String>,
    /// Sa nature.
    pub kind: Kind,
    /// Le path concerne, si la line en porte un.
    pub path: String,
    /// Le detail affiche a droite. Vide quand il n'y a rien a dire.
    pub detail: String,
}

/// Un panel de session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    /// Son identifiant.
    pub session: SessionId,
    /// Son nom affichable.
    pub name: String,
    /// Son state courant.
    pub state: SessionState,
    /// Le transport, donc ce qui est garanti.
    pub transport: Transport,
    /// Combien de lectures sont entrees dans son read-set.
    pub reads: usize,
    /// Combien d'ecritures ont ete admises.
    pub writes: usize,
    /// Combien de `StaleRead`. **Le compteur qui compte.**
    pub stale: usize,
    /// Combien d'avis lui ont ete poses.
    pub notices: usize,
}

impl Panel {
    /// Le symbole d'state, une colonne.
    #[must_use]
    pub const fn state_symbol(&self) -> &'static str {
        match self.state {
            SessionState::Idle => "○",
            SessionState::Thinking => "◐",
            SessionState::Writing => "●",
            SessionState::AwaitingPermission => "?",
            SessionState::Done => "✓",
            SessionState::Failed(_) => "✗",
            // `SessionState` est `#[non_exhaustive]` : un state futur s'affiche plutot que
            // de casser la compilation de l'interface.
            _ => "·",
        }
    }

    /// Vrai si l'interface doit crier la banniere de degradation pour cette session.
    #[must_use]
    pub const fn is_degraded(&self) -> bool {
        self.transport.is_degraded()
    }
}

/// L'state complet de l'interface.
pub struct App {
    /// Le nom du projet observe.
    pub project: String,
    /// Les panels, dans l'ordre d'apparition des sessions.
    pub panels: Vec<Panel>,
    /// Le feed, du plus ancien au plus recent.
    pub feed: VecDeque<Line>,
    /// Combien d'observations ont ete perdues en tout.
    pub lost: u64,
    /// ★ Le cumul d'avis **potential** du mode shadow.
    ///
    /// Compte a part des avis reels, et affiche a part : ce sont des avis qui n'ont PAS ete
    /// emis. C'est la donnee qui decidera si le trou lecture peut se fermer (ADR 0027).
    pub potential_notices: u64,
    /// Combien d'ecritures hors-bande ont ete constatees.
    ///
    /// Compte a part des ecritures admises : les melanger laisserait croire a une
    /// couverture qui n'existe pas.
    pub observed_writes: usize,
    /// L'interface doit-elle se fermer.
    pub quit: bool,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for App {
    /// Manuel : une horloge n'est pas `Debug`, et l'`Arc` qui la porte n'est pas de l'state
    /// metier — c'est une valeur immuable, donc l'invariant sur `Arc` ne s'applique pas.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("project", &self.project)
            .field("panels", &self.panels)
            .field("feed", &self.feed.len())
            .field("lost", &self.lost)
            .field("observed_writes", &self.observed_writes)
            .finish_non_exhaustive()
    }
}

impl App {
    /// Construit l'state initial.
    ///
    /// L'horloge est injectee : le feed est horodate, et un test qui depend de l'heure
    /// systeme est un test qu'on ne peut pas epingler.
    #[must_use]
    pub fn new(project: impl Into<String>, clock: Arc<dyn Clock>) -> Self {
        Self {
            project: project.into(),
            panels: Vec::new(),
            feed: VecDeque::with_capacity(FEED_CAPACITY),
            lost: 0,
            potential_notices: 0,
            observed_writes: 0,
            quit: false,
            clock,
        }
    }

    /// Vrai si **au moins une** session tourne en mode degrade.
    ///
    /// Une seule suffit a afficher la banniere : la garantie n'est pas partielle du point
    /// de vue de l'utilisateur, elle est absente pour cette session-la.
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.panels.iter().any(Panel::is_degraded)
    }

    /// ★ Applique une observation. **Le seul point d'entree de l'state.**
    pub fn apply(&mut self, observation: Observation) {
        let at = self.clock.now();
        match observation {
            Observation::SessionOpened {
                session,
                name,
                transport,
            } => {
                if let Some(panel) = self.panel_mut(session) {
                    panel.name = name;
                    panel.transport = transport;
                } else {
                    self.panels.push(Panel {
                        session,
                        name,
                        state: SessionState::Idle,
                        transport,
                        reads: 0,
                        writes: 0,
                        stale: 0,
                        notices: 0,
                    });
                }
            }

            Observation::StateChanged { session, state } => {
                if let Some(panel) = self.panel_mut(session) {
                    panel.state = state;
                }
            }

            Observation::Read { session, path } => {
                if let Some(panel) = self.panel_mut(session) {
                    panel.reads += 1;
                }
                let name = self.name_of(session);
                self.push(Line {
                    at,
                    session: name,
                    kind: Kind::Read,
                    path: display(&path),
                    detail: String::new(),
                });
            }

            Observation::Write {
                session,
                path,
                verdict,
            } => {
                let stale = verdict.needs_notice();
                if let Some(panel) = self.panel_mut(session) {
                    panel.writes += 1;
                    if stale {
                        panel.stale += 1;
                    }
                }
                let name = self.name_of(session);
                self.push(Line {
                    at,
                    session: name,
                    kind: if stale { Kind::Stale } else { Kind::Clean },
                    path: display(&path),
                    detail: describe(&verdict),
                });
            }

            Observation::Refused {
                session,
                path,
                reason,
            } => {
                let name = self.name_of(session);
                self.push(Line {
                    at,
                    session: name,
                    kind: Kind::Refused,
                    path: display(&path),
                    detail: reason,
                });
            }

            Observation::Notice { session, text } => {
                if let Some(panel) = self.panel_mut(session) {
                    panel.notices += 1;
                }
                let name = self.name_of(session);
                self.push(Line {
                    at,
                    session: name,
                    kind: Kind::Notice,
                    path: String::new(),
                    // L'avis est multiligne ; le feed en montre la premiere line.
                    detail: text.lines().next().unwrap_or_default().to_owned(),
                });
            }

            Observation::ExternalWrite { path } => {
                self.observed_writes += 1;
                self.push(Line {
                    at,
                    // Pas de session : le watcher constate, personne n'a demande.
                    session: None,
                    kind: Kind::Observed,
                    path: display(&path),
                    // Non negociable : l'utilisateur doit lire que rien n'a ete admis.
                    detail: "hors-bande, sans verdict".to_owned(),
                });
            }

            Observation::PotentialNotices { total } => {
                // On ne pousse une line que si le cumul a bouge, et on l'ecrit comme une
                // mesure : « auraient ete emis », jamais « ont ete emis ».
                if total != self.potential_notices {
                    self.potential_notices = total;
                    self.push(Line {
                        at,
                        session: None,
                        kind: Kind::Shadow,
                        path: String::new(),
                        detail: format!(
                            "{total} avis auraient ete emis si les lectures Grep comptaient \
                             (mode shadow, rien n'a ete injecte)"
                        ),
                    });
                }
            }

            Observation::Lost { count } => {
                self.lost = self.lost.saturating_add(count);
                self.push(Line {
                    at,
                    session: None,
                    kind: Kind::Lost,
                    path: String::new(),
                    detail: format!("{count} observations perdues, affichage incomplet"),
                });
            }

            _ => {}
        }
    }

    fn push(&mut self, line: Line) {
        if self.feed.len() == FEED_CAPACITY {
            self.feed.pop_front();
        }
        self.feed.push_back(line);
    }

    fn panel_mut(&mut self, session: SessionId) -> Option<&mut Panel> {
        self.panels.iter_mut().find(|p| p.session == session)
    }

    fn name_of(&self, session: SessionId) -> Option<String> {
        self.panels
            .iter()
            .find(|p| p.session == session)
            .map(|p| p.name.clone())
            // Une session inconnue du panel guard une trace lisible plutot que d'etre
            // muette : un feed qui masque ce qu'il ne sait pas nommer cache un bug.
            .or_else(|| Some(format!("session {}", &session.to_string()[..8])))
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

/// A verdict's detail, as it is displayed.
///
/// `StaleRead` names the stale files and their last writer — exactly the information that
/// makes the notice actionable. No summary of the change: the registry computes none
/// (ADR 0018).
///
/// # The truncation, and what it costs
///
/// Only the **first** stale file is named, followed by a count of the rest. A session that
/// has read eight files another session then changed shows one name and `+7 more`.
///
/// That is lossy on purpose — a feed row has to stay one line — but it is worth being
/// explicit about the trade: the row says *how many* worlds have moved under this session,
/// not *which*. The full list is in the injected notice, which is what the agent reads, and
/// in the journal, which is what an audit reads. The row is a signal, not the record.
///
/// The count is written `+N more` rather than `+N`, because a bare `+7` next to a path
/// reads like a line number or a sequence — it was in fact read that way.
fn describe(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Clean => String::new(),
        Verdict::StaleRead { stale } => {
            let mut parts = stale
                .iter()
                .map(|f| format!("{} (by {})", f.path.display(), f.last_writer_name));
            match parts.next() {
                None => "stale read".to_owned(),
                Some(first) if stale.len() == 1 => format!("stale: {first}"),
                Some(first) => format!("stale: {first} +{} more", stale.len() - 1),
            }
        }
        other => other.label().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use trame_core::clock::ManualClock;
    use trame_core::{Seq, StaleFile};

    use super::*;

    fn app() -> (App, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new());
        (App::new("demo", clock.clone()), clock)
    }

    fn ouvre(app: &mut App, name: &str, transport: Transport) -> SessionId {
        let session = SessionId::new();
        app.apply(Observation::SessionOpened {
            session,
            name: name.to_owned(),
            transport,
        });
        session
    }

    fn stale_read(writer: SessionId, path: &str, name: &str) -> Verdict {
        let now = ManualClock::new().now();
        Verdict::StaleRead {
            stale: vec![StaleFile {
                path: PathBuf::from(path),
                last_writer: writer,
                last_writer_name: name.to_owned(),
                read_at: now,
                written_at: now,
                seq: Seq::from_u64(1),
            }],
        }
    }

    /// ★ Le scenario canonique, vu par l'interface. C'est la raison d'etre du produit :
    /// deux fichiers differents, aucune collision d'ecriture, et pourtant un avis.
    #[test]
    fn the_canonical_scenario_produces_one_visually_distinct_line() {
        let (mut app, _clock) = app();
        let a = ouvre(&mut app, "session-a", Transport::Acp);
        let b = ouvre(&mut app, "session-b", Transport::Acp);

        app.apply(Observation::Read {
            session: a,
            path: PathBuf::from("auth.rs"),
        });
        app.apply(Observation::Write {
            session: b,
            path: PathBuf::from("auth.rs"),
            verdict: Verdict::Clean,
        });
        app.apply(Observation::Write {
            session: a,
            path: PathBuf::from("handlers.rs"),
            verdict: stale_read(b, "auth.rs", "session-b"),
        });

        let kinds: Vec<Kind> = app.feed.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, vec![Kind::Read, Kind::Clean, Kind::Stale]);
        assert!(
            !Kind::Clean.is_notable() && Kind::Stale.is_notable(),
            "un Clean et un StaleRead ne doivent pas se ressembler"
        );

        let derniere = app.feed.back().unwrap();
        assert_eq!(derniere.path, "handlers.rs");
        assert!(
            derniere.detail.contains("auth.rs") && derniere.detail.contains("session-b"),
            "l'avis nomme le file perime et qui l'a modifie, sinon il n'est pas \
             actionnable : {}",
            derniere.detail
        );

        let panel_a = app.panels.iter().find(|p| p.session == a).unwrap();
        assert_eq!((panel_a.reads, panel_a.writes, panel_a.stale), (1, 1, 1));
        let panel_b = app.panels.iter().find(|p| p.session == b).unwrap();
        assert_eq!(panel_b.stale, 0, "B n'a rien lu de perime");
    }

    /// Une ecriture hors-bande n'a pas de verdict et n'a pas de session. L'interface ne
    /// doit inventer ni l'un ni l'autre.
    #[test]
    fn an_observed_write_is_never_presented_as_admitted() {
        let (mut app, _clock) = app();
        let a = ouvre(&mut app, "session-a", Transport::Acp);
        app.apply(Observation::ExternalWrite {
            path: PathBuf::from("notes.txt"),
        });

        let line = app.feed.back().unwrap();
        assert_eq!(line.kind, Kind::Observed);
        assert_eq!(line.session, None, "personne ne l'a demandee");
        assert!(line.detail.contains("sans verdict"));
        assert_eq!(app.observed_writes, 1);
        assert_eq!(
            app.panels.iter().find(|p| p.session == a).unwrap().writes,
            0,
            "une ecriture observee ne se compte pas comme une ecriture admise"
        );
    }

    #[test]
    fn session_states_follow_the_transitions() {
        let (mut app, _clock) = app();
        let a = ouvre(&mut app, "session-a", Transport::Acp);
        let displayed = |app: &App| app.panels[0].state.clone();

        assert_eq!(displayed(&app), SessionState::Idle);
        for state in [
            SessionState::Thinking,
            SessionState::Writing,
            SessionState::Idle,
        ] {
            app.apply(Observation::StateChanged {
                session: a,
                state: state.clone(),
            });
            assert_eq!(displayed(&app), state);
        }
    }

    #[test]
    fn a_pty_session_raises_the_degradation_banner() {
        let (mut app, _clock) = app();
        ouvre(&mut app, "session-acp", Transport::Acp);
        assert!(!app.is_degraded());
        ouvre(&mut app, "session-pty", Transport::Pty);
        assert!(
            app.is_degraded(),
            "une seule session sans interception suffit a l'afficher"
        );
    }

    #[test]
    fn dropped_observations_are_shown_and_accumulated() {
        let (mut app, _clock) = app();
        app.apply(Observation::Lost { count: 3 });
        app.apply(Observation::Lost { count: 2 });
        assert_eq!(app.lost, 5);
        assert!(app.feed.back().unwrap().detail.contains("incomplet"));
    }

    /// Le feed est limit. Sans ca, une session longue fait croitre la memoire sans fin.
    #[test]
    fn the_feed_is_bounded_and_keeps_the_most_recent_entries() {
        let (mut app, _clock) = app();
        let a = ouvre(&mut app, "session-a", Transport::Acp);
        for i in 0..FEED_CAPACITY + 10 {
            app.apply(Observation::Read {
                session: a,
                path: PathBuf::from(format!("f{i}.rs")),
            });
        }
        assert_eq!(app.feed.len(), FEED_CAPACITY);
        assert_eq!(
            app.feed.back().unwrap().path,
            format!("f{}.rs", FEED_CAPACITY + 9)
        );
    }

    /// Le feed est horodate par l'horloge injectee, pas par l'heure systeme.
    #[test]
    fn the_feed_is_timestamped_by_the_injected_clock() {
        let (mut app, clock) = app();
        let a = ouvre(&mut app, "session-a", Transport::Acp);
        let debut = clock.now();
        clock.advance(chrono::TimeDelta::seconds(42));
        app.apply(Observation::Read {
            session: a,
            path: PathBuf::from("auth.rs"),
        });
        assert_eq!(
            app.feed.back().unwrap().at,
            debut + chrono::TimeDelta::seconds(42)
        );
    }

    /// ★ Two events inside the same second must not render identically.
    ///
    /// The feed was `%H:%M:%S`, and in a real scenario run three or four consecutive rows
    /// carried the same timestamp — so the feed said nothing about the order of the events
    /// it is ordered by. On a product whose whole claim is "this changed **since** you read
    /// it", that undercuts the argument the screen exists to make.
    ///
    /// Pinned as a property rather than as a format string: what matters is that two events
    /// 40 ms apart are distinguishable, not that the separator is a dot.
    ///
    /// Its negative control: setting `TIME_FORMAT` back to second resolution fails this.
    #[test]
    fn two_events_in_the_same_second_render_distinguishably() {
        let (mut app, clock) = app();
        let a = ouvre(&mut app, "session-a", Transport::Acp);

        app.apply(Observation::Read {
            session: a,
            path: PathBuf::from("auth.rs"),
        });
        clock.advance(chrono::TimeDelta::milliseconds(40));
        app.apply(Observation::Read {
            session: a,
            path: PathBuf::from("handlers.rs"),
        });

        let mut rows = app.feed.iter().rev();
        let second = rows.next().expect("two rows").at;
        let first = rows.next().expect("two rows").at;
        assert_eq!(
            second - first,
            chrono::TimeDelta::milliseconds(40),
            "the clock must carry the difference"
        );
        assert_ne!(
            first.format(crate::TIME_FORMAT).to_string(),
            second.format(crate::TIME_FORMAT).to_string(),
            "two events 40 ms apart render identically, so the feed says nothing about the \
             order of the events it is ordered by"
        );
    }
}
