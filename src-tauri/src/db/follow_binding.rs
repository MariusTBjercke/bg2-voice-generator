//! Live “follow character” voice bindings.
//!
//! A follower clone stores `binding_source = follow` and `follow_speaker_id`.
//! Generation and effective-binding reads resolve through that pointer (with cycle
//! detection) to the target’s current effective voice. Soft `voice_changed` compares
//! generation snapshots against that resolved voice.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::generation::clone_for_speaker;
use crate::error::AppError;
use crate::models::{BindingSource, Clone};

const MAX_FOLLOW_DEPTH: usize = 8;

/// SQL fragment: recursive CTE that yields one terminal voice row per origin speaker.
///
/// Columns: `origin_speaker_id`, `voice_profile_id`, `primary_sample_id`, `status`,
/// `resolved_speaker_id`. Used by completed-generation / export `voice_changed` checks.
pub const RESOLVED_VOICE_CTE: &str = r#"
WITH RECURSIVE follow_chain(
    origin_speaker_id, cur_speaker_id, binding_source, follow_speaker_id,
    voice_profile_id, primary_sample_id, status, depth
) AS (
    SELECT c.speaker_id, c.speaker_id, c.binding_source, c.follow_speaker_id,
           c.voice_profile_id, c.primary_sample_id, c.status, 0
    FROM clone c
    UNION ALL
    SELECT fc.origin_speaker_id, c2.speaker_id, c2.binding_source, c2.follow_speaker_id,
           c2.voice_profile_id, c2.primary_sample_id, c2.status, fc.depth + 1
    FROM follow_chain fc
    JOIN clone c2 ON c2.speaker_id = fc.follow_speaker_id
    WHERE fc.binding_source = 'follow'
      AND fc.follow_speaker_id IS NOT NULL
      AND fc.depth < 8
),
resolved_voice AS (
    SELECT origin_speaker_id, voice_profile_id, primary_sample_id, status,
           cur_speaker_id AS resolved_speaker_id
    FROM (
        SELECT origin_speaker_id, voice_profile_id, primary_sample_id, status,
               cur_speaker_id, depth,
               ROW_NUMBER() OVER (
                 PARTITION BY origin_speaker_id ORDER BY depth DESC
               ) AS rn
        FROM follow_chain fc
        WHERE fc.binding_source != 'follow'
           OR fc.follow_speaker_id IS NULL
           OR NOT EXISTS (SELECT 1 FROM clone cx WHERE cx.speaker_id = fc.follow_speaker_id)
           OR fc.depth >= 8
    )
    WHERE rn = 1
)
"#;

/// Compare generation snapshots to the resolved effective voice (profile or sample).
pub const VOICE_CHANGED_CASE: &str = r#"
CASE WHEN c.id IS NULL OR c.status != 'ready'
       OR g.render_settings_hash IS NULL
       OR rv.origin_speaker_id IS NULL OR rv.status != 'ready'
       OR (rv.voice_profile_id IS NOT NULL AND
           NOT (g.voice_profile_id_snapshot IS rv.voice_profile_id))
       OR (rv.voice_profile_id IS NULL AND (g.reference_sample_id IS NULL
           OR rv.primary_sample_id IS NULL
           OR g.reference_sample_id != rv.primary_sample_id))
     THEN 1 ELSE 0 END
"#;

/// Walk follow edges to the terminal non-follow clone. Returns the follower’s own
/// clone when it is not a follow binding. Errors if unbound or a cycle is detected.
pub fn resolve_effective_clone(conn: &Connection, speaker_id: i64) -> Result<Clone, AppError> {
    let mut seen = Vec::with_capacity(MAX_FOLLOW_DEPTH);
    let mut current_id = speaker_id;
    for _ in 0..=MAX_FOLLOW_DEPTH {
        if seen.contains(&current_id) {
            return Err(AppError::Other(format!(
                "follow cycle detected involving speaker {speaker_id}"
            )));
        }
        seen.push(current_id);
        let clone = clone_for_speaker(conn, current_id)?.ok_or_else(|| {
            AppError::Other(format!(
                "speaker {current_id} has no bound clone; bind it first"
            ))
        })?;
        if clone.binding_source != BindingSource::Follow {
            return Ok(clone);
        }
        let follow_id = clone.follow_speaker_id.ok_or_else(|| {
            AppError::Other(format!(
                "speaker {current_id} is marked follow but has no follow target"
            ))
        })?;
        current_id = follow_id;
    }
    Err(AppError::Other(format!(
        "follow chain for speaker {speaker_id} exceeds {MAX_FOLLOW_DEPTH} hops"
    )))
}

