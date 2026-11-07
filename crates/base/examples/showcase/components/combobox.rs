use super::*;
use gpui_base::HighlightStep;

/// The frameworks the example combobox offers.
const FRAMEWORKS: [&str; 4] = ["GPUI", "React", "SwiftUI", "Vue"];

impl BaseShowcase {
    /// A combobox with its text input in the field: typing filters the list
    /// and opens it, the arrows move the highlight while the caret stays in
    /// the input, Enter commits, and Escape closes, then clears.
    pub(in super::super) fn combobox(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.combobox_open;
        let text = self.combobox_query.read(cx).value();
        // The field showing the committed label filters nothing, so the list
        // opens on every framework with the committed one highlighted.
        let filter = (&*text != self.combobox_selection.as_str()).then(|| text.to_lowercase());
        let matches: Vec<&'static str> = FRAMEWORKS
            .into_iter()
            .filter(|label| {
                filter
                    .as_ref()
                    .is_none_or(|filter| label.to_lowercase().contains(filter.as_str()))
            })
            .collect();
        let highlight = self.combobox_highlight.filter(|ix| *ix < matches.len());
        let clearable = !open && !text.is_empty();
        let entity = cx.weak_entity();

        let combobox = Combobox::new("example-combobox")
            .open(open)
            .on_open_change({
                let entity = entity.clone();
                move |open, window, cx| {
                    _ = entity.update(cx, |this, cx| {
                        if open {
                            this.open_combobox(cx);
                        } else {
                            this.close_combobox(window, cx);
                        }
                    });
                }
            })
            .on_highlight({
                let entity = entity.clone();
                let count = matches.len();
                move |step, _, cx| {
                    _ = entity.update(cx, |this, cx| {
                        this.combobox_highlight =
                            step_highlight(this.combobox_highlight, step, count);
                        cx.notify();
                    });
                }
            })
            .on_confirm({
                let entity = entity.clone();
                let matches = matches.clone();
                move |window, cx| {
                    let Some(label) = highlight.and_then(|ix| matches.get(ix).copied()) else {
                        return;
                    };
                    _ = entity.update(cx, |this, cx| this.commit_framework(label, window, cx));
                }
            })
            .when(clearable, |this| {
                let entity = entity.clone();
                this.on_clear(move |window, cx| {
                    _ = entity.update(cx, |this, cx| {
                        this.combobox_selection.clear();
                        this.combobox_query
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        cx.notify();
                    });
                })
            })
            .w_56()
            .child(
                div()
                    .id("combobox-field")
                    .w_full()
                    .h_7()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_1()
                    .border_color(super::example_rgb(0xd4d4d4))
                    .text_xs()
                    .bg(super::example_rgb(0xffffff))
                    .on_click({
                        let entity = entity.clone();
                        let query = self.combobox_query.clone();
                        move |_, window, cx| {
                            query.update(cx, |state, cx| state.focus(window, cx));
                            _ = entity.update(cx, |this, cx| this.open_combobox(cx));
                        }
                    })
                    .child(
                        InputBase::new("combobox-input")
                            .flex_1()
                            .child(self.combobox_query.clone()),
                    )
                    .child(
                        div()
                            .text_color(super::example_rgb(0x737373))
                            .child(super::chevron(false)),
                    ),
            );
        let popup = div()
            .w_56()
            .p_1()
            .text_xs()
            .border_1()
            .border_color(super::example_rgb(0xd4d4d4))
            .bg(super::example_rgb(0xffffff))
            .children(matches.into_iter().enumerate().map(|(ix, label)| {
                let entity = entity.clone();
                div()
                    .id(format!("combobox-{label}"))
                    .px_2()
                    .h_7()
                    .flex()
                    .items_center()
                    .text_xs()
                    .when(highlight == Some(ix), |this| {
                        this.bg(super::example_rgb(0xf5f5f5))
                    })
                    .on_click(move |_, window, cx| {
                        _ = entity.update(cx, |this, cx| this.commit_framework(label, window, cx));
                    })
                    .child(label)
            }));

        Popup::new("example-combobox-popup", combobox).when(open, |this| this.content(popup))
    }

    /// Opens the list with the committed framework highlighted.
    fn open_combobox(&mut self, cx: &mut Context<Self>) {
        if self.combobox_open {
            return;
        }
        self.combobox_open = true;
        self.combobox_highlight = FRAMEWORKS
            .iter()
            .position(|label| *label == self.combobox_selection);
        cx.notify();
    }

    /// Closes the list and puts the committed label back over the typed text.
    fn close_combobox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.combobox_open = false;
        self.combobox_highlight = None;
        let label = self.combobox_selection.clone();
        self.combobox_query
            .update(cx, |state, cx| state.set_value(label, window, cx));
        cx.notify();
    }

    fn commit_framework(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.combobox_selection = label.into();
        self.close_combobox(window, cx);
    }
}

/// Where an arrow key moves the highlight among `count` rows: Down and Up
/// wrap, and a list opened with nothing highlighted starts at an end.
fn step_highlight(current: Option<usize>, step: HighlightStep, count: usize) -> Option<usize> {
    let last = count.checked_sub(1)?;
    Some(match (step, current) {
        (HighlightStep::Next, Some(ix)) if ix < last => ix + 1,
        (HighlightStep::Next, Some(_)) => 0,
        (HighlightStep::Previous, Some(ix)) if ix > 0 => ix - 1,
        (HighlightStep::Previous, Some(_)) => last,
        (HighlightStep::SelectedOrFirst | HighlightStep::SelectedOrLast, Some(ix)) => ix.min(last),
        (HighlightStep::Next | HighlightStep::SelectedOrFirst, None) => 0,
        (HighlightStep::Previous | HighlightStep::SelectedOrLast, None) => last,
    })
}
