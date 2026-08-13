// A probe binary, meant for human hands. `eprintln!` is its interface.
#![allow(clippy::print_stderr, clippy::expect_used, clippy::unwrap_used)]

//! ★ **Probe: is `gpui-component` 0.5.1 a dependency we can build on?**
//!
//! Reading source told us three things about this crate: components implement gpui's
//! `Styled`, they compose as plain elements, and the caller's style refines on top of their
//! preset. **None of that is an observation** — it is a claim derived from reading, and this
//! project has been wrong that way often enough to have a rule about it.
//!
//! So this probe exists to be *used by hand*, in priority order, with a strict exit rule.
//!
//! # What it puts on screen, and why in this order
//!
//! 1. **A multi-line prompt field.** The point that decides everything: 5-10 days to write
//!    ourselves, and the thing a user notices immediately if it is approximate. Type ten
//!    lines into it. Try IME, mouse selection, click-to-place-cursor, soft wrap, copy-paste.
//! 2. **A `virtual_list` of 1000 rows**, fed from the same shape as our observation feed.
//! 3. **The theme, and a component restyled past its preset** — the `.refine_style()` claim,
//!    checked on screen rather than in source.
//! 4. **A button and a panel**, the cheap ones.
//!
//! And underneath all of it, the thing that must not break: **our own
//! `Receiver<Observation>` drives a live counter**, awaited from gpui's executor exactly as
//! `trame-gui` does it. If adopting this crate cost us that, nothing else would matter.
//!
//! # What the probe already established before it was run
//!
//! Two facts came out of wiring it, and they belong in the report whatever the screen shows:
//!
//! - `gpui-component` asks for `gpui = "0.2.2"` **without** a feature spec, so it takes
//!   gpui's defaults: `font-kit, wayland, x11, windows-manifest`. Cargo unions features and
//!   cannot subtract them, so adopting this crate turns on `wayland`, `x11`,
//!   `blade-graphics`, `cosmic-text` and the X11/Wayland client stack — **on a macOS-only
//!   application**. That is precisely what `default-features = false` was set for in
//!   ADR 0023.
//! - It is therefore a **dev-dependency** here. Under resolver v3 that keeps
//!   `cargo build -p trame-gui` free of the unification, so the shipped binary is untouched
//!   while the probe is undecided. Adoption would remove that containment.
//!
//! # Running it
//!
//! ```sh
//! just probe-component
//! ```

use std::time::Instant;

