//! Native screenshot annotation surface shown after an interactive area
//! capture. Annotations are previewed with GPUI paths and baked into a PNG
//! before the draft enters the request composer.

use crate::icons::lucide_icon;
use crate::issue_attachments::AttachmentDraft;
use crate::overlay::window_backdrop;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::icon_source::IconSource;
use gpui::prelude::*;
use gpui::*;
use image::{DynamicImage, ImageFormat as RasterFormat, Rgba, RgbaImage};
use parking_lot::Mutex;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::Arc;

type ApplyCallback = Rc<dyn Fn(AttachmentDraft, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationTool {
    Arrow,
    Rectangle,
    Pen,
    Marker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationColor {
    Red,
    Yellow,
    Blue,
    Green,
}

impl AnnotationColor {
    const ALL: [Self; 4] = [Self::Red, Self::Yellow, Self::Blue, Self::Green];

    fn ui(self) -> Hsla {
        let rgba = match self {
            Self::Red => gpui::rgba(0xef4444ff),
            Self::Yellow => gpui::rgba(0xfbbf24ff),
            Self::Blue => gpui::rgba(0x3b82f6ff),
            Self::Green => gpui::rgba(0x22c55eff),
        };
        rgba.into()
    }

    fn raster(self) -> Rgba<u8> {
        match self {
            Self::Red => Rgba([239, 68, 68, 255]),
            Self::Yellow => Rgba([251, 191, 36, 255]),
            Self::Blue => Rgba([59, 130, 246, 255]),
            Self::Green => Rgba([34, 197, 94, 255]),
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Green => "green",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NormPoint {
    x: f32,
    y: f32,
}

impl NormPoint {
    fn distance(self, other: Self) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

#[derive(Clone, Debug)]
enum Annotation {
    Arrow {
        start: NormPoint,
        end: NormPoint,
        color: AnnotationColor,
        width: f32,
    },
    Rectangle {
        start: NormPoint,
        end: NormPoint,
        color: AnnotationColor,
        width: f32,
    },
    Pen {
        points: Vec<NormPoint>,
        color: AnnotationColor,
        width: f32,
    },
    Marker {
        point: NormPoint,
        number: u8,
        color: AnnotationColor,
    },
}

#[derive(Clone, Copy, Debug, Default)]
struct CanvasGeometry {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl CanvasGeometry {
    fn normalized(self, point: Point<Pixels>) -> Option<NormPoint> {
        if self.width <= 0.0 || self.height <= 0.0 {
            return None;
        }
        let x = point.x.to_f64() as f32;
        let y = point.y.to_f64() as f32;
        if x < self.x || y < self.y || x > self.x + self.width || y > self.y + self.height {
            return None;
        }
        Some(NormPoint {
            x: ((x - self.x) / self.width).clamp(0.0, 1.0),
            y: ((y - self.y) / self.height).clamp(0.0, 1.0),
        })
    }

    fn screen(self, point: NormPoint) -> Point<Pixels> {
        gpui::point(
            gpui::px(self.x + point.x * self.width),
            gpui::px(self.y + point.y * self.height),
        )
    }

    fn min_side(self) -> f32 {
        self.width.min(self.height)
    }
}

pub struct AttachmentAnnotator {
    draft: AttachmentDraft,
    image_width: u32,
    image_height: u32,
    tool: AnnotationTool,
    color: AnnotationColor,
    stroke_ratio: f32,
    annotations: Vec<Annotation>,
    redo: Vec<Annotation>,
    drawing: Option<Annotation>,
    geometry: Arc<Mutex<CanvasGeometry>>,
    focus_handle: FocusHandle,
    focused: bool,
    error: Option<String>,
    on_cancel: Rc<dyn Fn(&mut App)>,
    on_apply: ApplyCallback,
}

impl AttachmentAnnotator {
    pub fn new(
        draft: AttachmentDraft,
        on_cancel: impl Fn(&mut App) + 'static,
        on_apply: impl Fn(AttachmentDraft, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let dimensions = image::load_from_memory(draft.bytes.as_ref())
            .map(|image| (image.width(), image.height()))
            .unwrap_or((1, 1));
        Self {
            draft,
            image_width: dimensions.0.max(1),
            image_height: dimensions.1.max(1),
            tool: AnnotationTool::Arrow,
            color: AnnotationColor::Red,
            stroke_ratio: 0.007,
            annotations: Vec::new(),
            redo: Vec::new(),
            drawing: None,
            geometry: Arc::new(Mutex::new(CanvasGeometry::default())),
            focus_handle: cx.focus_handle(),
            focused: false,
            error: None,
            on_cancel: Rc::new(on_cancel),
            on_apply: Rc::new(on_apply),
        }
    }

    fn close(&self, cx: &mut App) {
        (self.on_cancel)(cx);
    }

    fn begin(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(point) = self.geometry.lock().normalized(position) else {
            return;
        };
        self.redo.clear();
        self.error = None;
        self.drawing = match self.tool {
            AnnotationTool::Arrow => Some(Annotation::Arrow {
                start: point,
                end: point,
                color: self.color,
                width: self.stroke_ratio,
            }),
            AnnotationTool::Rectangle => Some(Annotation::Rectangle {
                start: point,
                end: point,
                color: self.color,
                width: self.stroke_ratio,
            }),
            AnnotationTool::Pen => Some(Annotation::Pen {
                points: vec![point],
                color: self.color,
                width: self.stroke_ratio,
            }),
            AnnotationTool::Marker => {
                let number = (self
                    .annotations
                    .iter()
                    .filter(|annotation| matches!(annotation, Annotation::Marker { .. }))
                    .count()
                    % 9
                    + 1) as u8;
                self.annotations.push(Annotation::Marker {
                    point,
                    number,
                    color: self.color,
                });
                None
            }
        };
        cx.notify();
    }

    fn update_drawing(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(point) = self.geometry.lock().normalized(position) else {
            return;
        };
        match self.drawing.as_mut() {
            Some(Annotation::Arrow { end, .. }) | Some(Annotation::Rectangle { end, .. }) => {
                *end = point;
            }
            Some(Annotation::Pen { points, .. }) => {
                if points
                    .last()
                    .map(|last| last.distance(point) >= 0.002)
                    .unwrap_or(true)
                {
                    points.push(point);
                }
            }
            _ => return,
        }
        cx.notify();
    }

    fn finish_drawing(&mut self, cx: &mut Context<Self>) {
        let Some(annotation) = self.drawing.take() else {
            return;
        };
        let keep = match &annotation {
            Annotation::Arrow { start, end, .. } | Annotation::Rectangle { start, end, .. } => {
                start.distance(*end) >= 0.006
            }
            Annotation::Pen { points, .. } => points.len() >= 2,
            Annotation::Marker { .. } => true,
        };
        if keep {
            self.annotations.push(annotation);
        }
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(annotation) = self.annotations.pop() {
            self.redo.push(annotation);
            cx.notify();
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some(annotation) = self.redo.pop() {
            self.annotations.push(annotation);
            cx.notify();
        }
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.annotations.is_empty() {
            return;
        }
        self.redo.extend(self.annotations.drain(..).rev());
        self.drawing = None;
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        match annotated_draft(&self.draft, &self.annotations) {
            Ok(draft) => (self.on_apply)(draft, cx),
            Err(error) => {
                self.error =
                    Some(t!("user.requests.annotator.export_error", error = error).to_string());
                cx.notify();
            }
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let command = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;
        if key == "escape" {
            self.close(cx);
        } else if command && key.eq_ignore_ascii_case("z") {
            if event.keystroke.modifiers.shift {
                self.redo(cx);
            } else {
                self.undo(cx);
            }
        } else if command && key.eq_ignore_ascii_case("y") {
            self.redo(cx);
        }
    }

    fn tool_button(
        &self,
        tool: AnnotationTool,
        id: &'static str,
        icon: &'static str,
        label: String,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id, label)
            .size(ButtonSize::Sm)
            .variant(ButtonVariant::Outline)
            .selected(self.tool == tool)
            .icon(IconSource::from(icon))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.tool = tool;
                this.drawing = None;
                cx.notify();
            }))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut colors = div().flex().items_center().gap(px(5.0));
        for color in AnnotationColor::ALL {
            colors = colors.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "annotator-color-{}",
                        color.slug()
                    ))))
                    .size(px(24.0))
                    .p(px(3.0))
                    .rounded_full()
                    .border_2()
                    .border_color(if color == self.color {
                        ShellDeckColors::text_primary()
                    } else {
                        gpui::transparent_black()
                    })
                    .cursor_pointer()
                    .child(div().size_full().rounded_full().bg(color.ui()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.color = color;
                        cx.notify();
                    })),
            );
        }

        let thickness =
            |id: &'static str, label: String, ratio: f32, this: &Self, cx: &mut Context<Self>| {
                Button::new(id, label)
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Outline)
                    .selected((this.stroke_ratio - ratio).abs() < f32::EPSILON)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.stroke_ratio = ratio;
                        cx.notify();
                    }))
            };

        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .child(self.tool_button(
                AnnotationTool::Arrow,
                "annotator-arrow",
                "arrow-right",
                t!("user.requests.annotator.arrow").to_string(),
                cx,
            ))
            .child(self.tool_button(
                AnnotationTool::Rectangle,
                "annotator-rectangle",
                "square",
                t!("user.requests.annotator.rectangle").to_string(),
                cx,
            ))
            .child(self.tool_button(
                AnnotationTool::Pen,
                "annotator-pen",
                "pencil",
                t!("user.requests.annotator.pen").to_string(),
                cx,
            ))
            .child(self.tool_button(
                AnnotationTool::Marker,
                "annotator-marker",
                "pin",
                t!("user.requests.annotator.marker").to_string(),
                cx,
            ))
            .child(div().w(px(1.0)).h(px(24.0)).bg(ShellDeckColors::border()))
            .child(colors)
            .child(div().w(px(1.0)).h(px(24.0)).bg(ShellDeckColors::border()))
            .child(thickness(
                "annotator-thin",
                t!("user.requests.annotator.thin").to_string(),
                0.004,
                self,
                cx,
            ))
            .child(thickness(
                "annotator-medium",
                t!("user.requests.annotator.medium").to_string(),
                0.007,
                self,
                cx,
            ))
            .child(thickness(
                "annotator-thick",
                t!("user.requests.annotator.thick").to_string(),
                0.012,
                self,
                cx,
            ))
            .child(div().flex_1())
            .child(
                Button::new(
                    "annotator-undo",
                    t!("user.requests.annotator.undo").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .icon(IconSource::from("rotate-ccw"))
                .disabled(self.annotations.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
            )
            .child(
                Button::new(
                    "annotator-redo",
                    t!("user.requests.annotator.redo").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .icon(IconSource::from("refresh-cw"))
                .disabled(self.redo.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.redo(cx))),
            )
            .child(
                Button::new(
                    "annotator-clear",
                    t!("user.requests.annotator.clear").to_string(),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Ghost)
                .icon(IconSource::from("trash-2"))
                .disabled(self.annotations.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
            )
            .into_any_element()
    }

    fn render_canvas(&self, cx: &mut Context<Self>) -> AnyElement {
        let geometry = self.geometry.clone();
        let width = self.image_width;
        let height = self.image_height;
        let mut annotations = self.annotations.clone();
        if let Some(drawing) = self.drawing.clone() {
            annotations.push(drawing);
        }

        let entity = cx.entity().downgrade();
        let down_entity = entity.clone();
        let move_entity = entity.clone();
        let up_entity = entity;

        div()
            .id("attachment-annotator-canvas")
            .relative()
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .cursor_crosshair()
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    if let Some(entity) = down_entity.upgrade() {
                        entity.update(cx, |this, cx| {
                            this.focus_handle.focus(window);
                            this.begin(event.position, cx);
                        });
                    }
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    if let Some(entity) = move_entity.upgrade() {
                        entity.update(cx, |this, cx| this.update_drawing(event.position, cx));
                    }
                }
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                if let Some(entity) = up_entity.upgrade() {
                    entity.update(cx, |this, cx| this.finish_drawing(cx));
                }
            })
            .child(
                img(self.draft.image.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        let scale = (bounds.size.width.to_f64() as f32 / width as f32)
                            .min(bounds.size.height.to_f64() as f32 / height as f32);
                        let actual_width = width as f32 * scale;
                        let actual_height = height as f32 * scale;
                        let actual = CanvasGeometry {
                            x: bounds.origin.x.to_f64() as f32
                                + (bounds.size.width.to_f64() as f32 - actual_width) / 2.0,
                            y: bounds.origin.y.to_f64() as f32
                                + (bounds.size.height.to_f64() as f32 - actual_height) / 2.0,
                            width: actual_width,
                            height: actual_height,
                        };
                        *geometry.lock() = actual;
                        (actual, annotations)
                    },
                    move |_, (actual, annotations), window, _| {
                        for annotation in annotations {
                            paint_annotation(window, actual, &annotation);
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .into_any_element()
    }
}

impl Render for AttachmentAnnotator {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            window.focus(&self.focus_handle);
            self.focused = true;
        }

        let close_entity = cx.entity();
        let apply_entity = close_entity.clone();

        window_backdrop("attachment-annotator", window.is_maximized())
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .size(px(34.0))
                            .rounded(px(9.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(ShellDeckColors::primary().opacity(0.12))
                            .child(lucide_icon("pencil", 17.0, ShellDeckColors::primary())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("user.requests.annotator.title").to_string()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("user.requests.annotator.subtitle").to_string()),
                            ),
                    )
                    .child(
                        Button::new(
                            "annotator-header-close",
                            t!("user.requests.annotator.cancel").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .icon(IconSource::from("x"))
                        .on_click(move |_, _, cx| {
                            close_entity.update(cx, |this, cx| this.close(cx));
                        }),
                    ),
            )
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .p(px(14.0))
                    .bg(gpui::black().opacity(0.9))
                    .child(self.render_canvas(cx)),
            )
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .px(px(16.0))
                        .py(px(8.0))
                        .bg(ShellDeckColors::error().opacity(0.12))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::error())
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(16.0))
                    .py(px(11.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.requests.annotator.hint").to_string()),
                    )
                    .child(
                        Button::new(
                            "annotator-cancel",
                            t!("user.requests.annotator.cancel").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click({
                            let entity = cx.entity();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| this.close(cx));
                            }
                        }),
                    )
                    .child(
                        Button::new(
                            "annotator-apply",
                            t!("user.requests.annotator.attach").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Default)
                        .icon(IconSource::from("check"))
                        .on_click(move |_, _, cx| {
                            apply_entity.update(cx, |this, cx| this.apply(cx));
                        }),
                    ),
            )
            .into_any_element()
    }
}

