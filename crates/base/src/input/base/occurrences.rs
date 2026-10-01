//! Selecting the occurrences of a selection: the next one added as another selection, or all
//! of them at once. With only a caret, the first press selects the word under it.

use gpui::{Context, Window, actions};

use super::cursor::CursorSelection;
use super::{InputBaseState, InputModeKind, RopeExt as _};

actions!(
    input,
    [
        /// Select the word under the caret, or add the next occurrence of the selected text as
        /// another selection, wrapping past the end.
        SelectNextOccurrence,
        /// Select the word under the caret, then every occurrence of the selected text.
        SelectAllOccurrences,
    ]
);

impl<M: InputModeKind> InputBaseState<M> {
    /// The word under the newest caret selected in its place, when every selection is a caret.
    /// Returns whether it did, so the press does nothing more.
    fn select_word_under_caret(&mut self, cx: &mut Context<Self>) -> bool {
        if self.selections.iter().any(|s| !s.is_empty()) {
            return false;
        }
        let caret = self.active_selection().cursor_offset();
        // At a word's end the caret is past it: the word is the one just before.
        let word = self.text.word_range(caret).or_else(|| {
            caret
                .checked_sub(1)
                .and_then(|before| self.text.word_range(before))
                .filter(|word| word.end == caret)
        });
        let Some(word) = word else { return true };
        self.selections.remove_all_but_active();
        let active = self.active_selection_mut();
        active.start = word.start;
        active.end = word.end;
        active.reversed = false;
        active.column_anchor = None;
        cx.notify();
        true
    }

    /// The selected text the occurrences are of: the newest selection's.
    fn occurrence_needle(&self) -> Option<(String, usize)> {
        let newest = self.selections.iter().last()?;
        (!newest.is_empty()).then(|| {
            (
                self.text.slice(newest.start..newest.end).to_string(),
                newest.end,
            )
        })
    }

    /// Whether `start..end` overlaps a selection already made.
    fn overlaps_selection(&self, start: usize, end: usize) -> bool {
        self.selections
            .iter()
            .any(|s| start < s.end && s.start < end)
    }

    fn add_occurrence(&mut self, start: usize, end: usize) {
        let id = self.selections.generate_id();
        self.selections.add(CursorSelection::new(id, start, end));
    }

    pub(super) fn select_next_occurrence(
        &mut self,
        _: &SelectNextOccurrence,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_multi_line() {
            return;
        }
        self.pause_blink_cursor(cx);
        self.undo_manager.break_transaction_coalescing();
        if self.select_word_under_caret(cx) {
            return;
        }
        let Some((needle, from)) = self.occurrence_needle() else {
            return;
        };
        let text = self.text.to_string();
        let after = text
            .get(from..)
            .unwrap_or_default()
            .match_indices(&needle)
            .map(|(at, _)| at + from);
        let before = text
            .get(..from)
            .unwrap_or_default()
            .match_indices(&needle)
            .map(|(at, _)| at);
        let next = after
            .chain(before)
            .find(|&at| !self.overlaps_selection(at, at + needle.len()));
        if let Some(start) = next {
            self.add_occurrence(start, start + needle.len());
            self.scroll_to(start + needle.len(), None, cx);
        }
        cx.notify();
    }

    pub(super) fn select_all_occurrences(
        &mut self,
        _: &SelectAllOccurrences,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_multi_line() {
            return;
        }
        self.pause_blink_cursor(cx);
        self.undo_manager.break_transaction_coalescing();
        self.select_word_under_caret(cx);
        let Some((needle, _)) = self.occurrence_needle() else {
            return;
        };
        let text = self.text.to_string();
        let starts: Vec<usize> = text.match_indices(&needle).map(|(at, _)| at).collect();
        for start in starts {
            if !self.overlaps_selection(start, start + needle.len()) {
                self.add_occurrence(start, start + needle.len());
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Context, Entity, IntoElement, Render, TestAppContext, VisualTestContext,
        Window,
    };

    use crate::input::{EditorMode, InputBaseState};

    use super::{SelectAllOccurrences, SelectNextOccurrence};

    struct View(Entity<InputBaseState<EditorMode>>);

    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.clone()
        }
    }

    fn editor<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
        caret: usize,
    ) -> (
        Entity<InputBaseState<EditorMode>>,
        &'a mut VisualTestContext,
    ) {
        cx.update(|cx| {
            cx.set_global(crate::Theme::default());
            crate::input::init(cx);
        });
        let text = text.to_owned();
        let (view, cx) = cx.add_window_view(|window, cx| {
            let state = cx.new(|cx| {
                let mut state = InputBaseState::<EditorMode>::new(window, cx);
                state.set_value(text.clone(), window, cx);
                state.set_selected_range(caret..caret, cx);
                state
            });
            View(state)
        });
        let state = view.read_with(cx, |v, _| v.0.clone());
        (state, cx)
    }

    fn selected(
        state: &Entity<InputBaseState<EditorMode>>,
        cx: &VisualTestContext,
    ) -> Vec<(usize, usize)> {
        state.read_with(cx, |s, _| {
            let mut out: Vec<_> = s.selections.iter().map(|s| (s.start, s.end)).collect();
            out.sort_unstable();
            out
        })
    }

    #[gpui::test]
    fn test_select_next_occurrence_starts_with_the_word_and_wraps(cx: &mut TestAppContext) {
        let (state, cx) = editor(cx, "foo bar foo baz foo", 9);
        let next = |cx: &mut VisualTestContext| {
            cx.update(|window, cx| {
                state.update(cx, |s, cx| {
                    s.select_next_occurrence(&SelectNextOccurrence, window, cx)
                });
            });
        };
        next(cx);
        assert_eq!(selected(&state, cx), [(8, 11)], "the word under the caret");
        next(cx);
        assert_eq!(
            selected(&state, cx),
            [(8, 11), (16, 19)],
            "the next one after it"
        );
        next(cx);
        assert_eq!(
            selected(&state, cx),
            [(0, 3), (8, 11), (16, 19)],
            "wrapping past the end"
        );
        next(cx);
        assert_eq!(
            selected(&state, cx).len(),
            3,
            "none left that is not selected"
        );
    }

    #[gpui::test]
    fn test_select_all_occurrences_selects_every_one(cx: &mut TestAppContext) {
        let (state, cx) = editor(cx, "ab x ab y ab", 0);
        cx.update(|window, cx| {
            state.update(cx, |s, cx| {
                s.select_all_occurrences(&SelectAllOccurrences, window, cx)
            });
        });
        assert_eq!(selected(&state, cx), [(0, 2), (5, 7), (10, 12)]);
    }

    #[gpui::test]
    fn test_a_caret_off_any_word_selects_nothing(cx: &mut TestAppContext) {
        let (state, cx) = editor(cx, "a  b", 2);
        cx.update(|window, cx| {
            state.update(cx, |s, cx| {
                s.select_next_occurrence(&SelectNextOccurrence, window, cx)
            });
        });
        assert_eq!(selected(&state, cx), [(2, 2)]);
    }
}