use gpui::{
    App, Application, Bounds, Context, Entity, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use gpui_component::{ActiveTheme as _, Root, button::Button, input::InputState, v_virtual_list};
use trame_core::SessionId;
use trame_daemon::{Observation, Transport, observe_channel};

/// How many rows the virtual list gets. A thousand is the figure that separates "a list" from
/// "a list that has to be virtualised".
const ROWS: usize = 1_000;

struct Probe {
    /// ★ The prompt field, built the way that actually works: `auto_grow`.
    ///
    /// See [`Probe::new`] for why this is not `multi_line(true)`.
    prompt: Entity<InputState>,
    /// `multi_line(true).rows(5)` — the documented pairing, and it works.
    ///
    /// `rows()` is a public builder on `InputState` (0.5.1, state.rs:495). An earlier round of
    /// this probe concluded there was "no public workaround" for `multi_line`; that conclusion
    /// was **wrong**, and it was wrong because the source was grepped for a list of guessed
    /// names instead of being enumerated. See the tenth case in AGENTS.md.
    paired: Entity<InputState>,
    /// `multi_line(true)` **alone**, kept on purpose: one row, whatever the content.
    ///
    /// Not a library defect — a defaults problem. `plain_text()` sets `rows: 1` while the doc
    /// on `multi_line` says "Default rows is 2". The flag needs `rows()` beside it.
    trap: Entity<InputState>,
    /// Rows for the virtual list, shaped like our feed lines so the test is not a toy.
    rows: Vec<String>,
    /// How many observations arrived from **our** channel, proving cohabitation.
    observed: usize,
    /// ★ Where the cursor is, as `line:column`, read from `InputState::cursor_position()`.
    ///
    /// Two gestures were untested after the first round, and both are the kind that a
    /// single-line field cannot exercise: **mouse selection across lines**, and
    /// **click-to-place-cursor inside a block**. Judging them by eye is exactly the sort of
    /// "it looked fine" this project keeps getting caught by, so this turns one of them into
    /// a number: click at a spot, and the readout has to name that spot.
    ///
    /// `selected_range` is `pub(super)`, so the selection cannot be read the same way. It is
    /// checked through the clipboard instead — copy a multi-line span, paste it into the
    /// second field, and the newlines either survived or they did not.
    cursor: String,
    /// Wall-clock from process start to first frame, printed on stderr.
    started: Instant,
    first_frame: Option<f64>,
}

impl Probe {
    fn new(window: &mut Window, cx: &mut Context<Self>, started: Instant) -> Self {
        // ★ `auto_grow(min, max)` is the path that lays out multiple rows.
        //
        // The first version of this probe used `.multi_line(true)` and produced a field one
        // line tall that accepted newlines — content present, scrollbar present, nothing
        // visible. Unusable for a prompt, and worse than refusing newlines outright.
        //
        // That is a defect in 0.5.1, not a mistake in the call. `InputMode::plain_text()`
        // initialises `rows: 1`, and `multi_line(bool)` only flips the boolean — it never
        // touches `rows`. At layout time `element.rs` computes
        // `max_rows().min(rows())`, so the field stays one row. The doc on
        // `InputState::multi_line` says "Default rows is 2", which is **false for this
        // path**.
        //
        // There is no public escape either: `set_rows` is `pub(super)`. From outside the
        // crate, `multi_line(true)` cannot be made to render more than one row.
        //
        // `AutoGrow { rows: min_rows, min_rows, max_rows }` starts at `min_rows` and is
        // multi-line whenever `max_rows > 1`, which is what a prompt field needs.
        let prompt = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(5, 20)
                .placeholder("auto_grow(5, 20) — paste five lines of Rust here")
        });

        // The documented pairing. `rows()` is public and this is what the official docs show.
        let paired = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .rows(5)
                .soft_wrap(true)
                .placeholder("multi_line(true).rows(5) — the documented pairing")
        });

        // Kept as a live counter-example: the flag WITHOUT rows().
        let trap = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("multi_line(true) alone — one row, because plain_text() sets rows: 1")
        });

        let rows = (0..ROWS)
            .map(|i| format!("{i:04}  module_{i:04}.rs  →  StaleRead (by refactor-api)"))
            .collect();

        // ★ Our own channel, awaited from gpui's executor. This is the mechanism the whole
        // architecture rests on (ADR 0022), and a component library that broke it would be
        // disqualified whatever its widgets look like.
        let (mut observer, mut receiver) = observe_channel();
        cx.spawn(async move |this, cx| {
            while let Some(observation) = receiver.recv().await {
                let is_row = !matches!(observation, Observation::Lost { .. });
                if this
                    .update(cx, |probe: &mut Self, cx| {
                        if is_row {
                            probe.observed += 1;
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        // A producer, so the counter actually moves while you use the field.
        // Timed through gpui's own executor rather than an extra timer crate: the point is
        // that OUR channel survives THEIR runtime, so borrowing anything else would blur it.
        let timer = cx.background_executor().clone();
        cx.background_spawn(async move {
            for index in 0..200_usize {
                observer.emit(Observation::SessionOpened {
                    session: SessionId::new(),
                    name: format!("probe-{index}"),
                    transport: Transport::Absent,
                });
                timer.timer(std::time::Duration::from_millis(200)).await;
            }
        })
        .detach();

        // Re-read the cursor whenever the field notifies. If the readout turns out to lag
        // behind a click, that is information too — it would mean the field does not notify
        // on cursor movement, and a live caret indicator would need another route.
        cx.observe(&prompt, |probe: &mut Self, state, cx| {
            let position = state.read(cx).cursor_position();
            probe.cursor = format!("{}:{}", position.line, position.character);
            cx.notify();
        })
        .detach();

        Self {
            prompt,
            paired,
            trap,
            rows,
            cursor: "0:0".to_owned(),
            observed: 0,
            started,
            first_frame: None,
        }
    }
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.first_frame.is_none() {
            let ms = self.started.elapsed().as_secs_f64() * 1000.0;
            self.first_frame = Some(ms);
            eprintln!("FIRST_FRAME_MS {ms:.0}");
        }

        let theme = cx.theme();
        let rows = self.rows.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            // 4. A panel: a bordered strip with theme tokens, nothing exotic.
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().font_weight(gpui::FontWeight::BOLD).child("probe"))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(format!("{ROWS} rows · {} observed", self.observed)),
                    )
                    // ★ The cursor readout. Click somewhere in the top field: this must name
                    // the line and column you clicked, not the end of the text.
                    .child(
                        div()
                            .px_2()
                            .rounded_sm()
                            .bg(theme.secondary)
                            .child(format!("cursor {}", self.cursor)),
                    )
                    // 4. A stock button.
                    .child(Button::new("stock").label("stock button"))
                    // ★ 3. The claim under test: our own tailwind-like methods chained onto
                    // THEIR component. If `.refine_style()` works as the source says, this
                    // button is visibly squarer, wider and differently coloured than the one
                    // above — while staying their Button.
                    .child(
                        Button::new("restyled")
                            .label("restyled past preset")
                            .px_8()
                            .rounded_none()
                            .bg(rgb(0x7c3aed))
                            .text_color(rgb(0xffffff)),
                    ),
            )
            // ★ 1. The point that decides, and its counter-example directly underneath.
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("auto_grow(5, 20) — this one is the real test"),
                    )
                    .child(gpui_component::input::Input::new(&self.prompt))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("multi_line(true).rows(5) — the documented pairing"),
                    )
                    .child(gpui_component::input::Input::new(&self.paired))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xef4444))
                            .child("multi_line(true) alone — one row: a defaults problem, not a wall"),
                    )
                    .child(gpui_component::input::Input::new(&self.trap)),
            )
            // 2. A thousand rows, virtualised.
            .child(
                div().flex_1().overflow_hidden().child(v_virtual_list(
                    cx.entity(),
                    "feed",
                    std::rc::Rc::new(vec![gpui::size(px(0.0), px(22.0)); ROWS]),
                    move |_probe, visible: std::ops::Range<usize>, _window, _cx| {
                        visible
                            .filter_map(|index| rows.get(index))
                            .map(|line| div().h(px(22.0)).px_4().child(line.clone()))
                            .collect::<Vec<_>>()
                    },
                )),
            )
    }
}

