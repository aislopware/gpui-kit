# Architecture

These documents describe the current architecture implemented by `gpui-base`
and the crates built directly on it. They are maintained as durable references
rather than project-progress logs.

- [Architecture](ARCHITECTURE.md) explains the crate boundaries, ownership model,
  component taxonomy, state flow, overlay system, and native integration.
- [Styling and Motion](STYLING-AND-MOTION.md) explains semantic tokens, typed
  state styles, application-owned presentation, and animation primitives.

For component-level APIs and runnable examples, see the rustdoc of each crate
and `crates/base/examples`.
