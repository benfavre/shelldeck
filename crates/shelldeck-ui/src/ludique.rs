//! The street of inklura.fr, drawn in the application.
//!
//! The public site opens on a small town ("Le quartier"): one building per
//! trade, a van that delivers, a sun to click. The same artwork gives the
//! welcome screen, the home banner and the empty states of Inklura Desktop
//! their playful side, so the application and the site read as one product.
//!
//! The artwork is exported by `scripts/export-ludique-art.py`. The scene owns
//! its day and its night (see [`StreetColors`]): it follows the active theme
//! until the sun or the moon is clicked. Everything that moves runs on the
//! shared motion clock and stands still when the platform asks for reduced
//! motion.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;

use crate::motion::repeating_phase;
use crate::scale::px;
use crate::t;
use crate::theme::{ShellDeckColors, StreetColors};

/// Every image the scene draws. Each one must be embedded by the binary
/// (`Assets::load` and `Assets::list` in `shelldeck/src/main.rs`): the test
/// below reads this list, so add a new image here first.
#[cfg(test)]
const STREET_ASSETS: &[&str] = &[
    "images/ludique/town-day.svg",
    "images/ludique/town-night.svg",
    "images/ludique/road-day.svg",
    "images/ludique/road-night.svg",
    "images/ludique/sun-a.svg",
    "images/ludique/sun-b.svg",
    "images/ludique/moon.svg",
    "images/ludique/stars.svg",
    "images/ludique/cloud-day.svg",
    "images/ludique/cloud-night.svg",
    "images/ludique/balloon.svg",
    "images/ludique/van.svg",
    "images/ludique/bird.svg",
    "images/ludique/parcel.svg",
    "images/ludique/pin.svg",
    "images/ludique/badge-account-day.svg",
    "images/ludique/badge-account-night.svg",
    "images/ludique/badge-shop-day.svg",
    "images/ludique/badge-shop-night.svg",
    "images/ludique/badge-laptop-day.svg",
    "images/ludique/badge-laptop-night.svg",
    "images/ludique/badge-shield-day.svg",
    "images/ludique/badge-shield-night.svg",
];

/// Width and height of the town strip in its own units; the road strip is as
/// wide and `ROAD_UNITS` high. Both repeat without a seam.
const TOWN_UNITS: (f32, f32) = (2400.0, 300.0);
const ROAD_UNITS: f32 = 76.0;
/// Share of the road strip taken by the pavement, at its top.
const PAVEMENT_SHARE: f32 = 20.0 / 76.0;

const HONK: Duration = Duration::from_millis(1_500);
const BALLOON_AWAY: Duration = Duration::from_millis(9_000);

/// Where the scene is drawn. The layout fixes the size of the town and what
/// surrounds it; the scene fills whatever box it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreetLayout {
    /// Full window: the whole sky, the town across the bottom.
    Welcome,
    /// Same scene for a window too short for the large town.
    WelcomeCompact,
    /// The head of a page: a short banner with a title on the sky to the
    /// left, and most of its width for a small town on its stretch of road,
    /// which fades into the sky towards the copy.
    PageHead,
}

impl StreetLayout {
    /// Logical pixels per artwork unit.
    fn scale(self) -> f32 {
        match self {
            Self::Welcome => 0.62,
            Self::WelcomeCompact | Self::PageHead => 0.40,
        }
    }

    /// Height of the pavement and the road under the town.
    pub(crate) fn road_height(self) -> f32 {
        ROAD_UNITS * self.scale()
    }

    /// Height of the town above the road, back row included.
    pub(crate) fn town_height(self) -> f32 {
        TOWN_UNITS.1 * self.scale()
    }

    /// Share of the width kept for the copy, from the left edge. The town
    /// starts there and fades in; full-window layouts keep none.
    pub(crate) fn copy_share(self) -> f32 {
        match self {
            Self::PageHead => 0.38,
            Self::Welcome | Self::WelcomeCompact => 0.0,
        }
    }

    /// Full-window layouts carry the balloon, the larger sky and the hint.
    fn is_large(self) -> bool {
        matches!(self, Self::Welcome | Self::WelcomeCompact)
    }

    /// Time the van takes to cross the scene.
    fn van_lap(self) -> Duration {
        Duration::from_secs(if self.is_large() { 24 } else { 22 })
    }

