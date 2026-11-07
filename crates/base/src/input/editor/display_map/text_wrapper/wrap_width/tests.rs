//! Differential wrapping and edit invalidation checks over the original display-map implementation.

use super::super::{TextWrapper, WrappingIndent, measured_wrap, measured_wrap_boundaries};
use super::*;
use crate::input::RopeExt as _;
use gpui::{App, TestAppContext, font};
use ropey::Rope;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation as _;

/// Deterministic fractional glyph widths include multibyte graphemes and contextual ligatures.
fn measure(text: &str) -> Pixels {
    let width: f32 = text
        .graphemes(true)
        .map(|g| {
            if g == "\t" {
                28.25
            } else if g.is_ascii() {
                7.125
            } else {
                15.75
            }
        })
        .sum();
    px(width - text.matches("fi").count() as f32 * 0.875)
}

/// Compare complete logical-to-visual row geometry, including continuation indentation.
fn rows(wrapper: &TextWrapper) -> Vec<(usize, u32, Vec<Range<usize>>)> {
    wrapper
        .iter_lines()
        .map(|line| (line.len(), line.indent, line.wrapped_lines.to_vec()))
        .collect()
}

/// Run the same row-splice path with deterministic glyph widths and count actual rewraps.
fn resize(wrapper: &mut TextWrapper, text: &Rope, width: Pixels, reuse: bool) -> usize {
    wrapper.wrap_width = Some(width);
    let indent = wrapper.wrapping_indent;
    let mut calls = 0;
    wrapper.update_lines(
        text,
        &(0..text.len()),
        text,
        reuse,
        &mut |line, width, _| {
            calls += 1;
            measured_wrap(line, width, indent, measure)
        },
    );
    calls
}

#[test]
fn oversized_words_keep_measurement_work_local() {
    for length in [8192, 16_384] {
        for prefix in ["", "short ", "    "] {
            let text = format!("{prefix}{}", "x".repeat(length));
            for indent in [WrappingIndent::None, WrappingIndent::Same] {
                for width in [80., 256., 512.] {
                    let mut measured_bytes = 0;
                    let mut longest_probe = 0;
                    let measured = measured_wrap(&text, px(width), indent, |part| {
                        measured_bytes += part.len();
                        longest_probe = longest_probe.max(part.len());
                        px(part.len() as f32)
                    });
                    let original = measured_wrap_boundaries(&text, px(width), indent, |part| {
                        px(part.len() as f32)
                    });
                    assert_eq!(measured.boundaries, original);
                    assert!(
                        longest_probe <= 4 * width as usize,
                        "a {width}-pixel row measured a {longest_probe}-byte suffix"
                    );
                    assert!(
                        measured_bytes <= 32 * text.len(),
                        "{} bytes required {measured_bytes} bytes of shaping",
                        text.len()
                    );
                }
            }
        }
    }
}

#[test]
fn cached_resize_matches_cold_unicode_wrapping_at_fractional_thresholds() {
    let text = Rope::from(
        "    alphabet 中文😀 e\u{301} family 👨‍👩‍👧‍👦 finish\n\tfi ffi office\nمرحبا بالعالم\n\r\nend",
    );
    for indent in [WrappingIndent::None, WrappingIndent::Same] {
        let mut cached = TextWrapper::new(font("test"), px(14.), Some(px(160.)));
        cached.wrapping_indent = indent;
        resize(&mut cached, &text, px(160.), false);
        let mut reused = 0;
        // Narrow, widen, then jump across multiple break thresholds; test exact and adjacent floats.
        for value in (1..1440).chain((1..1440).rev()).chain([4096, 1, 1600, 23]) {
            for width in [
                value as f32 / 8.,
                (value as f32 / 8.).next_up(),
                (value as f32 / 8.).next_down(),
            ] {
                let wrapped = resize(&mut cached, &text, px(width), true);
                reused += text.lines_len().saturating_sub(wrapped);
                let mut cold = TextWrapper::new(font("test"), px(14.), Some(px(width)));
                cold.wrapping_indent = indent;
                cold._update(&text, &(0..text.len()), &text, &mut |line, width, _| {
                    measured_wrap_boundaries(line, width, indent, measure)
                });
                assert_eq!(
                    rows(&cached),
                    rows(&cold),
                    "width={width}, indent={indent:?}"
                );
            }
        }
        assert!(reused > 1000, "nearby widths must reuse real row geometry");
    }
}

/// A short changed line invalidates only itself while distant rows retain their measured decisions.
#[gpui::test]
fn resize_reuses_measured_lines_and_invalidates_edits_fonts_indent_and_tokens(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let text = Rope::from("short\n    a longer line containing 中文 and fi\nlast\n");
        let mut cached = TextWrapper::new(font("Arial"), px(14.), Some(px(1000.)));
        cached.update(&text, &(0..0), &text, cx);
        MEASURED_LINES.with(|count| count.set(0));
        for width in (800..1000).rev() {
            cached.set_wrap_width(Some(px(width as f32)), cx);
        }
        assert_eq!(
            MEASURED_LINES.with(|count| count.get()),
            0,
            "fitting lines must not be shaped again during a width drag"
        );
        cached.set_wrap_width(Some(px(1.)), cx);
        assert!(MEASURED_LINES.with(|count| count.get()) > 0);
        assert_cold(&cached, cx);

        let mut edited = text.clone();
        edited.replace(0..5, "edited 中文😀 new line\ninserted");
        cached.update(
            &edited,
            &(0..5),
            &Rope::from("edited 中文😀 new line\ninserted"),
            cx,
        );
        cached.set_wrap_width(Some(px(82.5)), cx);
        assert_cold(&cached, cx);
        cached.set_font(font("Arial"), px(22.), cx);
        assert_cold(&cached, cx);
        cached.set_wrapping_indent(WrappingIndent::None, cx);
        assert_cold(&cached, cx);
        cached.set_inline_metrics(std::rc::Rc::from([(0..6, px(80.))]), cx);
        cached.set_wrap_width(Some(px(120.)), cx);
        assert_cold(&cached, cx);
        cached.set_wrap_width(None, cx);
        assert_cold(&cached, cx);
        cached.set_wrap_width(Some(px(130.)), cx);
        assert_cold(&cached, cx);
    });
}

/// A fresh wrapper with the same source and rendering inputs is the uncached layout oracle.
fn assert_cold(cached: &TextWrapper, cx: &mut App) {
    let mut cold = TextWrapper::new(cached.font.clone(), cached.font_size, cached.wrap_width);
    cold.wrapping_indent = cached.wrapping_indent;
    cold.tab_size = cached.tab_size;
    cold.inline_metrics = cached.inline_metrics.clone();
    cold.update(&cached.text, &(0..0), &cached.text, cx);
    assert_eq!(rows(cached), rows(&cold));
}
