//! Drive the local OmniVoice engine to synthesize the missing dialogue lines into
//! Ogg Vorbis clips (from temporary PCM renders), tracking per-line progress and
//! resumability (item-08).
//!
//! [`binding`] is the pure clone-binding precedence (override -> default -> generic);
//! [`clone`] validates a reference derivative against the OmniVoice input spec before
//! it can drive a clone; [`run`] orchestrates ONE line end to end, resumably. The DB
//! side (clone upsert, generation state) lives in `db::generation`; the engine client
//! in `tts`. See `docs/adr/0003-repo-module-layout.md`.

pub mod batch;
pub mod binding;
pub mod clone;
pub mod fanout;
pub mod metadata_binding;
pub mod reference;
pub mod run;

pub use batch::{
    generate_batch, plan_batches, resolve_limits, BatchLimits, DEFAULT_BATCH_SIZE,
    DEFAULT_CHAR_BUDGET,
};
pub use binding::{choose, BindingCandidate, BindingInputs};
pub use clone::{validate_file, ValidatedReference, REFERENCE_SAMPLE_RATE};
pub use run::{generate_line, output_path_for, LineJob, LineResult};