    /// How long the scene plays before it rests: one crossing of the van in a
    /// banner, which stays on screen all day, five on the welcome screen.
    fn run(self) -> Duration {
        self.van_lap() * if self.is_large() { 5 } else { 1 }
    }
}

/// Time of the scene. It runs while the scene plays and stands still while it
/// rests, so nothing jumps when the scene wakes up again.
///
/// A scene that repaints for ever keeps a processor busy for as long as the
/// window is open. The street plays for a set time, stops when the van has
/// left the picture, and plays again when it is shown again or when the
/// pointer comes over it.
#[derive(Default)]
struct StageClock {
    /// Scene time played before the current run.
    played: Duration,
    /// Start of the current run; `None` while the scene rests.
    running_since: Option<Instant>,
    /// When, and in which frame of its window, the scene was last drawn.
    last_drawn: Option<(Instant, u64)>,
}

impl StageClock {
    fn wake(&mut self, now: Instant) {
        if self.running_since.is_none() {
            self.running_since = Some(now);
        }
    }

    /// Scene time at `now`, and whether the scene is still playing. `frame`
    /// counts the frames of the window (see [`begin_frame`]): a scene that
    /// missed one was not on screen. A run ends on the first multiple of
    /// `run` it reaches: the van laps divide `run`, so the scene always rests
    /// with the van out of the picture.
    fn tick(&mut self, now: Instant, frame: u64, run: Duration) -> (Duration, bool) {
        let shown_again = self.last_drawn.is_none_or(|(_, last)| frame > last + 1);
        if shown_again {
            // Time spent out of sight does not count: a run cut short by
            // leaving the screen goes on from where it was last seen, and a
            // resting scene plays again.
            if let (Some(since), Some((last, _))) = (self.running_since, self.last_drawn) {
                self.played += last.saturating_duration_since(since);
            }
            self.running_since = Some(now);
        }
        self.last_drawn = Some((now, frame));
        let Some(since) = self.running_since else {
            return (self.played, false);
        };
        let time = self.played + now.saturating_duration_since(since);
        let end = rest_point(self.played, run);
        if time >= end {
            self.played = end;
            self.running_since = None;
            return (end, false);
        }
        (time, true)
    }
}

/// The scene time at which a run that starts at `played` comes to rest: the
/// next multiple of `run`.
fn rest_point(played: Duration, run: Duration) -> Duration {
    let run_ms = run.as_millis().max(1);
    let laps = played.as_millis() / run_ms + 1;
    Duration::from_millis((laps * run_ms) as u64)
}

/// What the visitor did to the street, and the clocks of its scenes. One
/// state for the application: day or night is the same on every screen.
#[derive(Default)]
struct StreetState {
    /// Day or night chosen by a click on the sun or the moon.
    night: Option<bool>,
    honked_at: Option<Instant>,
    balloon_left_at: Option<Instant>,
    /// One clock per scene and per vignette, by its id: each plays again when
    /// its own screen comes back.
    clocks: HashMap<&'static str, StageClock>,
    /// Frames drawn by the window that hosts the scene.
    frame: u64,
}

impl StreetState {
    fn clock(&mut self, id: &'static str) -> &mut StageClock {
        self.clocks.entry(id).or_default()
    }
}

impl Global for StreetState {}

/// Count one frame of the window that hosts the scene. Call it once at the
/// start of that window's render: a scene absent from a frame was hidden, and
/// plays again when it comes back. Without the call the scene still plays
/// once and wakes under the pointer.
pub(crate) fn begin_frame(cx: &mut App) {
    let state = cx.default_global::<StreetState>();
    state.frame = state.frame.wrapping_add(1);
}

/// Night in the scene: the visitor's choice, else the active theme.
pub(crate) fn street_is_night(cx: &App) -> bool {
    cx.try_global::<StreetState>()
        .and_then(|state| state.night)
        .unwrap_or_else(ShellDeckColors::is_dark)
}

/// Vertical offset of the balloon after a click, in artwork units, or `None`
/// while it is out of sight: it climbs away for three seconds, stays away,
/// then comes back down during the last two.
fn balloon_flight(elapsed: Duration, away: Duration) -> Option<f32> {
    const CLIMB: f32 = 3.0;
    const RETURN: f32 = 2.0;
    const HEIGHT: f32 = 520.0;
    let t = elapsed.as_secs_f32();
    let total = away.as_secs_f32();
    if t >= total {
        return Some(0.0);
    }
    if t < CLIMB {
        let p = t / CLIMB;
        return Some(-HEIGHT * p * p);
    }
    if t > total - RETURN {
        let p = (total - t) / RETURN;
        return Some(-HEIGHT * p * p);
    }
    None
}

