# AGENTS.md

This file provides shared guidance for coding agents working in this repository.
Keep project rules here; `CLAUDE.md` imports this file for Claude Code.

## This Fork

aislopware/gpui-kit is onicode's component layer (see the fork note at the top
of `README.md`). Its rulings override anything below that disagrees:

- Apple only (owner ruling, 2026-11-07): macOS on Apple silicon and iOS, 27.2
  and up. Add no `cfg` for Linux, Windows, FreeBSD, Android or wasm, and no
  dependency only those need; `cfg(target_os = "macos")` and
  `cfg(target_os = "ios")` are the only platform split.
- GPUI is aislopware/gpui-fast by git, at the rev onicode pins in its root
  `Cargo.toml`. There is no `gpui-pre` alias, no `gpui-fast` feature and no
  `[patch]`; bump the rev in the workspace `Cargo.toml` together with onicode's.
- No backward compatibility: replace and delete the old API, no shims.
- Nothing is published to crates.io; onicode depends on this repository by git.
- Upstream merges keep this fork's deletions (`crates/story-web`, `crates/shell`,
  `crates/webview`, `website/`): resolve a modify/delete conflict by keeping the
  deletion.

## Canonical Design and Coding Guides

Before changing UI, interaction, interface language, layout, styling,
components, or application architecture, read and follow the repository's
canonical guides:

- [Design Guides](skills/gpui-kit-design-guides/references/design-guides.md)
- [Coding Guides](skills/gpui-kit/references/coding-guides.md)

These guides are requirements, not optional inspiration. Do not copy generic
web conventions, infer a design system from one existing screen, or add a
control merely because the underlying feature exists. Preserve the documented
task hierarchy, interaction promise, desktop conventions, spacing, alignment,
theme tokens, component boundaries, naming, and crate architecture. Review the
finished work against both guides before considering it complete.

For Chinese documentation and UI, apply the terminology rules in Design Guides.
Keep established framework, component, and API names in their canonical English
form when translation would reduce precision; write the surrounding Chinese as
natural Chinese rather than word-for-word translation.

## Project Overview

GPUI Kit is a Rust application framework built on GPUI. It ships as two crates: `gpui-base` (unstyled behavior and infrastructure) and `gpui-component` (GPUI Component, the styled component library with 60+ UI components, inspired by macOS controls and combined with shadcn/ui design). Applications depend on the umbrella crate `gpui-kit` (`crates/kit`), which depends on GPUI and puts it at its root (`use gpui_kit::*;`) with `gpui_kit::platform`, `gpui_kit::base`, `gpui_kit::component` and `gpui_kit::assets` reachable by name, so they never list GPUI itself.

This is a Rust workspace project with the following main crates:

- `crates/kit` - Umbrella crate applications depend on (published as `gpui-kit`)
- `crates/component` - Core UI component library (published as `gpui-component`)
- `crates/story` - Gallery application for showcasing and testing components (macOS)
- `crates/component-macros` - Procedural macros (`IntoPlot` derive)
- `crates/assets` - Static assets
- `crates/fps` - Realtime FPS, frame time, CPU, GPU and memory HUD
- `examples/` - Various example applications

## Common Commands

### Development and Testing

```bash
# Run Story Gallery (component showcase application)
cargo run

# Run individual examples
cargo run --example hello_world
cargo run --example table

# Build the project
cargo build

# Lint check
cargo clippy -- --deny warnings

# Format check
cargo fmt --check

# Spell check
typos

# Check for unused dependencies
cargo machete
```

### Testing

**Note**: Per user configuration, tests do not need to be run.

For pure UI visual or sizing adjustments, do not add automated tests solely to
assert presentation dimensions. Add tests when the change affects behavior,
interaction, data flow, or prevents a meaningful regression.

```bash
# Run all tests
cargo test --all

# Run tests for a specific crate
cargo test -p gpui-component

# Run doc tests
cargo test -p gpui-component --doc
```

### Performance Profiling

