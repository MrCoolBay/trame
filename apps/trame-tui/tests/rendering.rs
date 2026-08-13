// Un test d'integration est un binaire ordinaire : les exemptions `allow-*-in-tests` de
// `clippy.toml` ne s'y appliquent pas.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Ce que l'interface **montre reellement**, relu dans le buffer.
//!
//! On draw dans un `TestBackend` et on relit les cellules. Affirmer « les `StaleRead`
//! sont visuellement distincts » sans relire le rendu serait une affirmation non mesuree —
//! le mode d'echec que ce projet a paye deux fois.

use std::path::PathBuf;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use trame_core::clock::{Clock, ManualClock};
use trame_core::{Seq, SessionId, SessionState, StaleFile, Verdict};
use trame_daemon::{Observation, Transport};
use trame_tui::app::{App, Kind};
use trame_tui::ui;

/// Dessine l'state et rend le buffer, pour le relire.
fn draw(app: &App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(110, 24)).expect("terminal de test");
    terminal
        .draw(|frame| ui::render(frame, app))
        .expect("rendu impossible");
    terminal.backend().buffer().clone()
}

/// Le content textuel du buffer, une chaine par line.
fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .filter_map(|x| buffer.cell((x, y)).map(|c| c.symbol().to_owned()))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

/// Le style de la cellule ou commence ce fragment.
///
/// **Surtout pas la colonne 0** : elle porte la bordure du cadre, dont le style est le meme
/// pour toutes les lines. La premiere version de ce helper lisait la bordure et concluait
/// que deux natures differentes se peignaient pareil — le test a trouve le helper, pas le
/// rendu.
fn style_of_fragment(buffer: &Buffer, fragment: &str) -> ratatui::style::Style {
    let toutes = lines(buffer);
    let (index, line) = toutes
        .iter()
        .enumerate()
        .find(|(_, l)| l.contains(fragment))
        .unwrap_or_else(|| panic!("fragment absent du rendu : {fragment}"));
    let y = u16::try_from(index).expect("hauteur raisonnable");
    // Position en *caracteres* : une cellule du buffer porte un caractere, pas un octet.
    let octets = line.find(fragment).expect("fragment present");
    let x = u16::try_from(line[..octets].chars().count()).expect("largeur raisonnable");
    buffer.cell((x, y)).expect("cellule presente").style()
}

fn app_with(observations: Vec<Observation>) -> App {
    let clock: Arc<dyn Clock> = Arc::new(ManualClock::new());
    let mut app = App::new("projet-demo", clock);
    for observation in observations {
        app.apply(observation);
    }
    app
}

fn opened(name: &str, transport: Transport) -> (SessionId, Observation) {
    let session = SessionId::new();
    (
        session,
        Observation::SessionOpened {
            session,
            name: name.to_owned(),
            transport,
        },
    )
}

