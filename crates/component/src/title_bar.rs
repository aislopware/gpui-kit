use crate::{ActiveTheme, InteractiveElementExt as _, StyledExt, h_flex};
use gpui::{
    AnyElement, App, Background, Context, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, RenderOnce, Rgba, StyleRefinement, Styled, TitlebarOptions,
    Window, WindowControlArea, WindowOptions, div, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _, px,
};
use gpui_base::TestSupportExt as _;
use smallvec::SmallVec;

pub const TITLE_BAR_HEIGHT: Pixels = px(34.);
#[cfg(target_os = "macos")]
const TITLE_BAR_LEFT_PADDING: Pixels = px(80.);
#[cfg(target_os = "ios")]
const TITLE_BAR_LEFT_PADDING: Pixels = px(12.);

fn default_title_bar_background(title_bar: Hsla, background: Hsla) -> Background {
    let title_bar_rgb = title_bar.to_rgb();
    let background_rgb = background.to_rgb();
    let mixed = Hsla::from(Rgba {
        r: title_bar_rgb.r * 0.55 + background_rgb.r * 0.45,
        g: title_bar_rgb.g * 0.55 + background_rgb.g * 0.45,
        b: title_bar_rgb.b * 0.55 + background_rgb.b * 0.45,
        a: title_bar_rgb.a * 0.55 + background_rgb.a * 0.45,
    });

    linear_gradient(
        180.,
        linear_color_stop(mixed, 0.),
        linear_color_stop(title_bar, 1.),
    )
}

/// TitleBar used to customize the appearance of the title bar.
///
/// We can put some elements inside the title bar.
#[derive(IntoElement)]
pub struct TitleBar {
    style: StyleRefinement,
    children: SmallVec<[AnyElement; 1]>,
}

impl TitleBar {
    /// Create a new TitleBar.
    pub fn new() -> Self {
        Self {
            style: StyleRefinement::default(),
            children: SmallVec::new(),
        }
    }

    /// Returns the default title bar options for compatible with the [`crate::TitleBar`].
    pub fn title_bar_options() -> TitlebarOptions {
        TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(gpui::point(px(9.0), px(9.0))),
        }
    }

    /// Returns the default window options for compatible with the [`crate::TitleBar`].
    ///
    /// Use this as the base of the [`WindowOptions`] of any window that renders a
    /// [`crate::TitleBar`], so the title bar owns dragging and double clicking itself:
    ///
    /// ```no_run
    /// # mod gpui_kit { pub use gpui::*; pub extern crate gpui_component as component; }
    /// # use gpui_kit::WindowOptions;
    /// # use gpui_kit::component::TitleBar;
    /// let options = WindowOptions {
    ///     window_min_size: None,
    ///     ..TitleBar::window_options()
    /// };
    /// ```
    pub fn window_options() -> WindowOptions {
        WindowOptions {
            titlebar: Some(Self::title_bar_options()),
            // The title bar draws itself and moves the window via `start_window_move`,
            // so AppKit must not treat it as a system window-move region. Otherwise macOS
            // handles title bar double clicks on its own (in addition to `on_double_click`
            // below) and delays title bar clicks while disambiguating double clicks.
            app_owns_titlebar_drag: true,
            ..Default::default()
        }
    }
}

impl Styled for TitleBar {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for TitleBar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

struct TitleBarState {
    should_move: bool,
}

// TODO: Remove this when GPUI has released v0.2.3
impl Render for TitleBarState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl RenderOnce for TitleBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_macos = cfg!(target_os = "macos");

        let state = window.use_state(cx, |_, _| TitleBarState { should_move: false });

        div().flex_shrink_0().child(
            div()
                .id("title-bar")
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(TITLE_BAR_HEIGHT)
                .pl(TITLE_BAR_LEFT_PADDING)
                .border_b_1()
                .border_color(cx.theme().title_bar_border)
                .bg(default_title_bar_background(
                    cx.theme().title_bar,
                    cx.theme().background,
                ))
                .refine_style(&self.style)
                .when(is_macos, |this| {
                    this.on_double_click(|_, window, _| window.titlebar_double_click())
                })
                // After `on_double_click`, which is implemented for `Stateful` only.
                .test_support()
                .on_mouse_down_out(window.listener_for(&state, |state, _, _, _| {
                    state.should_move = false;
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(&state, |state, _, _, _| {
                        state.should_move = true;
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    window.listener_for(&state, |state, _, _, _| {
                        state.should_move = false;
                    }),
                )
                .on_mouse_move(window.listener_for(&state, |state, _, window, _| {
                    if state.should_move {
                        state.should_move = false;
                        window.start_window_move();
                    }
                }))
                .child(
                    h_flex()
                        .id("bar")
                        .h_full()
                        .justify_between()
                        .flex_shrink_0()
                        .flex_1()
                        .window_control_area(WindowControlArea::Drag)
                        .when(window.is_fullscreen(), |this| this.pl_3())
                        .children(self.children),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Rgba, linear_color_stop, linear_gradient};

    #[test]
    fn test_default_title_bar_background() {
        let title_bar = Hsla::black();
        let background = Hsla::white();

        assert_eq!(
            default_title_bar_background(title_bar, background),
            linear_gradient(
                180.,
                linear_color_stop(
                    Hsla::from(Rgba {
                        r: 0.45,
                        g: 0.45,
                        b: 0.45,
                        a: 1.,
                    }),
                    0.,
                ),
                linear_color_stop(title_bar, 1.),
            )
        );
    }
}
