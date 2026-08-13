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
    /// The prompt field's state. The whole probe hinges on how this feels.
    prompt: Entity<InputState>,
    /// Rows for the virtual list, shaped like our feed lines so the test is not a toy.
    rows: Vec<String>,
    /// How many observations arrived from **our** channel, proving cohabitation.
    observed: usize,
    /// Wall-clock from process start to first frame, printed on stderr.
    started: Instant,
    first_frame: Option<f64>,
}

impl Probe {
    fn new(window: &mut Window, cx: &mut Context<Self>, started: Instant) -> Self {
        let prompt = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("Type ten lines here. Try selection, click, wrap, paste…")
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

        Self {
            prompt,
            rows,
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
            // ★ 1. The point that decides. Give it room: ten lines must fit.
            .child(
                div()
                    .flex_none()
                    .p_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .h(px(220.0))
                            .child(gpui_component::input::Input::new(&self.prompt)),
                    ),
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
         1. The prompt field: type TEN LINES. Then try, in this order —\n\
            selection by mouse drag, click to place the cursor, a long line that must\n\
            soft-wrap, cmd-C / cmd-V, and an accented or IME character.\n\
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
