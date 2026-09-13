use gpui::{
    Action, AsKeystroke, FocusHandle, Half, IntoElement, KeyContext, Keystroke, ParentElement as _,
    RenderOnce, StyleRefinement, Styled, Window, div, prelude::FluentBuilder as _, relative,
};

use crate::{ActiveTheme, StyledExt};

/// A tag for displaying keyboard keybindings.
#[derive(IntoElement, Clone, Debug)]
pub struct Kbd {
    style: StyleRefinement,
    stroke: Keystroke,
    appearance: bool,
    outline: bool,
}

impl From<Keystroke> for Kbd {
    fn from(stroke: Keystroke) -> Self {
        Self {
            style: StyleRefinement::default(),
            stroke,
            appearance: true,
            outline: false,
        }
    }
}

impl Kbd {
    /// Create a new Kbd element with the given [`Keystroke`].
    pub fn new(stroke: Keystroke) -> Self {
        Self {
            style: StyleRefinement::default(),
            stroke,
            appearance: true,
            outline: false,
        }
    }

    /// Set the appearance of the keybinding, default is `true`.
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }

    /// Use outline style for the keybinding, default is `false`.
    pub fn outline(mut self) -> Self {
        self.outline = true;
        self
    }

    /// Return the first keybinding for the given action and context.
    pub fn binding_for_action(
        action: &dyn Action,
        context: Option<&str>,
        window: &Window,
    ) -> Option<Self> {
        let key_context = context.and_then(|context| KeyContext::parse(context).ok());
        let binding = match key_context {
            Some(context) => {
                window.highest_precedence_binding_for_action_in_context(action, context)
            }
            None => window.highest_precedence_binding_for_action(action),
        }?;

        if let Some(key) = binding.keystrokes().first() {
            Some(Self::new(key.as_keystroke().clone()))
        } else {
            None
        }
    }

    /// Return the first keybinding for the given action and focus handle.
    pub fn binding_for_action_in(
        action: &dyn Action,
        focus_handle: &FocusHandle,
        window: &Window,
    ) -> Option<Self> {
        let binding = window.highest_precedence_binding_for_action_in(action, focus_handle)?;
        if let Some(key) = binding.keystrokes().first() {
            Some(Self::new(key.as_keystroke().clone()))
        } else {
            None
        }
    }

    /// Return the Platform specific keybinding string by KeyStroke
    ///
    /// macOS: https://support.apple.com/en-us/HT201236
    /// Windows: https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec
    pub fn format(key: &Keystroke) -> String {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        const SEPARATOR: &str = "";
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        const SEPARATOR: &str = "+";

        let mut parts = vec![];

        // The key map order in macOS is: ⌃⌥⇧⌘
        // And in Windows is: Ctrl+Alt+Shift+Win

        if key.modifiers.control {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            parts.push("⌃");

            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            parts.push("Ctrl");
        }

        if key.modifiers.alt {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            parts.push("⌥");

            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            parts.push("Alt");
        }

        if key.modifiers.shift {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            parts.push("⇧");

            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            parts.push("Shift");
        }

        if key.modifiers.platform {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            parts.push("⌘");

            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            parts.push("Win");
        }

        let mut keys = String::new();
        let key_str = key.key.as_str();
        match key_str {
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "ctrl" => keys.push('⌃'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "ctrl" => keys.push_str("Ctrl"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "alt" => keys.push('⌥'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "alt" => keys.push_str("Alt"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "shift" => keys.push('⇧'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "shift" => keys.push_str("Shift"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "cmd" => keys.push('⌘'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "cmd" => keys.push_str("Win"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "space" => keys.push_str("Space"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "backspace" => keys.push('⌫'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "backspace" => keys.push_str("Backspace"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "delete" => keys.push('⌫'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "delete" => keys.push_str("Delete"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "escape" => keys.push('⎋'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "escape" => keys.push_str("Esc"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "enter" => keys.push('⏎'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "enter" => keys.push_str("Enter"),
            "pagedown" => keys.push_str("Page Down"),
            "pageup" => keys.push_str("Page Up"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "left" => keys.push('←'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "left" => keys.push_str("Left"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "right" => keys.push('→'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "right" => keys.push_str("Right"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "up" => keys.push('↑'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "up" => keys.push_str("Up"),
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            "down" => keys.push('↓'),
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            "down" => keys.push_str("Down"),
            _ => {
                if key_str.len() == 1 {
                    keys.push_str(&key_str.to_uppercase());
                } else {
                    let mut chars = key_str.chars();
                    if let Some(first_char) = chars.next() {
                        keys.push_str(&format!(
                            "{}{}",
                            first_char.to_uppercase(),
                            chars.collect::<String>()
                        ));
                    } else {
                        keys.push_str(&key_str);
                    }
                }
            }
        }

        parts.push(&keys);
        parts.join(SEPARATOR)
    }
}

impl Styled for Kbd {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Kbd {
    fn render(self, _: &mut gpui::Window, cx: &mut gpui::App) -> impl gpui::IntoElement {
        if !self.appearance {
            return Self::format(&self.stroke).into_any_element();
        }

        div()
            .text_color(cx.theme().muted_foreground)
            .bg(cx.theme().tokens.muted)
            .when(self.outline, |this| {
                this.border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().tokens.background)
            })
            .py_0p5()
            .px_1()
            .min_w_5()
            .text_center()
            .rounded(cx.theme().radius.half())
            .line_height(relative(1.))
            .text_xs()
            .whitespace_normal()
            .flex_shrink_0()
            .refine_style(&self.style)
            .child(Self::format(&self.stroke))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_format() {
        use super::Kbd;
        use gpui::Keystroke;

        if cfg!(target_os = "macos") {
            assert_eq!(Kbd::format(&Keystroke::parse("cmd-a").unwrap()), "⌘A");
            assert_eq!(Kbd::format(&Keystroke::parse("cmd--").unwrap()), "⌘-");
            assert_eq!(Kbd::format(&Keystroke::parse("cmd-+").unwrap()), "⌘+");
            assert_eq!(Kbd::format(&Keystroke::parse("cmd-enter").unwrap()), "⌘⏎");
            assert_eq!(
                Kbd::format(&Keystroke::parse("secondary-f12").unwrap()),
                "⌘F12"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("shift-pagedown").unwrap()),
                "⇧Page Down"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("shift-pageup").unwrap()),
                "⇧Page Up"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("shift-space").unwrap()),
                "⇧Space"
            );
            assert_eq!(Kbd::format(&Keystroke::parse("cmd-ctrl-a").unwrap()), "⌃⌘A");
            assert_eq!(
                Kbd::format(&Keystroke::parse("cmd-alt-backspace").unwrap()),
                "⌥⌘⌫"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("shift-delete").unwrap()),
                "⇧⌫"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("cmd-ctrl-shift-a").unwrap()),
                "⌃⇧⌘A"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("cmd-ctrl-shift-alt-a").unwrap()),
                "⌃⌥⇧⌘A"
            );
        } else {
            assert_eq!(Kbd::format(&Keystroke::parse("a").unwrap()), "A");
            assert_eq!(Kbd::format(&Keystroke::parse("ctrl-a").unwrap()), "Ctrl+A");
            assert_eq!(
                Kbd::format(&Keystroke::parse("shift-space").unwrap()),
                "Shift+Space"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("ctrl-alt-a").unwrap()),
                "Ctrl+Alt+A"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("ctrl-alt-shift-a").unwrap()),
                "Ctrl+Alt+Shift+A"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("ctrl-alt-shift-win-a").unwrap()),
                "Ctrl+Alt+Shift+Win+A"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("ctrl-shift-backspace").unwrap()),
                "Ctrl+Shift+Backspace"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("alt-delete").unwrap()),
                "Alt+Delete"
            );
            assert_eq!(
                Kbd::format(&Keystroke::parse("alt-tab").unwrap()),
                "Alt+Tab"
            );
        }
    }
}
