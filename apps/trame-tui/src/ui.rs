//! Le rendu. **Trois distinctions a ne jamais perdre.**
//!
//! 1. Un `StaleRead` ne ressemble pas a un `Clean`. C'est le seul verdict qui compte, et
//!    il doit sauter aux yeux dans un feed de lines propres.
//! 2. Une ecriture **observee** ne ressemble pas a une ecriture **admise**. Le watcher
//!    constate apres coup ; laisser croire l'inverse serait promettre une garantie qu'on
//!    n'a pas.
//! 3. Une session **degradee** le dit. Un utilisateur qui croit avoir l'admission sans
//!    l'avoir est dans une situation pire que sans outil.
//!
//! Le reste doit etre discret : ~95 % du trafic passe sans un mot.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TextLine, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use trame_view::{App, Kind, Line, Panel};

/// La color d'une nature de line.
///
/// Fonction publique et sans state : les tests de rendu s'appuient dessus pour check que
/// deux natures **differentes** ne se peignent pas pareil.
#[must_use]
pub const fn kind_style(kind: Kind) -> Style {
    match kind {
        // Le propre est terne, a dessein.
        Kind::Read => Style::new().fg(Color::DarkGray),
        Kind::Clean => Style::new().fg(Color::Gray),
        // ★ Le seul verdict qui compte.
        Kind::Stale => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        Kind::Refused => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        Kind::Notice => Style::new().fg(Color::Cyan),
        // Hors-bande : une color a soi, parce que ce n'est pas une admission.
        Kind::Observed => Style::new().fg(Color::Magenta),
        Kind::Lost => Style::new().fg(Color::Red).add_modifier(Modifier::REVERSED),
        // Le mode shadow est une mesure, pas un avis : terne et sans emphase, pour qu'on ne le
        // confonde jamais avec un StaleRead.
        Kind::Shadow => Style::new().fg(Color::Blue),
    }
}

/// Dessine l'interface entiere.
pub fn render(frame: &mut Frame, app: &App) {
    let banniere = u16::from(app.is_degraded());
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),        // titre
            Constraint::Length(banniere), // degraded_banner, seulement si elle existe
            Constraint::Length(6),        // panels de session
            Constraint::Min(3),           // feed
        ])
        .split(frame.area());

    render_title(frame, zones[0], app);
    if banniere == 1 {
        render_degraded(frame, zones[1], app);
    }
    render_panels(frame, zones[2], app);
    render_feed(frame, zones[3], app);
}

fn render_title(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " Trame ",
            Style::new()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {} ", app.project)),
        Span::styled(
            format!("· {} session(s)", app.panels.len()),
            Style::new().fg(Color::DarkGray),
        ),
    ];
    if app.observed_writes > 0 {
        spans.push(Span::styled(
            format!(" · {} hors-bande", app.observed_writes),
            kind_style(Kind::Observed),
        ));
    }
    if app.potential_notices > 0 {
        spans.push(Span::styled(
            format!(" · {} potential (shadow)", app.potential_notices),
            kind_style(Kind::Shadow),
        ));
    }
    if app.lost > 0 {
        spans.push(Span::styled(
            format!(" · {} perdues", app.lost),
            kind_style(Kind::Lost),
        ));
    }
    spans.push(Span::styled(
        "   q pour quitter",
        Style::new().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(TextLine::from(spans)), area);
}

/// La banniere de degraded_banner. **Explicite sur ce qui n'est pas garanti.**
///
/// « Mode degrade » ne dit rien a personne. « Les ecritures ne sont pas interceptees » dit
/// exactement ce qui manque.
fn render_degraded(frame: &mut Frame, area: Rect, app: &App) {
    let noms: Vec<&str> = app
        .panels
        .iter()
        .filter(|p| p.is_degraded())
        .map(|p| p.name.as_str())
        .collect();
    let texte = format!(
        " ⚠ DEGRADE — {} : les ecritures ne sont PAS interceptees avant le disque ",
        noms.join(", ")
    );
    frame.render_widget(
        Paragraph::new(TextLine::from(Span::styled(
            texte,
            Style::new()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ))),
        area,
    );
}