/// Position of something that crosses the scene once per period, as a share
/// of the width: it enters from outside the left edge and leaves on the right.
fn crossing(phase: f32, margin: f32) -> f32 {
    -margin + phase.clamp(0.0, 1.0) * (1.0 + 2.0 * margin)
}

/// Share of a period reached at scene time `seconds`.
fn phase_of(seconds: f32, period: f32) -> f32 {
    (seconds / period).fract()
}

fn sprite(path: &'static str, width: f32, height: f32) -> Img {
    img(path)
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .object_fit(ObjectFit::Contain)
}

/// Three copies of a repeating strip, centred: enough for a window three
/// strips wide, and a narrower box shows the middle of the street.
fn strip_row(path: &'static str, tile_width: f32, height: f32) -> Div {
    let mut row = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(height))
        .flex()
        .justify_center()
        .overflow_hidden();
    for _ in 0..3 {
        row = row.child(
            img(path)
                .w(px(tile_width))
                .h(px(height))
                .flex_shrink_0()
                .object_fit(ObjectFit::Fill),
        );
    }
    row
}

/// The whole scene, as an absolute layer that fills its (relative) parent.
/// Put the copy after it, in a sibling, so it is drawn above the sky.
pub(crate) fn street_scene<V: 'static>(
    id: &'static str,
    layout: StreetLayout,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let night = street_is_night(cx);
    let still = cx.prefers_reduced_motion();
    let now = Instant::now();
    let scale = layout.scale();
    let unit = |value: f32| value * scale;
    let road_h = layout.road_height();
    let town_h = layout.town_height();
    let tile_w = unit(TOWN_UNITS.0);
    let large = layout.is_large();
    let copy = layout.copy_share();

    let (seconds, playing, honk_age, balloon_offset) = {
        let state = cx.default_global::<StreetState>();
        let (time, playing) = if still {
            (Duration::ZERO, false)
        } else {
            let frame = state.frame;
            state.clock(id).tick(now, frame, layout.run())
        };
        let honk_age = state
            .honked_at
            .map(|at| now.saturating_duration_since(at))
            .filter(|age| *age < HONK);
        let balloon = match state.balloon_left_at {
            Some(at) => balloon_flight(now.saturating_duration_since(at), BALLOON_AWAY),
            None => Some(0.0),
        };
        (time.as_secs_f32(), playing, honk_age, balloon)
    };
    if playing || honk_age.is_some() {
        // Lease the shared clock: it repaints this view while the scene plays.
        let _ = repeating_phase(Duration::from_secs(1), cx);
    }

    let bob = (phase_of(seconds, 5.2) * std::f32::consts::TAU).sin();
    // A full window warms towards the horizon. A banner keeps one sky colour:
    // its town fades into the sky sideways, which only one colour can hide.
    let horizon = if large {
        StreetColors::sky_bottom(night)
    } else {
        StreetColors::sky_top(night)
    };
    let mut scene = div()
        .id(SharedString::from(format!("{id}-street")))
        .absolute()
        .inset_0()
        .overflow_hidden()
        .bg(linear_gradient(
            180.0,
            linear_color_stop(StreetColors::sky_top(night), 0.0),
            linear_color_stop(horizon, 1.0),
        ));
    if !still {
        // A resting scene plays again when the pointer comes over it.
        scene = scene.on_hover(move |hovered, _, cx| {
            if *hovered {
                let now = Instant::now();
                cx.default_global::<StreetState>().clock(id).wake(now);
                cx.refresh_windows();
            }
        });
    }

    if night {
        let mut stars = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(if large { 300.0 } else { 110.0 }))
            .flex()
            .justify_center()
            .overflow_hidden();
        for _ in 0..4 {
            stars = stars.child(
                img("images/ludique/stars.svg")
                    .w(px(if large { 800.0 } else { 293.0 }))
                    .h_full()
                    .flex_shrink_0()
                    .object_fit(ObjectFit::Fill),
            );
        }
        scene = scene.child(stars);
    }

    // Sun or moon. A click turns the day into night and back. In a banner it
    // hangs over the first roofs of the town, drawn after the town so that
    // the fade of the town does not wash it out.
    // The sky of a short window is furnished at the size of its town.
    let sky_scale = if large {
        scale / StreetLayout::Welcome.scale()
    } else {
        1.0
    };
    // The head of a page is too low for an orb above its roofs: a smaller
    // one sits where the town fades out, before the first building.
    let page_head = layout == StreetLayout::PageHead;
    let orb_size = if large {
        86.0 * sky_scale
    } else if page_head {
        36.0
    } else {
        46.0
    };
    let orb_top = if large {
        26.0
    } else if page_head {
        7.0
    } else {
        10.0
    };
    let orb_right = 0.07;
    let orb_path = if night {
        "images/ludique/moon.svg"
    } else if phase_of(seconds, 1.6) < 0.5 {
        "images/ludique/sun-a.svg"
    } else {
        "images/ludique/sun-b.svg"
    };
    let orb = div()
        .id(SharedString::from(format!("{id}-orb")))
        .absolute()
        .top(px(orb_top + if large { bob * 2.0 } else { 0.0 }))
        .cursor_pointer()
        .child(sprite(orb_path, orb_size, orb_size))
        .on_click(move |_, _, cx| {
            let next = !street_is_night(cx);
            let state = cx.default_global::<StreetState>();
            state.night = Some(next);
            state.clock(id).wake(Instant::now());
            cx.refresh_windows();
        });
    let mut banner_orb = None;
    if large {
        scene = scene.child(orb.right(relative(orb_right)));
    } else {
        banner_orb = Some(orb.left(relative(copy + if page_head { 0.015 } else { 0.075 })));
    }
    let untouched = cx
        .try_global::<StreetState>()
        .is_none_or(|state| state.night.is_none());
    if layout == StreetLayout::Welcome && untouched {
        scene = scene.child(
            div()
                .absolute()
                .top(px(orb_top + orb_size + 6.0))
                .right(relative(orb_right))
                .w(px(orb_size))
                .flex()
                .justify_center()
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(StreetColors::ink(night).opacity(0.72))
                        .child(if night {
                            t!("ludique.orb_hint_night").to_string()
                        } else {
                            t!("ludique.orb_hint_day").to_string()
                        }),
                ),
        );
    }

    // Clouds. Across a full window they drift from one side to the other; in
    // a banner they stay above the town and only sway, clear of the copy.
    let cloud_path = if night {
        "images/ludique/cloud-night.svg"
    } else {
        "images/ludique/cloud-day.svg"
    };
    let cloud_h = |width: f32| width * 52.0 / 98.0;
    if large {
        // (start, seconds per crossing, top as a share of the height, width)
        for (start, lap, top, width) in [
            (0.12, 120.0, 0.07, 150.0),
            (0.58, 75.0, 0.19, 104.0),
            (0.86, 150.0, 0.31, 122.0),
        ] {
            let x = crossing((start + phase_of(seconds, lap)).fract(), 0.14);
            let width = width * sky_scale;
            scene = scene.child(
                sprite(cloud_path, width, cloud_h(width))
                    .absolute()
                    .top(relative(top))
                    .left(relative(x)),
            );
        }
    } else {
        let sway = (phase_of(seconds, 12.0) * std::f32::consts::TAU).sin() * 0.012;
        for (place, top, width, way) in [(0.36, 8.0, 72.0, 1.0), (0.80, 24.0, 54.0, -1.0)] {
            let x = copy + (1.0 - copy) * place + sway * way;
            scene = scene.child(
                sprite(cloud_path, width, cloud_h(width))
                    .absolute()
                    .top(px(top))
                    .left(relative(x)),
            );
        }
    }

    // By day a bird crosses the sky of a full window, then is away for a while.
    if large && !night && playing {
        let lap = phase_of(seconds + 9.0, 40.0);
        if lap < 0.6 {
            let flap = phase_of(seconds, 0.9) < 0.5;
            scene = scene.child(
                sprite("images/ludique/bird.svg", 30.0, 15.0)
                    .absolute()
                    .top(relative(if flap { 0.115 } else { 0.125 }))
                    .left(relative(crossing(lap / 0.6, 0.06))),
            );
        }
    }

    // The balloon floats on the left; a click sends it off for a while.
    if large {
        if let Some(offset) = balloon_offset {
            let (w, h) = (unit(90.0), unit(126.0));
            let mut balloon = div()
                .id(SharedString::from(format!("{id}-balloon")))
                .absolute()
                .top(px(unit(96.0) + bob * 5.0 + unit(offset)))
                .left(relative(0.075))
                .child(sprite("images/ludique/balloon.svg", w, h));
            if !still {
                balloon = balloon.cursor_pointer().on_click(move |_, _, cx| {
                    let now = Instant::now();
                    let state = cx.default_global::<StreetState>();
                    let flying = state
                        .balloon_left_at
                        .is_some_and(|at| now.saturating_duration_since(at) < BALLOON_AWAY);
                    if !flying {
                        state.balloon_left_at = Some(now);
                    }
                    state.clock(id).wake(now);
                    cx.refresh_windows();
                });
            }
            scene = scene.child(balloon);
        }
    }

    // The town, the road under it, and the van on the road.
    let town_path = if night {
        "images/ludique/town-night.svg"
    } else {
        "images/ludique/town-day.svg"
    };
    let road_path = if night {
        "images/ludique/road-night.svg"
    } else {
        "images/ludique/road-day.svg"
    };
    let town = strip_row(town_path, tile_w, town_h);
    let road = strip_row(road_path, tile_w, road_h).bottom_0();

    // One crossing of the road per lap. Without motion the van is parked in
    // front of the town.
    let (van_w, van_h) = (unit(150.0), unit(90.0));
    let van_x = if still {
        0.22
    } else {
        crossing(phase_of(seconds, layout.van_lap().as_secs_f32()), 0.16)
    };
    // One short jump at the start of a honk.
    let hop = honk_age.map_or(0.0, |age| {
        let p = (age.as_secs_f32() / 0.45).min(1.0);
        (1.0 - (p * 2.0 - 1.0).powi(2)) * unit(26.0)
    });
    let wheel_line = road_h * (1.0 - PAVEMENT_SHARE) * 0.30;
    let mut van = div()
        .id(SharedString::from(format!("{id}-van")))
        .absolute()
        .bottom(px(wheel_line + hop))
        .left(relative(van_x))
        .child(sprite("images/ludique/van.svg", van_w, van_h));
    if !still {
        van = van.cursor_pointer().on_click(move |_, _, cx| {
            cx.default_global::<StreetState>().honked_at = Some(Instant::now());
            cx.refresh_windows();
        });
    }
    if honk_age.is_some() {
        van = van.child(
            div()
                .absolute()
                .bottom(px(van_h + 4.0))
                .left(px(van_w * 0.2))
                .px(px(if large { 10.0 } else { 7.0 }))
                .py(px(if large { 5.0 } else { 3.0 }))
                .rounded_full()
                .bg(StreetColors::paper())
                .border_2()
                .border_color(StreetColors::line())
                .whitespace_nowrap()
                .text_size(px(if large { 12.0 } else { 10.0 }))
                .font_weight(FontWeight::BOLD)
                .text_color(StreetColors::line())
                .child(t!("ludique.honk").to_string()),
        );
    }

    if copy > 0.0 {
        // The banner keeps its left side for the copy. The street lives in a
        // box on the right: town, road and van are cut by it, and a fade to
        // the sky colour covers the cut, so the road comes out of the sky and
        // the van drives out of it.
        let sky = horizon;
        scene = scene.child(
            div()
                .absolute()
                .bottom_0()
                .right_0()
                .w(relative(1.0 - copy))
                .h(px(town_h + road_h))
                .overflow_hidden()
                .child(town.top_0())
                .child(road)
                .child(van)
                .child(
                    // Fully sky over the cut, then thinning out: the first
                    // stretch must hide the edge of whatever it falls on.
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(px(150.0))
                        .bg(multi_stop_linear_gradient(
                            90.0,
                            &[
                                linear_color_stop(sky, 0.0),
                                linear_color_stop(sky, 0.22),
                                linear_color_stop(sky.opacity(0.0), 1.0),
                            ],
                        )),
                ),
        );
        if let Some(orb) = banner_orb {
            scene = scene.child(orb);
        }
        scene
    } else {
        scene.child(town.bottom(px(road_h))).child(road).child(van)
    }
}