```bash
# View FPS on macOS (using Metal HUD)
MTL_HUD_ENABLED=1 cargo run

# Profile performance using samply
samply record cargo run
```

## Core Architecture

### Architecture Refactoring Constraints

The implemented foundation architecture is documented in
`docs/ARCHITECTURE.md`, with styling and motion rules in
`docs/STYLING-AND-MOTION.md`. Preserve these constraints when designing or
implementing this architecture:

- Do not modify `gpui-base` unless the user explicitly requests a Base-layer
  change. By default, implement component behavior and visual styling in
  `crates/component` or the application layer.

- Keep GPUI Kit as the ecosystem and product brand; `gpui-component` is its
  styled component layer, alongside `gpui-base`.
- Name the foundation crate `gpui-base`.
- Follow the ownership boundary: the framework owns behavior and infrastructure;
  the application owns component source and visual style.
- Keep the base layer visually unopinionated. It may provide interaction behavior,
  accessibility, focus, overlay and popup infrastructure, positioning, animation,
  virtual lists, dock infrastructure, and semantic design tokens.
- Theme APIs must expose semantic tokens (colors, spacing, radius, typography, and
  shadows), not an ever-growing set of component-specific styling fields.
- Keep source distribution or registry tooling above the `gpui-base` seam; no
  registry or CLI crate is currently part of the workspace.
- Keep the public paths coherent, such as
  `use gpui_kit::component::button::Button;`, but carry no backward-compatibility
  shims: this fork replaces and deletes old API (see "This Fork").

### Component Initialization

Call `gpui_kit::init(cx)` before opening windows that use styled components.
It includes Base initialization and registers Component's window extension.

```rust
fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| MyView)
        })
        .expect("Failed to open window");
    });
}
```

Base-only fixtures use GPUI directly and must not depend on Kit or Component.
Applications and Kit tests use `gpui_kit::open_window` as their window entry point.

### Root View System

`gpui_base::Root` is the top-level view for every window created by
`gpui_kit::open_window`. The helper belongs only to Kit; Base supplies the Root
implementation. `component::Root` re-exports the Base type. Cargo features do not
select a different root type.

Base owns content and overlay hosting, keyboard navigation (Tab/Shift-Tab),
and selection copying. Explicit Component initialization registers per-window
state and presentation for dialogs, sheets, notifications, tooltips, menus,
touch selection, themes and window chrome. Initialize before creating windows.

The helper returns the window handle and application content entity. Its builder
must return content, not another Root. Overlay layers mount automatically; do not
call the removed `Root::render_*_layer` methods. In async contexts call the helper
inside `cx.update`. Tests should use the same helper and retain its returned
content entity when they need to inspect or update application state. Avoid
constructing `Root` directly in application and test startup code.

Quit/close actions, keyboard shortcuts and confirmation flows belong to the application.

### Theme System

- Uses `Theme` global singleton for theme configuration
- Supports light/dark mode switching
- Access theme via `ActiveTheme` trait: `cx.theme()`
- Theme configuration includes:
  - Colors (`ThemeColor`)
  - Syntax highlighting theme (`HighlightTheme`)
  - Font configuration (system font and monospace font)
  - UI parameters like border radius, shadows
  - Scrollbar display mode

### Dock System

Layout behavior lives in `crates/base/src/dock`; `crates/component/src/dock` is a
presentation skin (`DockSkin`) over it. See `docs/ARCHITECTURE.md`.

- **`LayoutTree`**: Pure-data layout tree, the single source of truth.
  - `NodeKind::Split` / `Tabs`: containers, addressed by `NodeId`
  - Panels are addressed by `PanelId`; the tree holds no entity handles
- **`DockArea`**: Owns the center and dock trees, reconciles them into a
  cache of container entities keyed by `NodeId`
- **`TabGroup`**: The `Tabs` container entity
- **`Panel`**: Split at the seam — `gpui_base::dock::Panel` for behavior,
  `gpui_component::dock::Panel` for presentation; a panel implements both
- **`PanelRegistry`**: Resolves a persisted `panel_name` back to a panel type