/// Like [`resolve_effective_clone`], but returns `None` when the speaker (or a hop)
/// is unbound instead of erroring.
pub fn try_resolve_effective_clone(
    conn: &Connection,
    speaker_id: i64,
) -> Result<Option<Clone>, AppError> {
    match resolve_effective_clone(conn, speaker_id) {
        Ok(c) => Ok(Some(c)),
        Err(AppError::Other(msg))
            if msg.contains("has no bound clone")
                || msg.contains("has no follow target")
                || msg.contains("follow cycle")
                || msg.contains("exceeds") =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// True if setting `follower` → `target` would create a cycle (including self).
pub fn would_create_follow_cycle(
    conn: &Connection,
    follower_id: i64,
    target_id: i64,
) -> Result<bool, AppError> {
    if follower_id == target_id {
        return Ok(true);
    }
    let mut seen = vec![follower_id];
    let mut current = target_id;
    for _ in 0..=MAX_FOLLOW_DEPTH {
        if seen.contains(&current) {
            return Ok(true);
        }
        seen.push(current);
        let Some(clone) = clone_for_speaker(conn, current)? else {
            return Ok(false);
        };
        if clone.binding_source != BindingSource::Follow {
            return Ok(false);
        }
        let Some(next) = clone.follow_speaker_id else {
            return Ok(false);
        };
        current = next;
    }
    Ok(true)
}

/// Point `speaker_id` (and optional display-group members) at `follow_speaker_id`.
///
/// When `identity_key` is set, fans out to that display group (Binding card).
/// Otherwise fans out to the operational identity group of `speaker_id`.
pub fn follow_speaker_voice(
    conn: &Connection,
    project_id: i64,
    speaker_id: i64,
    follow_speaker_id: i64,
    identity_key: Option<&str>,
) -> Result<usize, AppError> {
    let follower_project: i64 = conn.query_row(
        "SELECT project_id FROM speaker WHERE id=?1",
        [speaker_id],
        |r| r.get(0),
    )?;
    if follower_project != project_id {
        return Err(AppError::Other("speaker is outside this project".into()));
    }
    let target_project: i64 = conn
        .query_row(
            "SELECT project_id FROM speaker WHERE id=?1",
            [follow_speaker_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::Other(format!("unknown follow target {follow_speaker_id}")))?;
    if target_project != project_id {
        return Err(AppError::Other(
            "follow target is outside this project".into(),
        ));
    }
    if would_create_follow_cycle(conn, speaker_id, follow_speaker_id)? {
        return Err(AppError::Other(
            "cannot follow that character: it would create a follow cycle".into(),
        ));
    }

    let members = if let Some(key) = identity_key {
        crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, key)?
    } else {
        let identity = crate::db::speaker_groups::identity_key_for_speaker(conn, speaker_id)?;
        crate::db::speaker_groups::speaker_ids_in_group(conn, project_id, &identity)?
    };

    for member in &members {
        if *member == follow_speaker_id {
            return Err(AppError::Other(
                "cannot follow a character that is in the same binding group".into(),
            ));
        }
        if would_create_follow_cycle(conn, *member, follow_speaker_id)? {
            return Err(AppError::Other(
                "cannot follow that character: it would create a follow cycle".into(),
            ));
        }
        upsert_follow_clone(conn, *member, follow_speaker_id)?;
    }
    Ok(members.len())
}

fn upsert_follow_clone(
    conn: &Connection,
    speaker_id: i64,
    follow_speaker_id: i64,
) -> Result<i64, AppError> {
    if let Some(existing) = clone_for_speaker(conn, speaker_id)? {
        conn.execute("DELETE FROM clone_reference WHERE clone_id=?1", [existing.id])?;
        conn.execute(
            "UPDATE clone SET primary_sample_id=NULL, voice_profile_id=NULL, \
             follow_speaker_id=?2, binding_source='follow', status='ready' WHERE id=?1",
            params![existing.id, follow_speaker_id],
        )?;
        return Ok(existing.id);
    }
    conn.execute(
        "INSERT INTO clone (speaker_id, primary_sample_id, voice_profile_id, follow_speaker_id, \
         binding_source, status) VALUES (?1, NULL, NULL, ?2, 'follow', 'ready')",
        params![speaker_id, follow_speaker_id],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::generation::{
        completed_generations_for_project, mark_done, upsert_clone,
    };
    use crate::db::schema::run_migrations;
    use crate::models::{BindingSource, CloneStatus, OmniVoiceRenderSettings};

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project (game_root, edition, active_language, generator_version, created_at) \
             VALUES ('C:/game','bg2ee','en_US','0.1.0','t')",
            [],
        )
        .unwrap();
        conn
    }

    fn add_speaker(conn: &Connection, cre: &str) -> i64 {
        conn.execute(
            "INSERT INTO speaker (project_id, cre_resref, display_name, sex, race, class, \
             creature_category) VALUES (1,?1,?1,1,1,1,1)",
            [cre],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn add_sample(conn: &Connection, speaker_id: i64, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO reference_sample (speaker_id, source_strref, source_sound_resref, \
             decision, local_derivative_path) VALUES (?1,1,'snd','approved',?2)",
            params![speaker_id, path],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn add_line(conn: &Connection, speaker_id: i64, strref: i64) -> i64 {
        conn.execute(
            "INSERT INTO line (project_id, strref, text, original_text, kind, status, \
             is_voiced, has_tokens, speaker_id) \
             VALUES (1,?1,'hi','hi','state','ready',0,0,?2)",
            params![strref, speaker_id],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn resolve_walks_one_hop() {
        let conn = setup();
        let monk = add_speaker(&conn, "MONK");
        let ghost = add_speaker(&conn, "GHOSTMONK");
        let sample = add_sample(&conn, monk, "monk.wav");
        let clone_id = upsert_clone(&conn, monk, sample, BindingSource::Default).unwrap();
        crate::db::generation::set_clone_status(&conn, clone_id, CloneStatus::Ready).unwrap();
        follow_speaker_voice(&conn, 1, ghost, monk, None).unwrap();
        let resolved = resolve_effective_clone(&conn, ghost).unwrap();
        assert_eq!(resolved.speaker_id, monk);
        assert_eq!(resolved.primary_sample_id, Some(sample));
    }

    #[test]
    fn cycle_rejected() {
        let conn = setup();
        let a = add_speaker(&conn, "A");
        let b = add_speaker(&conn, "B");
        follow_speaker_voice(&conn, 1, a, b, None).unwrap();
        let err = follow_speaker_voice(&conn, 1, b, a, None).unwrap_err();
        assert!(err.to_string().contains("cycle"), "{err}");
    }

    #[test]
    fn target_rebind_marks_follower_voice_changed() {
        let conn = setup();
        let monk = add_speaker(&conn, "MONK");
        let ghost = add_speaker(&conn, "GHOSTMONK");
        let sample_a = add_sample(&conn, monk, "a.wav");
        let sample_b = add_sample(&conn, monk, "b.wav");
        let monk_clone = upsert_clone(&conn, monk, sample_a, BindingSource::Default).unwrap();
        crate::db::generation::set_clone_status(&conn, monk_clone, CloneStatus::Ready).unwrap();
        follow_speaker_voice(&conn, 1, ghost, monk, None).unwrap();
        let ghost_clone = clone_for_speaker(&conn, ghost).unwrap().unwrap();
        let line = add_line(&conn, ghost, 100);
        let gen = crate::db::generation::get_or_create_generation(&conn, line, ghost_clone.id)
            .unwrap();
        let resolved = resolve_effective_clone(&conn, ghost).unwrap();
        mark_done(
            &conn,
            gen.id,
            ghost_clone.id,
            resolved.primary_sample_id.unwrap(),
            BindingSource::Follow,
            "out.ogg",
            "{}",
            &OmniVoiceRenderSettings::default(),
            "ref",
            resolved.voice_profile_id,
        )
        .unwrap();
        let before = completed_generations_for_project(&conn, 1).unwrap();
        assert_eq!(before.len(), 1);
        assert!(!before[0].2, "fresh follow generation should not be voice_changed");

        let rebound = upsert_clone(&conn, monk, sample_b, BindingSource::Override).unwrap();
        crate::db::generation::set_clone_status(&conn, rebound, CloneStatus::Ready).unwrap();
        let after = completed_generations_for_project(&conn, 1).unwrap();
        assert!(after[0].2, "target rebind must mark follower lines voice_changed");
    }
}
