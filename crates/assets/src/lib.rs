//! Bundled Lucide icons shared by GPUI Kit presentation and behavior layers.
//!
//! [`IconName`] and [`IconNamed`] do not depend on GPUI Component. Register
//! [`Assets`] for the default component bundle, or [`AllAssets`] for the full
//! catalog. Applications can provide their own source for additional icons.

mod icon;
pub use icon::{IconName, IconNamed};

/// Embed application assets for GPUI Component.
///
/// This assets provides icons svg files for [IconName].
///
/// ## Usage
///
/// ```rust,no_run
/// use gpui_kit_assets::Assets;
///
/// let app = gpui_platform::application().with_assets(Assets);
/// ```
///
/// Icons are embedded in the binary with RustEmbed.
mod native_assets;

pub use native_assets::{AllAssets, Assets};

// Public only so exported macros can resolve dependencies in downstream crates,
// including applications that depend solely on the gpui-kit umbrella crate.
#[doc(hidden)]
pub mod __private {
    pub use crate::icon::embedded::*;
    pub use gpui::{AssetSource, Result, SharedString};
}

/// Embed only the selected icons in an application.
///
/// ```ignore
/// use gpui_kit::assets::{icon_assets, IconName};
/// icon_assets!(AppAssets, [Search, Check]);
/// // gpui_kit::application().with_assets(AppAssets)
/// ```
///
/// Unlisted paths return `Ok(None)`. The selected SVG bytes are borrowed from
/// static storage; loading them does not copy them or build a runtime cache.
/// This source contains only the selected icons. Applications using components
/// can compose it with `Assets` to retain their default icons.
#[macro_export]
macro_rules! icon_assets {
    ($vis:vis $name:ident, [$($icon:ident),* $(,)?]) => {
        #[derive(Clone, Copy, Debug, Default)]
        $vis struct $name;

        impl $crate::__private::AssetSource for $name {
            fn load(&self, path: &str) -> $crate::__private::Result<Option<::std::borrow::Cow<'static, [u8]>>> {
                $(if path == $crate::__private::$icon.0 {
                    return Ok(Some(::std::borrow::Cow::Borrowed($crate::__private::$icon.1)));
                })*
                let _ = path;
                Ok(None)
            }

            fn list(&self, path: &str) -> $crate::__private::Result<::std::vec::Vec<$crate::__private::SharedString>> {
                let paths: &[&str] = &[$($crate::__private::$icon.0),*];
                Ok(paths.iter().copied().filter(|name| name.starts_with(path)).map(Into::into).collect())
            }
        }
    };
}