/// The site's label: a white pill with a hard shadow, set above a title.
pub(crate) fn sticker(label: impl Into<SharedString>) -> Div {
    div()
        .px(px(13.0))
        .py(px(6.0))
        .rounded_full()
        .bg(StreetColors::paper())
        .shadow(
            vec![BoxShadow {
                color: StreetColors::line().opacity(0.16),
                offset: point(gpui::px(0.0), gpui::px(3.0)),
                blur_radius: gpui::px(0.0),
                spread_radius: gpui::px(0.0),
                inset: false,
            }]
            .into(),
        )
        .whitespace_nowrap()
        .text_size(px(12.0))
        .font_weight(FontWeight::BOLD)
        .text_color(StreetColors::line())
        .child(label.into())
}

/// A word under the marker pen: the title of a scene, with the site's
/// highlighter across its lower half.
pub(crate) fn highlighted(label: impl Into<SharedString>, text_size: f32, night: bool) -> Div {
    div()
        .relative()
        .px(px(text_size * 0.12))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(text_size * 0.10))
                .h(px(text_size * 0.42))
                .bg(StreetColors::marker(night)),
        )
        .child(
            div()
                .relative()
                .text_size(px(text_size))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(StreetColors::ink(night))
                .child(label.into()),
        )
}

/// Which of the site's two buttons to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkyKind {
    /// Blue, for the one action the screen exists for.
    Primary,
    /// White with an ink outline, for the alternative.
    Paper,
}

