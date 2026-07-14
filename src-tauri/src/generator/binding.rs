//! Clone-binding precedence (item-08).
//!
//! Deciding which reference clip drives a speaker's clone is a DETERMINISTIC choice
//! over three tiers, highest first:
//!
//!   1. `override` - an explicit per-NPC binding the user set for THIS speaker.
//!   2. `default`  - the factual/archetype default: the speaker's own approved clip.
//!   3. `generic`  - an optional fallback clip shared across unbound speakers.
//!
//! The resolution is pure (a set of available candidates -> a chosen one), so it is
//! unit-tested in isolation; the DB lookups that gather the candidates live in the
//! command/orchestration layer. The chosen tier is persisted as `clone.binding_source`
//! (see `db::generation::upsert_clone`).

use crate::models::BindingSource;

/// A candidate reference clip for a binding, tagged with the tier it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingCandidate {
    /// The `reference_sample.id` backing this candidate.
    pub sample_id: i64,
    /// The on-disk LOCAL derivative path (validated separately by `clone::validate`).
    pub derivative_path: String,
    /// Which tier this candidate satisfies.
    pub source: BindingSource,
}

/// The available candidates for a speaker, one Option per tier. Any/all may be
/// absent; `choose` applies the precedence.
#[derive(Debug, Clone, Default)]
pub struct BindingInputs {
    pub override_clip: Option<BindingCandidate>,
    pub default_clip: Option<BindingCandidate>,
    pub generic_clip: Option<BindingCandidate>,
}

/// Apply the precedence override -> default -> generic, returning the winning
/// candidate (already tagged with its tier) or `None` when no tier is available.
pub fn choose(inputs: &BindingInputs) -> Option<BindingCandidate> {
    inputs
        .override_clip
        .clone()
        .or_else(|| inputs.default_clip.clone())
        .or_else(|| inputs.generic_clip.clone())
}

/// A speaker's demographic IDS bytes used for fallback matching (raw, edition-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Demographics {
    pub sex: i64,
    pub creature_category: i64,
    pub race: i64,
    pub class: i64,
}

/// A donor candidate: the donor speaker id, its approved sample id + derivative path,
/// and the donor's demographics.
#[derive(Debug, Clone)]
pub struct DonorCandidate {
    pub speaker_id: i64,
    pub sample_id: i64,
    pub derivative_path: String,
    pub demo: Demographics,
}

/// Which attributes a chosen donor matched (for the UI "matched: ..." label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemographicMatch {
    pub sex: bool,
    pub creature_category: bool,
    pub race: bool,
    pub class: bool,
}