fn paint_annotation(window: &mut Window, geometry: CanvasGeometry, annotation: &Annotation) {
    match annotation {
        Annotation::Arrow {
            start,
            end,
            color,
            width,
        } => {
            let start = geometry.screen(*start);
            let end = geometry.screen(*end);
            let stroke = (geometry.min_side() * width).max(2.0);
            paint_line(window, start, end, stroke, color.ui());
            let dx = end.x.to_f64() as f32 - start.x.to_f64() as f32;
            let dy = end.y.to_f64() as f32 - start.y.to_f64() as f32;
            let angle = dy.atan2(dx);
            let head = (stroke * 4.2).max(12.0);
            for delta in [2.55_f32, -2.55_f32] {
                let point = gpui::point(
                    gpui::px(end.x.to_f64() as f32 + (angle + delta).cos() * head),
                    gpui::px(end.y.to_f64() as f32 + (angle + delta).sin() * head),
                );
                paint_line(window, end, point, stroke, color.ui());
            }
        }
        Annotation::Rectangle {
            start,
            end,
            color,
            width,
        } => {
            let a = geometry.screen(*start);
            let b = geometry.screen(*end);
            let points = [a, gpui::point(b.x, a.y), b, gpui::point(a.x, b.y), a];
            paint_polyline(
                window,
                &points,
                (geometry.min_side() * width).max(2.0),
                color.ui(),
            );
        }
        Annotation::Pen {
            points,
            color,
            width,
        } => {
            let points = points
                .iter()
                .map(|point| geometry.screen(*point))
                .collect::<Vec<_>>();
            paint_polyline(
                window,
                &points,
                (geometry.min_side() * width).max(2.0),
                color.ui(),
            );
        }
        Annotation::Marker {
            point,
            number,
            color,
        } => paint_marker(window, geometry, *point, *number, color.ui()),
    }
}