fn main() {
    let started = Instant::now();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    eprintln!(
        "gpui-component 0.5.1 probe.\n\
         \n\
         1. THREE prompt fields. Paste the SAME five lines of Rust into all three.\n\
            1st: auto_grow(5, 20)          — five rows, grows. VERIFIED.\n\
            2nd: multi_line(true).rows(5)  — the documented pairing. Should also work.\n\
            3rd: multi_line(true) alone    — ONE row, because plain_text() sets rows: 1.\n\
            The third is a DEFAULTS problem, not a missing capability: rows() is a\n\
            public builder. An earlier round of this probe concluded otherwise and\n\
            was wrong — see the tenth case in AGENTS.md.\n\
         \n\
         ★ THE TWO GESTURES STILL TO CHECK, both on the TOP field:\n\
         \n\
            a) CLICK TO PLACE THE CURSOR, inside the block, not at the end.\n\
               Watch the \"cursor L:C\" readout in the header. Click on line 3\n\
               around column 10: it must read 3:10, give or take a character.\n\
               If it jumps to the last line, click-positioning is broken.\n\
               If it does not move at all, the field does not notify on cursor\n\
               movement — also an answer, and one that matters for a caret.\n\
         \n\
            b) MOUSE SELECTION ACROSS LINES. Drag from the middle of line 2 to\n\
               the middle of line 4. Then cmd-C, click the BOTTOM field, cmd-V.\n\
               The paste must contain exactly that span, newlines included.\n\
               Partial span, lost newlines, or a whole-field copy all mean\n\
               multi-line selection does not work.\n\
         2. Scroll the 1000-row list.\n\
         3. Compare the two buttons: the right one is THEIR Button with OUR tailwind\n\
            methods chained on. If they look identical, .refine_style() does not do what\n\
            reading the source suggested.\n\
         4. The observed counter moves on its own: that is OUR Receiver<Observation>,\n\
            awaited from gpui's executor.\n\
         \n\
         Startup is printed as FIRST_FRAME_MS below. Close the window to exit.\n"
    );

    Application::new().run(move |cx: &mut App| {
        // What this actually does at startup is one of the probe's questions.
        let init_start = Instant::now();
        gpui_component::init(cx);
        eprintln!(
            "COMPONENT_INIT_MS {:.1}",
            init_start.elapsed().as_secs_f64() * 1000.0
        );

        let bounds = Bounds::centered(None, size(px(980.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                ..Default::default()
            },
            |window, cx| {
                let probe = cx.new(|cx| Probe::new(window, cx, started));
                // Root is required for their overlays; harmless here, and worth exercising.
                // The turbofish is needed because Root::new takes an AnyView and inference
                // cannot pick the conversion on its own.
                let view: gpui::AnyView = probe.into();
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("a window");
        cx.activate(true);
    });
}