The Dock system supports:

- Panel drag-and-drop reordering
- Panel zoom
- Layout locking
- Layout serialization/restoration

### Input System

Text input system based on Rope data structure:

- **InputState**: Input state management
- **Rope**: Efficient text storage (from ropey crate)
- LSP integration support (diagnostics, completion, hover)
- Syntax highlighting support (Tree-sitter)
- Multiple input modes:
  - Regular input (`Input`)
  - Number input (`NumberInput`)
  - OTP input (`OtpInput`)

### Component Design Principles

1. **Stateless design**: Use `RenderOnce` trait, components should be stateless when possible
2. **Size system**: Supports `xs`, `sm`, `md` (default), `lg` sizes via `Sizable` trait.
3. **Mouse cursor**: Buttons use `default` cursor not `pointer` (desktop app convention), unless it's a link button
4. **Style system**: Provides CSS-like styling API via `Styled` trait and `ElementExt` extensions
5. **Base controls are no-style**: Base controls and parts do not install layout,
   positioning, colors, sizing, gaps, radius, borders, shadows, variants, or animation.
   Complete presentation belongs to `crates/component` or the application. The deliberate
   exception is the foundational Base Input frame, which provides only a semantic
   one-pixel input border and semantic radius baseline; UI/application layers own
   its background, sizing, padding, typography, adornments, and richer focus style.
6. **GPUI builder style**: Keep element construction as one fluent builder chain. Express
   conditions with `when`, `when_some`, `when_none`, and `map`; do not split a chain into a
   mutable temporary element followed by imperative reassignment when the builder API can
   express the same operation.
7. **No `pub` fields on public data types**: A public struct handed across the
   `gpui-base`/application seam — a state snapshot, capability set, render context, or
   option set — keeps its fields private, is constructed with a builder, and is read
   through methods. Adding a `pub` field is a breaking change; adding one behind a builder
   is not. Setters and readers must not collide: an all-boolean type names setters after
   the field and readers `is_<adjective>`/`has_<noun>`, never `can_`; a type with
   non-boolean fields prefixes every setter with `with_` and keeps the field name for
   readers. Value types whose fields are the definition and cannot grow (`Point`,
   `Selection`, `Edges`) are exempt. See the "Public Data Types Across the Seam" section
   of `docs/ARCHITECTURE.md`.
8. **`#[non_exhaustive]` on every public struct with `pub` fields**: Where public fields
   are deliberate — themes, settings records, design tokens, serialized schemas — mark
   the struct `#[non_exhaustive]` and implement `Default` (or a constructor), so a new
   field is not a breaking change. Callers outside the crate then build it with
   `Default::default()` and assign fields; struct literals, including
   `..Default::default()`, no longer compile there, so code in other workspace crates
   must use the same form. Apply this to every new type and to any existing type whose
   fields a change adds or alters; the value types exempt above stay exempt.
9. **Spell `Context` out**: Name a context type `ResizeHandleContext`, never `…Ctx`.
   `cx` is reserved for GPUI's `App`, `Context<T>`, and `AsyncApp`, so `ctx` for anything
   else reads as a competing context. A callback receiving both takes the GPUI one as `cx`
   and names the other after what it holds (`trigger`, `state`).

## Pull Request Reviews

Every PR review must present its conclusions in this order:

1. Explain the PR's purpose: the problem it addresses and the scope of its changes.
2. Assess whether the requirement and proposed approach are reasonable.
3. Report concrete findings, followed by validation performed and its limitations.

Do not start a review with findings alone; include the purpose and reasonableness
assessment even when no issues are found.

## Code Style

- Follow naming and organization patterns from existing code
- Reference macOS/Windows control API design for naming
- AI-generated code must be refactored to match project style
- Mark AI-generated portions when submitting PRs
- When creating a PR, inspect previous PR titles in the repository and match
  that style. Do not blindly use conventional prefixes like `fix:` or `feat:`
  unless the existing PR title style uses them.
