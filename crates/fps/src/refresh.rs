//! The refresh rate of the display a window is on, read from the platform.
//!
//! GPUI does not report it, and it cannot be recovered from the frames a
//! window presented: those gaps are whole multiples of the panel's period, so
//! they bound it from below and never from above — 41.7ms is six refreshes at
//! 144Hz and one at 24Hz, and nothing in the timing says which. Every estimate
//! tried before this read a real window wrong.
//!
//! So it is asked for. GPUI does hand out the platform's own display handle
//! through [`gpui::DisplayId`], which is a `CGDirectDisplayID` on macOS. iOS
//! has no query here.
//!
//! `None` means nobody could say — a platform without a query here, a virtual
//! display, or a panel with no fixed rate — and the caller shows an uncapped
//! reading rather than one held to a guess.

use std::time::Duration;

use gpui::PlatformDisplay;

/// The period between refreshes of `display`, when the platform reports one.
pub(crate) fn display_refresh_rate(display: &dyn PlatformDisplay) -> Option<Duration> {
    platform::refresh_rate(display)
}

/// Turns a rate in hertz into the period the rest of the crate works in,
/// rejecting the zeroes platforms use to mean "no fixed rate".
#[cfg(target_os = "macos")]
fn period_from_hertz(hertz: f64) -> Option<Duration> {
    (hertz > 1.).then(|| Duration::from_secs_f64(1. / hertz))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use core_graphics::display::{CGDirectDisplayID, CGDisplay};

    pub(super) fn refresh_rate(display: &dyn PlatformDisplay) -> Option<Duration> {
        let id: u64 = display.id().into();
        // Zero rather than an error is how CoreGraphics says this display has
        // no fixed rate, which is what a built-in panel reports: on ProMotion
        // there genuinely is not one, and the nominal period would have to come
        // from CoreVideo instead.
        let mode = CGDisplay::new(id as CGDirectDisplayID).display_mode()?;
        period_from_hertz(mode.refresh_rate())
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use super::*;

    pub(super) fn refresh_rate(_display: &dyn PlatformDisplay) -> Option<Duration> {
        None
    }
}