fn render_panels(frame: &mut Frame, area: Rect, app: &App) {
    if app.panels.is_empty() {
        frame.render_widget(
            Paragraph::new("  aucune session — en attente d'observations")
                .style(Style::new().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL).title(" sessions ")),
            area,
        );
        return;
    }
    // Largeur egale : aucune session n'est plus importante qu'une autre.
    let parts =
        vec![Constraint::Ratio(1, u32::try_from(app.panels.len()).unwrap_or(1)); app.panels.len()];
    let colonnes = layout_default_horizontal(parts, area);
    for (panel, zone) in app.panels.iter().zip(colonnes.iter()) {
        render_panel(frame, *zone, panel);
    }
}

fn render_panel(frame: &mut Frame, area: Rect, panel: &Panel) {
    let (symbole, color) = match panel.state {
        trame_core::SessionState::Writing => (panel.state_symbol(), Color::Yellow),
        trame_core::SessionState::Thinking => (panel.state_symbol(), Color::Cyan),
        trame_core::SessionState::Failed(_) => (panel.state_symbol(), Color::Red),
        _ => (panel.state_symbol(), Color::DarkGray),
    };
    let transport = if panel.is_degraded() {
        Span::styled(
            format!("transport {} — non intercepte", panel.transport.label()),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!("transport {}", panel.transport.label()),
            Style::new().fg(Color::DarkGray),
        )
    };
    let lines = vec![
        TextLine::from(vec![
            Span::styled(format!("{symbole} "), Style::new().fg(color)),
            Span::styled(
                panel.state.label().to_owned(),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]),
        TextLine::from(transport),
        TextLine::from(Span::styled(
            format!("{} lues · {} ecrites", panel.reads, panel.writes),
            Style::new().fg(Color::DarkGray),
        )),
        TextLine::from(if panel.stale > 0 {
            Span::styled(
                format!("▲ {} perimee(s) · {} avis", panel.stale, panel.notices),
                kind_style(Kind::Stale),
            )
        } else {
            Span::styled("aucune lecture perimee", Style::new().fg(Color::DarkGray))
        }),
    ];
    let bordure = if panel.is_degraded() {
        Style::new().fg(Color::Red)
    } else if panel.stale > 0 {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(bordure)
                .title(format!(" {} ", panel.name)),
        ),
        area,
    );
}

fn render_feed(frame: &mut Frame, area: Rect, app: &App) {
    let bloc = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(" feed ");
    let interieur = bloc.inner(area);
    frame.render_widget(bloc, area);

    // Les plus recentes en bas, comme un terminal. On ne guard que ce qui tient.
    let hauteur = usize::from(interieur.height);
    let debut = app.feed.len().saturating_sub(hauteur);
    let lines: Vec<TextLine> = app
        .feed
        .iter()
        .skip(debut)
        .map(|line| feed_line(line))
        .collect();
    frame.render_widget(Paragraph::new(lines), interieur);
}

/// Une line de feed, telle qu'elle s'affiche.
///
/// Publique pour que les tests de rendu verifient la distinction visuelle sur la vraie
/// fonction, pas sur une reimplementation.
#[must_use]
pub fn feed_line<'a>(line: &'a Line) -> TextLine<'a> {
    let style = kind_style(line.kind);
    let mut spans = vec![
        // Un marker textuel en plus de la color : la distinction ne doit pas reposer
        // sur la seule color, qui disparait en niveaux de gris comme dans une capture.
        Span::styled(if line.kind.is_notable() { "▲ " } else { "  " }, style),
        Span::styled(
            format!("{} ", line.at.format(trame_view::TIME_FORMAT)),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{:<12} ", line.session.as_deref().unwrap_or("—")),
            Style::new().fg(Color::DarkGray),
        ),
        Span::styled(format!("{:<8} ", line.kind.verb()), style),
    ];
    if !line.path.is_empty() {
        spans.push(Span::styled(format!("{:<24} ", line.path), style));
    }
    if !line.detail.is_empty() {
        spans.push(Span::styled(line.detail.clone(), style));
    }
    TextLine::from(spans)
}

/// Decoupe horizontalement. Extrait pour garder [`render_panels`] lisible.
fn layout_default_horizontal(parts: Vec<Constraint>, area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(parts)
        .split(area)
}