fn paint_line(
    window: &mut Window,
    start: Point<Pixels>,
    end: Point<Pixels>,
    width: f32,
    color: Hsla,
) {
    paint_polyline(window, &[start, end], width, color);
}

fn paint_polyline(window: &mut Window, points: &[Point<Pixels>], width: f32, color: Hsla) {
    if points.len() < 2 {
        return;
    }
    let mut builder = PathBuilder::stroke(gpui::px(width));
    for (index, point) in points.iter().enumerate() {
        if index == 0 {
            builder.move_to(*point);
        } else {
            builder.line_to(*point);
        }
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn paint_marker(
    window: &mut Window,
    geometry: CanvasGeometry,
    point: NormPoint,
    number: u8,
    color: Hsla,
) {
    let center = geometry.screen(point);
    let radius = (geometry.min_side() * 0.026).clamp(12.0, 24.0);
    window.paint_quad(quad(
        Bounds::new(
            gpui::point(center.x - gpui::px(radius), center.y - gpui::px(radius)),
            gpui::size(gpui::px(radius * 2.0), gpui::px(radius * 2.0)),
        ),
        gpui::px(radius),
        color,
        gpui::px(2.0),
        gpui::white(),
        BorderStyle::default(),
    ));
    paint_digit(
        window,
        center,
        number,
        radius * 0.8,
        (radius * 0.13).max(1.5),
        gpui::white(),
    );
}

fn digit_segments(number: u8) -> [bool; 7] {
    match number {
        1 => [false, true, true, false, false, false, false],
        2 => [true, true, false, true, true, false, true],
        3 => [true, true, true, true, false, false, true],
        4 => [false, true, true, false, false, true, true],
        5 => [true, false, true, true, false, true, true],
        6 => [true, false, true, true, true, true, true],
        7 => [true, true, true, false, false, false, false],
        8 => [true, true, true, true, true, true, true],
        9 => [true, true, true, true, false, true, true],
        _ => [true, true, true, true, true, true, false],
    }
}

fn paint_digit(
    window: &mut Window,
    center: Point<Pixels>,
    number: u8,
    height: f32,
    width: f32,
    color: Hsla,
) {
    let half_w = height * 0.22;
    let half_h = height * 0.42;
    let cx = center.x.to_f64() as f32;
    let cy = center.y.to_f64() as f32;
    let segments = [
        ((cx - half_w, cy - half_h), (cx + half_w, cy - half_h)),
        ((cx + half_w, cy - half_h), (cx + half_w, cy)),
        ((cx + half_w, cy), (cx + half_w, cy + half_h)),
        ((cx - half_w, cy + half_h), (cx + half_w, cy + half_h)),
        ((cx - half_w, cy), (cx - half_w, cy + half_h)),
        ((cx - half_w, cy - half_h), (cx - half_w, cy)),
        ((cx - half_w, cy), (cx + half_w, cy)),
    ];
    for (enabled, (start, end)) in digit_segments(number).into_iter().zip(segments) {
        if enabled {
            paint_line(
                window,
                gpui::point(gpui::px(start.0), gpui::px(start.1)),
                gpui::point(gpui::px(end.0), gpui::px(end.1)),
                width,
                color,
            );
        }
    }
}

fn annotated_draft(
    original: &AttachmentDraft,
    annotations: &[Annotation],
) -> Result<AttachmentDraft, String> {
    if annotations.is_empty() {
        return Ok(original.clone());
    }
    let mut image = image::load_from_memory(original.bytes.as_ref())
        .map_err(|error| error.to_string())?
        .into_rgba8();
    for annotation in annotations {
        raster_annotation(&mut image, annotation);
    }
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, RasterFormat::Png)
        .map_err(|error| error.to_string())?;
    AttachmentDraft::from_bytes("capture-annotee.png", bytes.into_inner())
}

fn raster_annotation(image: &mut RgbaImage, annotation: &Annotation) {
    let width = image.width() as f32;
    let height = image.height() as f32;
    let min_side = width.min(height);
    let point = |p: NormPoint| ((p.x * width).round() as i32, (p.y * height).round() as i32);
    match annotation {
        Annotation::Arrow {
            start,
            end,
            color,
            width,
        } => {
            let start = point(*start);
            let end = point(*end);
            let stroke = (min_side * width).round().max(2.0) as i32;
            raster_line(image, start, end, stroke, color.raster());
            let dx = (end.0 - start.0) as f32;
            let dy = (end.1 - start.1) as f32;
            let angle = dy.atan2(dx);
            let head = (stroke as f32 * 4.2).max(12.0);
            for delta in [2.55_f32, -2.55_f32] {
                let tip = (
                    (end.0 as f32 + (angle + delta).cos() * head).round() as i32,
                    (end.1 as f32 + (angle + delta).sin() * head).round() as i32,
                );
                raster_line(image, end, tip, stroke, color.raster());
            }
        }
        Annotation::Rectangle {
            start,
            end,
            color,
            width,
        } => {
            let a = point(*start);
            let b = point(*end);
            let stroke = (min_side * width).round().max(2.0) as i32;
            for (from, to) in [
                (a, (b.0, a.1)),
                ((b.0, a.1), b),
                (b, (a.0, b.1)),
                ((a.0, b.1), a),
            ] {
                raster_line(image, from, to, stroke, color.raster());
            }
        }
        Annotation::Pen {
            points,
            color,
            width,
        } => {
            let stroke = (min_side * width).round().max(2.0) as i32;
            for pair in points.windows(2) {
                raster_line(
                    image,
                    point(pair[0]),
                    point(pair[1]),
                    stroke,
                    color.raster(),
                );
            }
        }
        Annotation::Marker {
            point: marker,
            number,
            color,
        } => {
            let center = point(*marker);
            let radius = (min_side * 0.026).round().clamp(12.0, 32.0) as i32;
            raster_disc(image, center, radius, color.raster());
            raster_digit(
                image,
                center,
                *number,
                radius as f32 * 0.8,
                (radius as f32 * 0.13).max(2.0) as i32,
                Rgba([255, 255, 255, 255]),
            );
        }
    }
}

fn raster_line(
    image: &mut RgbaImage,
    start: (i32, i32),
    end: (i32, i32),
    width: i32,
    color: Rgba<u8>,
) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let steps = dx.abs().max(dy.abs()).max(1);
    let radius = (width / 2).max(1);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = (start.0 as f32 + dx as f32 * t).round() as i32;
        let y = (start.1 as f32 + dy as f32 * t).round() as i32;
        raster_disc(image, (x, y), radius, color);
    }
}