/// Tiered score: sex=8, creature_category=4, race=2, class=1. A higher-priority match
/// strictly dominates any lower-priority combination (8 > 4+2+1). Determinism: on a
/// score tie, the lowest donor `speaker_id` wins (pool is ordered by id).
pub fn best_donor<'a>(
    target: &Demographics,
    pool: &'a [DonorCandidate],
) -> Option<(&'a DonorCandidate, DemographicMatch)> {
    pool.iter()
        .map(|d| {
            let m = DemographicMatch {
                sex: d.demo.sex == target.sex,
                creature_category: d.demo.creature_category == target.creature_category,
                race: d.demo.race == target.race,
                class: d.demo.class == target.class,
            };
            let score = (m.sex as i32) * 8
                + (m.creature_category as i32) * 4
                + (m.race as i32) * 2
                + (m.class as i32);
            (d, m, score)
        })
        // max_by prefers the LAST max on ties; pool is ascending by id, so negate id
        // to make the FIRST (lowest) id win the tie.
        .max_by_key(|(d, _, score)| (*score, -d.speaker_id))
        .map(|(d, m, _)| (d, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, source: BindingSource) -> BindingCandidate {
        BindingCandidate {
            sample_id: id,
            derivative_path: format!("/ws/{id}.wav"),
            source,
        }
    }

    #[test]
    fn override_wins_over_everything() {
        let inputs = BindingInputs {
            override_clip: Some(cand(1, BindingSource::Override)),
            default_clip: Some(cand(2, BindingSource::Default)),
            generic_clip: Some(cand(3, BindingSource::Generic)),
        };
        let chosen = choose(&inputs).unwrap();
        assert_eq!(chosen.sample_id, 1);
        assert_eq!(chosen.source, BindingSource::Override);
    }

    #[test]
    fn default_wins_when_no_override() {
        let inputs = BindingInputs {
            override_clip: None,
            default_clip: Some(cand(2, BindingSource::Default)),
            generic_clip: Some(cand(3, BindingSource::Generic)),
        };
        assert_eq!(choose(&inputs).unwrap().source, BindingSource::Default);
    }

    #[test]
    fn generic_is_the_last_resort() {
        let inputs = BindingInputs {
            override_clip: None,
            default_clip: None,
            generic_clip: Some(cand(3, BindingSource::Generic)),
        };
        assert_eq!(choose(&inputs).unwrap().source, BindingSource::Generic);
    }

    #[test]
    fn none_when_no_tier_available() {
        assert!(choose(&BindingInputs::default()).is_none());
    }

    fn donor(id: i64, sex: i64, cat: i64, race: i64, class: i64) -> DonorCandidate {
        DonorCandidate {
            speaker_id: id,
            sample_id: id * 10,
            derivative_path: format!("/ws/{id}.wav"),
            demo: Demographics {
                sex,
                creature_category: cat,
                race,
                class,
            },
        }
    }

    #[test]
    fn best_donor_prefers_sex_over_everything() {
        let target = Demographics { sex: 1, creature_category: 0, race: 0, class: 0 };
        // A matches sex only; B matches race+class but not sex.
        let a = donor(1, 1, 9, 9, 9);
        let b = donor(2, 2, 0, 0, 0);
        let pool = vec![a, b];
        let (chosen, _m) = best_donor(&target, &pool).unwrap();
        assert_eq!(chosen.speaker_id, 1);
    }

    #[test]
    fn best_donor_uses_full_priority_order() {
        let target = Demographics { sex: 1, creature_category: 5, race: 7, class: 8 };
        // Both match sex; A also matches creature_category, B matches race+class.
        let a = donor(1, 1, 5, 0, 0);
        let b = donor(2, 1, 0, 7, 8);
        let pool = vec![a, b];
        let (chosen, _m) = best_donor(&target, &pool).unwrap();
        assert_eq!(chosen.speaker_id, 1);
    }

    #[test]
    fn best_donor_falls_back_to_lowest_id_on_total_mismatch() {
        let target = Demographics { sex: 1, creature_category: 1, race: 1, class: 1 };
        // Nothing matches; lowest-id donor is returned (never None for a non-empty pool).
        let pool = vec![donor(5, 9, 9, 9, 9), donor(3, 8, 8, 8, 8), donor(4, 7, 7, 7, 7)];
        let (chosen, m) = best_donor(&target, &pool).unwrap();
        assert_eq!(chosen.speaker_id, 3);
        assert_eq!(m, DemographicMatch { sex: false, creature_category: false, race: false, class: false });
    }

    #[test]
    fn best_donor_none_for_empty_pool() {
        let target = Demographics { sex: 1, creature_category: 1, race: 1, class: 1 };
        assert!(best_donor(&target, &[]).is_none());
    }

    #[test]
    fn best_donor_match_flags_are_reported() {
        let target = Demographics { sex: 1, creature_category: 2, race: 3, class: 4 };
        let pool = vec![donor(1, 1, 2, 9, 4)];
        let (_chosen, m) = best_donor(&target, &pool).unwrap();
        assert_eq!(
            m,
            DemographicMatch { sex: true, creature_category: true, race: false, class: true }
        );
    }
}