/// The site's button: a pill standing on a hard edge that it presses into.
/// The caller adds the icon, the label and the click handler.
pub(crate) fn chunky_button(id: impl Into<ElementId>, kind: ChunkyKind) -> Stateful<Div> {
    let (fill, edge, text) = match kind {
        ChunkyKind::Primary => (
            StreetColors::button(),
            StreetColors::button_edge(),
            StreetColors::paper(),
        ),
        ChunkyKind::Paper => (
            StreetColors::paper(),
            StreetColors::line(),
            StreetColors::line(),
        ),
    };
    let edge_shadow = move |depth: f32| -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: edge,
            offset: point(gpui::px(0.0), gpui::px(depth)),
            blur_radius: gpui::px(0.0),
            spread_radius: gpui::px(0.0),
            inset: false,
        }]
    };
    let mut button = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .rounded_full()
        .bg(fill)
        .text_color(text)
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
        .shadow(edge_shadow(4.0).into())
        .hover(move |style| style.shadow(edge_shadow(6.0).into()))
        .active(move |style| style.shadow(edge_shadow(1.0).into()));
    if kind == ChunkyKind::Paper {
        button = button.border_2().border_color(StreetColors::line());
    }
    button
}

/// The drawing that opens a section of a page: a piece of the street with
/// three strokes of the marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SectionBadge {
    /// A client record: the account.
    Account,
    /// A shop of the street: the organisation and its sites.
    Shop,
    /// A laptop: this device and its session.
    Laptop,
    /// A shield with a tick: what the account may do.
    Shield,
}

