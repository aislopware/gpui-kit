//! Speech input: capture audio, recognize it and hand the text to the caller.
//!
//! [`SpeechState`] owns a session and [`SpeechButton`], [`SpeechIndicator`] and
//! [`SpeechWaveform`] render it. Recognition and audio capture are both replaceable:
//! implement [`SpeechRecognizer`] to use any speech service and [`AudioInput`]
//! to feed audio from anywhere.
//!
//! With the `speech` feature, a state without its own recognizer falls back to
//! the `SystemRecognizer` of macOS or Windows, and captures from the
//! `Microphone`. Linux has no system recognizer, so speech input works there
//! only with an application recognizer.

mod button;
mod indicator;
mod level;
mod recognizer;
mod state;
mod surface;
mod waveform;

#[cfg(feature = "speech")]
mod microphone;
#[cfg(feature = "speech")]
mod system;

use std::rc::Rc;

pub use button::SpeechButton;
pub use indicator::SpeechIndicator;
#[cfg(feature = "speech")]
pub use microphone::Microphone;
pub use recognizer::{
    AudioFormat, AudioInput, AudioSink, RecognitionSession, SpeechError, SpeechRecognizer,
    SpeechSink,
};
pub use state::{SpeechEvent, SpeechState, SpeechStatus};
#[cfg(feature = "speech")]
pub use system::SystemRecognizer;
pub use waveform::SpeechWaveform;

/// The input a [`SpeechState`] captures from unless told otherwise.
fn default_input() -> Option<Rc<dyn AudioInput>> {
    #[cfg(feature = "speech")]
    return Some(Rc::new(Microphone::default()));
    #[allow(unreachable_code)]
    None
}

/// The recognizer a [`SpeechState`] falls back to, if this platform has one.
fn system_recognizer() -> Option<Rc<dyn SpeechRecognizer>> {
    #[cfg(all(target_os = "macos", feature = "speech"))]
    return Some(Rc::new(SystemRecognizer::new()));
    #[allow(unreachable_code)]
    None
}
