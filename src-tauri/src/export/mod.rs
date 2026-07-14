//! Assemble a native, WeiDU-installed voice pack: generated Ogg streams carried as
//! BG2EE `.wav` resources plus the
//! generated `.tp2`/`.tra` installer, playable in the vanilla engine with no runtime
//! dependency on this app or EEex (see `docs/adr/0001-native-weidu-export.md` and
//! `docs/adr/0002-eeex-independence.md`).
//!
//! Pipeline (item-09):
//!   * [`resref`] - deterministic 8-char resref naming (collision-safe).
//!   * [`manifest`] - the shipped `manifest.json` + the shared `PackLine`/hash types.
//!   * [`plan`] - PURE eligibility filter (blocks every deferred category) + assembly.
//!   * [`tp2`] - PURE `.tp2`/`.tra` emitters (COPY-to-override + STRING_SET + guards).
//!   * [`docs`] - PURE install/verify/uninstall README.
//!   * [`build`] - IO: stage generated Ogg bytes as WAV resources and write the pack folder.
//!   * [`zip`] - IO: bundle the folder + WeiDU into a self-contained pack ZIP (item-10).
//!
//! Fingerprint capture lives in `crate::fingerprint`; the DB join + export record in
//! `crate::db::export`.

pub mod build;
pub mod docs;
pub mod manifest;
pub mod plan;
pub mod resref;
pub mod tp2;
pub mod zip;

pub use build::{write_pack, BuiltPack};
pub use manifest::Manifest;
pub use plan::{assemble, Candidate, PackPlan};
pub use zip::{zip_pack, ZippedPack};