/// Draw a section badge. It stands on a panel of the theme, not on the sky:
/// a dark theme gets the variant drawn with a light line.
pub(crate) fn section_badge(badge: SectionBadge, height: f32) -> Img {
    let dark = ShellDeckColors::is_dark();
    let path = match (badge, dark) {
        (SectionBadge::Account, false) => "images/ludique/badge-account-day.svg",
        (SectionBadge::Account, true) => "images/ludique/badge-account-night.svg",
        (SectionBadge::Shop, false) => "images/ludique/badge-shop-day.svg",
        (SectionBadge::Shop, true) => "images/ludique/badge-shop-night.svg",
        (SectionBadge::Laptop, false) => "images/ludique/badge-laptop-day.svg",
        (SectionBadge::Laptop, true) => "images/ludique/badge-laptop-night.svg",
        (SectionBadge::Shield, false) => "images/ludique/badge-shield-day.svg",
        (SectionBadge::Shield, true) => "images/ludique/badge-shield-night.svg",
    };
    sprite(path, height * 1.12, height)
}

/// A small sky with one piece of the street in it, for a list that is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Vignette {
    /// The delivery van and a client record: nothing has arrived yet.
    Delivery,
    /// The balloon: nothing in sight.
    Balloon,
    /// A map pin: no place to show.
    Pin,
}

/// Corner radius of a vignette. A band of ground at its bottom repeats it:
/// clipping to the patch is rectangular and would leave square corners.
const VIGNETTE_RADIUS: f32 = 16.0;