- When a PR adds, changes or removes public API in any crate, list every item
  under a `## Public API` section of the description, grouped by crate, with its
  signature and one line on its purpose (JavaScript methods and TypeScript
  declarations included). Changes to existing items also go under
  `## Breaking Changes` with `diff` blocks showing the old and new usage. See
  PR #2691 and the "Describe public API changes" section of `CONTRIBUTING.md`.
- Avoid `Kind` as a type-name suffix. It says an enum classifies something
  without saying what it classifies, and carries no meaning a reader could not
  already infer from `enum`. Name the type after what its variants _are_
  instead. Keep `Kind` only when no honest name covers the variant set —
  `NodeKind`'s variants straddle two levels (`Split` is an interior node,
  `Tabs` is a leaf), and every domain word for the leaf level
  (`Pane`, most of all) would misdescribe `Split`; a vaguer name is better
  than a precise wrong one. Prefer confining such a type to `pub(crate)`.
  This governs new code; existing `Kind` names are not a rewrite target on
  their own, and names owned by external crates (`CodeActionKind`,
  `CompletionItemKind`, `WindowKind`) keep their upstream spelling.

## Icon System

The `Icon` element does not include SVG files by default. You need to:

- Use [Lucide](https://lucide.dev) or other icon libraries
- Name SVG files according to the `IconName` enum definition (located in `crates/component/src/icon.rs`)

## Dependencies

- GPUI: aislopware/gpui-fast by git, at the rev onicode pins
- Tree-sitter: For syntax highlighting
- Ropey: Rope data structure for text, and `RopeExt` trait with more features.
- Markdown rendering: `markdown` crate
- HTML rendering: `html5ever` (basic support)
- Charts: Built-in chart components
- LSP: `ls-types` crate

## Release Requirements

Nothing is published. Onicode depends on a rev of this repository's `main`;
moving GPUI means bumping the gpui-fast rev in the workspace `Cargo.toml` to
the one onicode pins, refreshing Cargo.lock, and adapting the kit to any API
change in the same commit.

## Text

The kit's visible copy and accessibility labels are English, written inline where they are shown;
there is no localization layer. Keep the item's own label when naming a control that acts on it.

## Documentation

- The documentation site (`website/`) is deleted in this fork. The Design and
  Coding Guides live at `skills/gpui-kit-design-guides/references/design-guides.md`
  and `skills/gpui-kit/references/coding-guides.md`; edit them there.
- `docs/` holds internal architecture specifications (RFC, migration status, reviews).
  These are single-language; see `docs/README.md`.

## Platform Support

- macOS 27.2 and up on Apple silicon (`aarch64-apple-darwin`)
- iOS 27.2 and up (`aarch64-apple-ios`, `aarch64-apple-ios-sim`)

CI runs the test suite on macOS and checks the library crates for iOS.

## Skills Reference

This project provides skills for coding agents to assist with common development tasks:

- **gpui-kit** (`skills/gpui-kit/SKILL.md`) - Building applications on the `gpui-kit` crate: setup, component catalog, stateless/stateful patterns, theming, GPUI mechanics (actions, async, contexts, custom elements, entities, events, focus, global state, layout, `ElementId`, testing), and the normative Coding Guides
- **gpui-kit-design-guides** (`skills/gpui-kit-design-guides/SKILL.md`) - The normative Design Guides; load before any UI, layout, interaction, or interface-copy work

When working on tasks related to these areas, read the appropriate `SKILL.md`
for specialized guidance and patterns.

## Shared Workflows

- [Release notes](.agents/commands/release-notes.md) - Write release notes from a
  version tag or a pasted "What's Changed" list. Claude Code exposes this workflow
  through `/release-notes`; other agents can read and follow the shared file.

## Testing Guidelines

See `.agents/COMPONENT_TEST_RULES.md` for detailed testing principles:

- **Simplicity First**: Focus on complex logic and core functionality, avoid excessive simple tests
- **Builder Pattern Testing**: Every component should have a `test_*_builder` test covering the builder pattern
- **Complex Logic Testing**: Test conditional branching, state transitions, and edge cases