fn raster_disc(image: &mut RgbaImage, center: (i32, i32), radius: i32, color: Rgba<u8>) {
    for y in -radius..=radius {
        for x in -radius..=radius {
            if x * x + y * y <= radius * radius {
                let px = center.0 + x;
                let py = center.1 + y;
                if px >= 0 && py >= 0 && px < image.width() as i32 && py < image.height() as i32 {
                    image.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}

fn raster_digit(
    image: &mut RgbaImage,
    center: (i32, i32),
    number: u8,
    height: f32,
    width: i32,
    color: Rgba<u8>,
) {
    let half_w = height * 0.22;
    let half_h = height * 0.42;
    let cx = center.0 as f32;
    let cy = center.1 as f32;
    let segments = [
        ((cx - half_w, cy - half_h), (cx + half_w, cy - half_h)),
        ((cx + half_w, cy - half_h), (cx + half_w, cy)),
        ((cx + half_w, cy), (cx + half_w, cy + half_h)),
        ((cx - half_w, cy + half_h), (cx + half_w, cy + half_h)),
        ((cx - half_w, cy), (cx - half_w, cy + half_h)),
        ((cx - half_w, cy - half_h), (cx - half_w, cy)),
        ((cx - half_w, cy), (cx + half_w, cy)),
    ];
    for (enabled, (start, end)) in digit_segments(number).into_iter().zip(segments) {
        if enabled {
            raster_line(
                image,
                (start.0.round() as i32, start.1.round() as i32),
                (end.0.round() as i32, end.1.round() as i32),
                width,
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{annotated_draft, Annotation, AnnotationColor, NormPoint};
    use crate::issue_attachments::AttachmentDraft;
    use image::{DynamicImage, ImageFormat as RasterFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    fn png_draft() -> AttachmentDraft {
        let image = RgbaImage::from_pixel(80, 60, Rgba([255, 255, 255, 255]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, RasterFormat::Png)
            .unwrap();
        AttachmentDraft::from_bytes("capture.png", bytes.into_inner()).unwrap()
    }

    #[test]
    fn annotation_export_is_a_valid_changed_png() {
        let original = png_draft();
        let output = annotated_draft(
            &original,
            &[Annotation::Arrow {
                start: NormPoint { x: 0.1, y: 0.1 },
                end: NormPoint { x: 0.8, y: 0.8 },
                color: AnnotationColor::Red,
                width: 0.02,
            }],
        )
        .unwrap();
        assert_eq!(output.content_type, "image/png");
        assert_ne!(output.bytes.as_ref(), original.bytes.as_ref());
        assert_eq!(
            image::load_from_memory(output.bytes.as_ref())
                .unwrap()
                .width(),
            80
        );
    }

    #[test]
    fn empty_export_preserves_original_draft() {
        let original = png_draft();
        let output = annotated_draft(&original, &[]).unwrap();
        assert_eq!(output.bytes.as_ref(), original.bytes.as_ref());
    }
}