/// Draw a vignette: a patch of sky, a cloud, and the subject bobbing. Like
/// the street it moves for a short while, then rests until the pointer comes.
pub(crate) fn vignette<V: 'static>(
    id: &'static str,
    kind: Vignette,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    /// Three bobs of the subject, so the vignette rests in its neutral pose.
    const RUN: Duration = Duration::from_millis(12_600);
    let night = street_is_night(cx);
    let still = cx.prefers_reduced_motion();
    let (seconds, playing) = if still {
        (0.0, false)
    } else {
        let now = Instant::now();
        let state = cx.default_global::<StreetState>();
        let frame = state.frame;
        let (time, playing) = state.clock(id).tick(now, frame, RUN);
        (time.as_secs_f32(), playing)
    };
    if playing {
        let _ = repeating_phase(Duration::from_secs(1), cx);
    }
    let bob = (phase_of(seconds, 4.2) * std::f32::consts::TAU).sin();
    let cloud = if night {
        "images/ludique/cloud-night.svg"
    } else {
        "images/ludique/cloud-day.svg"
    };
    let mut patch = div()
        .id(SharedString::from(format!("{id}-vignette")))
        .relative()
        .flex_shrink_0()
        .w(px(132.0))
        .h(px(104.0))
        .rounded(px(VIGNETTE_RADIUS))
        .overflow_hidden()
        .bg(linear_gradient(
            180.0,
            linear_color_stop(StreetColors::sky_top(night), 0.0),
            linear_color_stop(StreetColors::sky_bottom(night), 1.0),
        ))
        .child(
            sprite(cloud, 46.0, 24.4)
                .absolute()
                .top(px(12.0))
                .left(px(12.0 + bob * 3.0)),
        );
    if !still {
        patch = patch.on_hover(move |hovered, _, cx| {
            if *hovered {
                cx.default_global::<StreetState>()
                    .clock(id)
                    .wake(Instant::now());
                cx.refresh_windows();
            }
        });
    }
    patch = match kind {
        Vignette::Delivery => patch
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(18.0))
                    .rounded_b(px(VIGNETTE_RADIUS))
                    .bg(StreetColors::road()),
            )
            .child(
                sprite("images/ludique/parcel.svg", 30.0, 30.0)
                    .absolute()
                    .top(px(20.0 - bob * 3.0))
                    .right(px(16.0)),
            )
            .child(
                sprite("images/ludique/van.svg", 70.0, 42.0)
                    .absolute()
                    .bottom(px(5.0))
                    .left(px(24.0 + bob * 2.0)),
            ),
        Vignette::Balloon => patch.child(
            sprite("images/ludique/balloon.svg", 46.0, 64.0)
                .absolute()
                .top(px(22.0 + bob * 4.0))
                .left(px(46.0)),
        ),
        Vignette::Pin => patch
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(18.0))
                    .rounded_b(px(VIGNETTE_RADIUS))
                    .border_t_2()
                    .border_color(StreetColors::line())
                    .bg(StreetColors::pavement(night)),
            )
            .child(
                sprite("images/ludique/pin.svg", 34.0, 45.0)
                    .absolute()
                    .bottom(px(14.0 + (bob + 1.0) * 3.0))
                    .left(px(49.0)),
            ),
    };
    patch
}

#[cfg(test)]
mod tests {
    use super::{
        balloon_flight, crossing, rest_point, StageClock, StreetLayout, BALLOON_AWAY, STREET_ASSETS,
    };
    use std::time::{Duration, Instant};

    /// SDTEST-3177 / SDUC-587.
    ///
    /// An image that is not registered in the binary fails to load without an
    /// error: the scene would show an empty sky and nothing would say why.
    #[test]
    fn sdtest_3177_every_street_image_is_embedded_and_listed() {
        let main_rs = include_str!("../../shelldeck/src/main.rs");
        for asset in STREET_ASSETS {
            assert!(
                main_rs.contains(&format!("\"{asset}\" =>")),
                "{asset} is not embedded in main.rs (Assets::load)",
            );
            assert!(
                main_rs.contains(&format!("SharedString::from(\"{asset}\")")),
                "{asset} is missing from Assets::list",
            );
            let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../shelldeck/assets")
                .join(asset);
            assert!(file.is_file(), "{asset} was not exported");
        }
    }