fn stale(writer: SessionId, path: &str, name: &str) -> Verdict {
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

/// ★ La propriete centrale de l'interface : un `StaleRead` ne ressemble pas a un `Clean`.
///
/// Verifie sur **deux** axes, parce qu'un seul ne suffit pas : le style, et un marker
/// textuel. La color seule disparait en niveaux de gris et dans une capture d'ecran.
#[test]
fn un_stale_read_est_visuellement_distinct_d_un_clean() {
    let (a, ouvre_a) = opened("session-a", Transport::Acp);
    let (b, ouvre_b) = opened("session-b", Transport::Acp);
    let app = app_with(vec![
        ouvre_a,
        ouvre_b,
        Observation::Write {
            session: b,
            path: PathBuf::from("auth.rs"),
            verdict: Verdict::Clean,
        },
        Observation::Write {
            session: a,
            path: PathBuf::from("handlers.rs"),
            verdict: stale(b, "auth.rs", "session-b"),
        },
    ]);
    let buffer = draw(&app);

    // Le verbe complet et rembourre : « wrote » seul apparait aussi dans le panneau
    // du panel, qui est draw plus haut — `find` y tomberait d'abord.
    let style_clean = style_of_fragment(&buffer, "wrote    auth.rs");
    let style_stale = style_of_fragment(&buffer, "WROTE    handlers.rs");
    assert_ne!(
        style_clean, style_stale,
        "un Clean et un StaleRead peints pareil rendent le mecanisme central invisible"
    );
    // On compare ce qui est affirme — color et emphase — et pas la `Style` entiere : une
    // cellule rendue a ses champs resolus (`bg: Reset`) la ou la `Style` source les laisse
    // a `None`. Comparer les deux structures ferait echouer un test sur une propriete que
    // personne ne cherchait a garantir.
    let attendu = ui::kind_style(Kind::Stale);
    assert_eq!(style_stale.fg, attendu.fg, "la color du StaleRead");
    assert_eq!(
        style_stale.add_modifier, attendu.add_modifier,
        "l'emphase du StaleRead"
    );

    let toutes = lines(&buffer);
    let ligne_stale = toutes
        .iter()
        .find(|l| l.contains("handlers.rs"))
        .expect("la line StaleRead est affichee");
    let ligne_clean = toutes
        .iter()
        .find(|l| l.contains("auth.rs") && !l.contains("handlers.rs"))
        .expect("la line Clean est affichee");
    assert!(
        ligne_stale.contains('▲'),
        "la gravite doit etre lisible sans color : {ligne_stale}"
    );
    assert!(
        !ligne_clean.contains('▲'),
        "une line propre doit rester discrete : {ligne_clean}"
    );
    assert!(
        ligne_stale.contains("auth.rs") && ligne_stale.contains("session-b"),
        "l'avis nomme le file perime et son auteur : {ligne_stale}"
    );
}

/// Le watcher constate apres coup. L'interface ne doit pas laisser croire a une admission.
#[test]
fn une_ecriture_observee_est_affichee_comme_telle() {
    let (_a, ouvre_a) = opened("session-a", Transport::Acp);
    let app = app_with(vec![
        ouvre_a,
        Observation::ExternalWrite {
            path: PathBuf::from("notes.txt"),
        },
    ]);
    let buffer = draw(&app);
    let toutes = lines(&buffer);

    let line = toutes
        .iter()
        .find(|l| l.contains("notes.txt"))
        .expect("l'ecriture hors-bande est affichee");
    assert!(
        line.contains("observe") && line.contains("sans verdict"),
        "l'utilisateur doit lire que personne n'a admis cette ecriture : {line}"
    );
    assert!(
        line.contains('—'),
        "aucune session ne doit etre imputee : {line}"
    );
    assert_ne!(
        style_of_fragment(&buffer, "notes.txt"),
        ui::kind_style(Kind::Clean),
        "observe et admis ne se peignent pas pareil"
    );
    assert!(
        toutes[0].contains("1 hors-bande"),
        "le compteur hors-bande est visible en tete : {}",
        toutes[0]
    );
}

/// Un utilisateur qui croit avoir la garantie d'admission sans l'avoir est dans une
/// situation pire que sans outil. La banniere doit donc dire **ce qui** n'est pas garanti.
#[test]
fn la_degradation_est_criee_et_nommee() {
    let (_p, ouvre_pty) = opened("session-pty", Transport::Pty);
    let app = app_with(vec![ouvre_pty]);
    let toutes = lines(&draw(&app));
    let banniere = toutes
        .iter()
        .find(|l| l.contains("DEGRADE"))
        .expect("la banniere de degraded_banner est affichee");
    assert!(
        banniere.contains("session-pty"),
        "elle nomme la session concernee : {banniere}"
    );
    assert!(
        banniere.contains("ecritures") && banniere.contains("PAS"),
        "« mode degrade » ne dit rien ; il faut dire ce qui manque : {banniere}"
    );
}

/// Controle negatif de la banniere : sans session degradee, elle ne doit pas apparaitre.
/// Une banniere permanente serait du bruit, et le bruit fait desactiver l'outil.
#[test]
fn sans_degradation_aucune_banniere() {
    let (_a, ouvre_a) = opened("session-a", Transport::Acp);
    let app = app_with(vec![ouvre_a]);
    assert!(
        !lines(&draw(&app)).iter().any(|l| l.contains("DEGRADE")),
        "en ACP, rien ne doit crier"
    );
}

#[test]
fn les_panneaux_montrent_l_etat_et_le_transport() {
    let (a, ouvre_a) = opened("session-a", Transport::Acp);
    let app = app_with(vec![
        ouvre_a,
        Observation::StateChanged {
            session: a,
            state: SessionState::Writing,
        },
    ]);
    let toutes = lines(&draw(&app));
    let rendu = toutes.join("\n");
    assert!(rendu.contains("session-a"), "le nom de la session");
    assert!(
        rendu.contains(SessionState::Writing.label()),
        "l'state courant : {rendu}"
    );
    assert!(rendu.contains("transport ACP"), "le transport : {rendu}");
}

/// Une interface vide doit le dire, pas afficher un cadre muet qui laisse croire a une
/// panne.
#[test]
fn sans_session_l_interface_le_dit() {
    let app = app_with(vec![]);
    let rendu = lines(&draw(&app)).join("\n");
    assert!(rendu.contains("aucune session"), "{rendu}");
}

/// Un feed troue presente comme complet serait un mensonge : la perte s'affiche.
#[test]
fn les_observations_perdues_sont_visibles() {
    let app = app_with(vec![Observation::Lost { count: 7 }]);
    let toutes = lines(&draw(&app));
    assert!(
        toutes[0].contains("7 perdues"),
        "compteur en tete : {}",
        toutes[0]
    );
    assert!(
        toutes.iter().any(|l| l.contains("incomplet")),
        "et une line de feed qui l'explique"
    );
}

/// Le rendu ne doit pas paniquer dans un terminal minuscule : une interface qui casse au
/// redimensionnement est une interface qu'on ferme.
#[test]
fn un_terminal_minuscule_ne_casse_pas() {
    let (a, ouvre_a) = opened("session-a", Transport::Pty);
    let app = app_with(vec![
        ouvre_a,
        Observation::Read {
            session: a,
            path: PathBuf::from("auth.rs"),
        },
    ]);
    for (largeur, hauteur) in [(20, 5), (1, 1), (200, 60)] {
        let mut terminal =
            Terminal::new(TestBackend::new(largeur, hauteur)).expect("terminal de test");
        terminal
            .draw(|frame| ui::render(frame, &app))
            .unwrap_or_else(|e| panic!("rendu casse en {largeur}x{hauteur} : {e}"));
    }
}
