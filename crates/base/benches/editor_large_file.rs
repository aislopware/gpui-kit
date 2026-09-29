//! A code editor over a 50,000-line file, the way an app's file tile holds
//! one: no soft wrap, line numbers, and a highlighter that styles the visible
//! range from per-line tokens. One frame per iteration.
//!
//! - `editor_open`: set the whole text and draw it.
//! - `editor_scroll`: a steady swipe, 40px a frame.
//! - `editor_type`: type a character in the middle of the file.
//! - `editor_idle`: draw a frame for a sibling view only; the editor did not
//!   change, so under retained drawing it should cost nothing.
//!
//! Run with `cargo bench -p gpui-base --bench editor_large_file`.

use std::{fmt::Write as _, ops::Range, rc::Rc};

use gpui::{
    AppContext as _, BenchAppContext, Context, Entity, HighlightStyle, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div, hsla, point, px,
};
use gpui_base::input::{
    Editor, EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope,
};

const LINES: usize = 50_000;

/// Rust-looking source: functions of a dozen lines with comments, strings and
/// numbers, so lines have several runs each.
fn source(seed: usize) -> String {
    let mut out = String::with_capacity(LINES * 40);
    let mut line = 0;
    let mut function = 0;
    while line < LINES {
        let _ = writeln!(out, "/// Handles case {function} of the table ({seed}).");
        let _ = writeln!(
            out,
            "pub fn handle_{function}(input: &str, count: usize) -> Option<usize> {{"
        );
        let _ = writeln!(out, "    let limit = {} + count * 3;", function % 97);
        let _ = writeln!(out, "    if input.is_empty() {{");
        let _ = writeln!(out, "        return None; // nothing to do");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(
            out,
            "    let name = format!(\"row-{{}}-{{}}\", input.len(), limit);"
        );
        let _ = writeln!(out, "    for (index, byte) in input.bytes().enumerate() {{");
        let _ = writeln!(out, "        if byte == b'x' && index > limit {{ break; }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "    Some(name.len() + limit)");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        line += 13;
        function += 1;
    }
    out
}

/// Styles keywords, comments and numbers in the requested range, reading the
/// rope the way a line-based highlighter reads its cached tokens.
struct KeywordHighlighter {
    text: Rope,
}

const KEYWORDS: [&str; 7] = ["pub", "fn", "let", "if", "for", "return", "Some"];

impl InputHighlighter for KeywordHighlighter {
    fn language(&self) -> SharedString {
        "rust".into()
    }

    fn update(
        &mut self,
        _: Option<InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        self.text = text.clone();
    }

    fn styles(
        &self,
        range: &Range<usize>,
        _: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let keyword = HighlightStyle {
            color: Some(hsla(0.8, 0.6, 0.6, 1.)),
            ..Default::default()
        };
        let comment = HighlightStyle {
            color: Some(hsla(0.3, 0.3, 0.5, 1.)),
            ..Default::default()
        };
        let mut runs = Vec::new();
        let mut plain_from = range.start;
        let mut push = |runs: &mut Vec<_>, run: Range<usize>, style: HighlightStyle| {
            if plain_from < run.start {
                runs.push((plain_from..run.start, HighlightStyle::default()));
            }
            plain_from = run.end;
            runs.push((run, style));
        };
        let end = range.end.min(self.text.len());
        let start = range.start.min(end);
        let text = self.text.slice(start..end).to_string();
        let mut offset = start;
        for line in text.split_inclusive('\n') {
            if let Some(at) = line.find("//") {
                push(&mut runs, offset + at..offset + line.len(), comment);
            } else {
                let mut word_start = None;
                for (i, c) in line.char_indices().chain([(line.len(), ' ')]) {
                    match (c.is_alphanumeric() || c == '_', word_start) {
                        (true, None) => word_start = Some(i),
                        (false, Some(from)) => {
                            if KEYWORDS.contains(&&line[from..i]) {
                                push(&mut runs, offset + from..offset + i, keyword);
                            }
                            word_start = None;
                        }
                        _ => {}
                    }
                }
            }
            offset += line.len();
        }
        if plain_from < range.end {
            runs.push((plain_from..range.end, HighlightStyle::default()));
        }
        runs
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

struct FileTile {
    editor: Entity<EditorState>,
    status: Entity<Status>,
}

/// A sibling that changes on its own, like a status bar clock.
struct Status {
    ticks: usize,
}

impl Render for Status {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().h(px(20.)).child(format!("{} ticks", self.ticks))
    }
}

impl Render for FileTile {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().child(Editor::new(&self.editor)))
            .child(self.status.clone())
    }
}

fn open(cx: &mut BenchAppContext, text: &str) -> Entity<FileTile> {
    cx.update(gpui_base::init);
    let mut window = cx.add_empty_window();
    let text = text.to_owned();
    let tile = window.update(|window, cx| {
        window.replace_root(cx, |window, cx| {
            let editor = cx.new(|cx| {
                let mut state = EditorState::new(window, cx).soft_wrap(false);
                state.set_highlighter_factory(
                    Rc::new(|_| {
                        Some(Box::new(KeywordHighlighter { text: Rope::new() })
                            as Box<dyn InputHighlighter>)
                    }),
                    cx,
                );
                state.set_highlighter("rust", cx);
                state.set_value(text, window, cx);
                state
            });
            FileTile {
                editor,
                status: cx.new(|_| Status { ticks: 0 }),
            }
        })
    });
    cx.run_until_idle();
    tile
}

#[gpui::bench]
fn editor_open(cx: &mut BenchAppContext) {
    let texts = [source(0), source(1)];
    let tile = open(cx, &texts[0]);
    let mut turn = 0;
    cx.bench_renderer(tile, move |tile, window, cx| {
        turn += 1;
        let text = texts[turn % 2].clone();
        tile.editor
            .update(cx, |state, cx| state.set_value(text, window, cx));
    });
}

#[gpui::bench]
fn editor_scroll(cx: &mut BenchAppContext) {
    let tile = open(cx, &source(0));
    let mut offset = px(0.);
    cx.bench_renderer(tile, move |tile, _, cx| {
        offset += px(40.);
        if offset > px(400_000.) {
            offset = px(0.);
        }
        tile.editor.update(cx, |state, cx| {
            state.set_scroll_offset(point(px(0.), -offset), cx)
        });
    });
}

#[gpui::bench]
fn editor_type(cx: &mut BenchAppContext) {
    let tile = open(cx, &source(0));
    cx.update(|cx| {
        let window = cx.windows()[0];
        window
            .update(cx, |_, window, cx| {
                tile.read(cx).editor.clone().update(cx, |state, cx| {
                    state.set_cursor_position(
                        gpui_base::input::Position::new(LINES as u32 / 2, 4),
                        window,
                        cx,
                    );
                });
            })
            .ok();
    });
    let mut turn = 0usize;
    cx.bench_renderer(tile, move |tile, window, cx| {
        turn += 1;
        // Type a line of text, breaking it every eighth key.
        let text = if turn % 8 == 0 { "\n" } else { "x" };
        tile.editor
            .update(cx, |state, cx| state.insert(text, window, cx));
    });
}

#[gpui::bench]
fn editor_idle(cx: &mut BenchAppContext) {
    let tile = open(cx, &source(0));
    cx.bench_renderer(tile, move |tile, _, cx| {
        tile.status.update(cx, |status, cx| {
            status.ticks += 1;
            cx.notify();
        });
    });
}

gpui::bench_group!(
    benches,
    editor_open,
    editor_scroll,
    editor_type,
    editor_idle
);
gpui::bench_main!(benches);
