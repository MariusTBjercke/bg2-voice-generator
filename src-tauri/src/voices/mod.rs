//! Voice bindings + profiles: which cloned/reference voice each speaker (or
//! race/gender pair) is generated with, resolved per generation run.
//!
//! `harvest` (item-07) finds usable original voice-reference clips per speaker,
//! decodes them into local derivatives, and scores them for auditioning. See
//! `docs/adr/0003-repo-module-layout.md`.

pub mod harvest;