    /// SDTEST-3178 / SDUC-587.
    ///
    /// The balloon leaves the screen, stays away, and is back in place when the
    /// flight ends: a click must never lose it for good.
    #[test]
    fn sdtest_3178_the_balloon_leaves_and_always_comes_back() {
        assert_eq!(balloon_flight(Duration::ZERO, BALLOON_AWAY), Some(0.0));
        let climbing = balloon_flight(Duration::from_millis(1_500), BALLOON_AWAY).unwrap();
        assert!(climbing < 0.0 && climbing > -520.0);
        assert_eq!(balloon_flight(Duration::from_secs(5), BALLOON_AWAY), None);
        let returning = balloon_flight(Duration::from_millis(8_500), BALLOON_AWAY).unwrap();
        assert!(returning < 0.0 && returning > -520.0);
        assert_eq!(balloon_flight(BALLOON_AWAY, BALLOON_AWAY), Some(0.0));
        assert_eq!(
            balloon_flight(Duration::from_secs(60), BALLOON_AWAY),
            Some(0.0)
        );
    }

    /// SDTEST-3179 / SDUC-587.
    ///
    /// What crosses the scene starts and ends out of sight, so the loop has no
    /// visible jump, and the banner keeps room for its copy.
    #[test]
    fn sdtest_3179_crossings_start_and_end_out_of_sight() {
        assert!(crossing(0.0, 0.16) < -0.15);
        assert!(crossing(1.0, 0.16) > 1.15);
        assert!((crossing(0.5, 0.16) - 0.5).abs() < 1e-6);
        assert!(crossing(7.0, 0.16) <= 1.16 + 1e-6);
        let share = StreetLayout::PageHead.copy_share();
        assert!(
            (0.3..=0.7).contains(&share),
            "the page head must leave room for both the copy and the town"
        );
        assert_eq!(StreetLayout::Welcome.copy_share(), 0.0);
        assert!(StreetLayout::PageHead.town_height() < StreetLayout::Welcome.town_height());
    }

    /// SDTEST-3180 / SDUC-587.
    ///
    /// The scene must not repaint for ever: it plays its run, rests exactly
    /// when the van has left the picture, keeps its time while it rests, and
    /// plays again when it is shown again or woken.
    #[test]
    fn sdtest_3180_the_scene_plays_its_run_then_rests_until_woken() {
        let run = StreetLayout::PageHead.run();
        assert_eq!(run, StreetLayout::PageHead.van_lap());
        assert_eq!(
            StreetLayout::Welcome.run().as_millis() % StreetLayout::Welcome.van_lap().as_millis(),
            0,
            "a run that is not whole laps would rest with the van in view",
        );
        assert_eq!(rest_point(Duration::ZERO, run), run);
        assert_eq!(rest_point(run, run), run * 2);
        assert_eq!(rest_point(run + Duration::from_secs(3), run), run * 2);

        let start = Instant::now();
        let mut clock = StageClock::default();
        let mut frame = 1;
        // First drawing: the scene starts on its own.
        assert_eq!(clock.tick(start, frame, run), (Duration::ZERO, true));
        // Drawn in every frame, once a second, it reaches the end of the run
        // and rests there.
        let mut now = start;
        let mut last = (Duration::ZERO, true);
        while last.1 {
            now += Duration::from_secs(1);
            frame += 1;
            last = clock.tick(now, frame, run);
        }
        assert_eq!(last, (run, false));
        // At rest nothing asks for a frame. The next one may come a minute
        // later, for another part of the window: the scene stays at rest.
        now += Duration::from_secs(60);
        frame += 1;
        assert_eq!(clock.tick(now, frame, run), (run, false));
        // Woken by the pointer: it goes on from where it stopped.
        clock.wake(now);
        now += Duration::from_secs(1);
        frame += 1;
        assert_eq!(
            clock.tick(now, frame, run),
            (run + Duration::from_secs(1), true)
        );
        // Another screen is drawn for a minute (frames without the scene),
        // then the scene is back: the minute does not count, the run goes on.
        now += Duration::from_secs(60);
        frame += 40;
        assert_eq!(
            clock.tick(now, frame, run),
            (run + Duration::from_secs(1), true)
        );
        // Played to its rest, hidden, shown again: a new run starts.
        while clock.tick(now, frame, run).1 {
            now += Duration::from_secs(1);
            frame += 1;
        }
        now += Duration::from_secs(60);
        frame += 40;
        assert_eq!(clock.tick(now, frame, run), (run * 2, true));
    }
}
