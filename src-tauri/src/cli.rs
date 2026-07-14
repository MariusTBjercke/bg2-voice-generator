//! Headless `bg2-synthesis` companion CLI used by external coding agents.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::models::{
    AgentRenderPreset, AgentRenderPresetState, AgentRenderPresetWriteResult,
    DictionaryMatchKind,
};

const HELP: &str = r#"bg2-synthesis — review and override generation-only dialogue text

USAGE:
  bg2-synthesis [--db <path>] <command> [options]

COMMANDS:
  catalog
  audit-corpus [--project <id>]
  list-flagged [--project <id>] [--limit <n>] [--after <line-id>] [--include-decided]
  auto-review-plain [--project <id>]
  list-untagged [--project <id>] [--limit <n>] [--after <line-id>] [--include-reviewed]
  show --line <id>
  tag --line <id> --text <synthesis-text>
  tag --batch <file|->
  clear --line <id>
  review --line <id>
  review --batch <file|->
  unreview --line <id>
  progress [--project <id>]
  audit [--project <id>]
  export --dir <path> [--project <id>] [--chunk-size <n>]
  import <file-or-directory>
  preset list
  preset show --line <id>
  preset set --line <id> --preset <name>
  preset set --batch <file|->
  preset clear --line <id>
  dict list [--enabled-only]
  dict show --id <id>
  dict add --find <text> --speak-as <text>
  dict set --id <id> [--find <text>] [--speak-as <text>] [--enabled <true|false>]
  dict remove --id <id>
  dict import --file <file|->
  dict export [--file <file|->]
  dict test --text <sentence>
  dict scan [--project <id>]

Overrides affect generated audio only. They never modify BG2 TLK text or exported subtitles.
Presets only change line pacing. They cannot render, audition, or accept audio candidates.
"#;

#[derive(Debug, Deserialize)]
struct TagInput {
    #[serde(alias = "id")]
    line: i64,
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportItem {
    id: i64,
    original: String,
    mapped: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    flags: Vec<crate::models::CorpusAuditFlag>,
}

#[derive(Debug, Deserialize)]
struct ImportItem {
    #[serde(alias = "line")]
    id: i64,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PresetInput {
    #[serde(alias = "id")]
    line: i64,
    preset: AgentRenderPreset,
}

#[derive(Debug, Serialize, Deserialize)]
struct DictionaryInput {
    find: String,
    speak_as: String,
    #[serde(default = "whole_word")]
    r#match: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn whole_word() -> String {
    "whole_word".into()
}

fn default_true() -> bool {
    true
}

fn value_after(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn integer_after<T: std::str::FromStr>(args: &[String], key: &str) -> Result<Option<T>, AppError> {
    value_after(args, key)
        .map(|value| {
            value
                .parse()
                .map_err(|_| AppError::Other(format!("{key} expects a number")))
        })
        .transpose()
}

fn default_db_path() -> Result<PathBuf, AppError> {
    if let Some(path) = std::env::var_os("BG2_SYNTHESIS_DB") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|p| p.join("Library").join("Application Support"));
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".local").join("share"))
        });
    base.map(|p| {
        p.join("com.bg2voicegen.desktop")
            .join(crate::db::DB_FILE_NAME)
    })
    .ok_or_else(|| {
        AppError::Other("cannot resolve app-data directory; pass --db or BG2_SYNTHESIS_DB".into())
    })
}

fn open_db(path: &Path) -> Result<Connection, AppError> {
    if !path.is_file() {
        return Err(AppError::Other(format!(
            "database does not exist: {}",
            path.display()
        )));
    }
    let conn = Connection::open(path)?;
    crate::db::tune_connection(&conn)?;
    crate::dictionary::ensure_default_rules(&conn)?;
    Ok(conn)
}

fn line_id(args: &[String]) -> Result<i64, AppError> {
    integer_after(args, "--line")?.ok_or_else(|| AppError::Other("missing --line <id>".into()))
}

fn read_json_source(source: &str) -> Result<String, AppError> {
    if source == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
        Ok(text)
    } else {
        Ok(fs::read_to_string(source)?)
    }
}

fn show(conn: &Connection, id: i64) -> Result<(), AppError> {
    let (project_id, strref, text): (i64, i64, String) = conn
        .query_row(
            "SELECT project_id,strref,text FROM line WHERE id=?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| AppError::Other(format!("line {id} not found")))?;
    let resolved = crate::synthesis::resolve_synthesis_text(conn, &text, true)?;
    println!("line: {id}");
    println!("project: {project_id}");
    println!("strref: {strref}");
    println!(
        "shared: {}",
        crate::synthesis::shared_line_count(conn, &text)?
    );
    println!("original: {text}");
    if !matches!(resolved.source, crate::models::SynthesisTextSource::Override) {
        println!("mapped: {}", resolved.text);
        for rule in &resolved.applied_rules {
            println!("dictionary: {} -> {}", rule.find_text, rule.speak_as);
        }
    }
    println!("synthesis: {}", resolved.text);
    println!("source: {:?}", resolved.source);
    let diagnostics: Option<String> = conn.query_row("SELECT diagnostics_json FROM generation WHERE line_id=?1", [id], |r| r.get(0)).optional()?.flatten();
    if let Some(json) = diagnostics {
        let diagnostics: crate::models::GenerationDiagnostics = serde_json::from_str(&json)?;
        println!("diagnostics: duration={:.2}s silence={:.0}% clipping={:.1}% flags={}", diagnostics.duration_secs, diagnostics.silence_fraction * 100.0, diagnostics.clipping_fraction * 100.0, diagnostics.flags.iter().map(|f| format!("{f:?}")).collect::<Vec<_>>().join(","));
    } else { println!("diagnostics: unavailable (render locally to measure)"); }
    Ok(())
}

fn list_untagged(conn: &Connection, args: &[String]) -> Result<(), AppError> {
    let project = integer_after(args, "--project")?;
    let limit = integer_after(args, "--limit")?
        .unwrap_or(500usize)
        .clamp(1, 10_000);
    let after = integer_after(args, "--after")?.unwrap_or(0i64);
    let rows = crate::synthesis::undecided_corpus(
        conn,
        project,
        after,
        limit,
        args.iter().any(|arg| arg == "--include-reviewed"),
    )?;
    for row in &rows {
        let flags = crate::synthesis_corpus_audit::audit_source_and_mapped_text(
            &row.text,
            &row.mapped_text,
            true,
        );
        let flags_suffix = if flags.is_empty() {
            String::new()
        } else {
            format!(
                "\n  flags:    {}",
                flags
                    .iter()
                    .map(|flag| format!("{flag:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!(
            "id={} project={} strref={} shared={}\n  original: {}\n  mapped:   {}{}",
            row.line_id, row.project_id, row.strref, row.shared_count, row.text, row.mapped_text,
            flags_suffix
        );
    }
    if let Some(last) = rows.last() {
        println!("next: --after {}", last.line_id);
    }
    println!("returned: {}", rows.len());
    Ok(())
}

fn tag_batch(conn: &Connection, source: &str) -> Result<(), AppError> {
    let items: Vec<TagInput> = serde_json::from_str(&read_json_source(source)?)?;
    for item in &items {
        crate::synthesis::write_override(conn, item.line, &item.text)?;
    }
    println!("tagged: {}", items.len());
    Ok(())
}

fn review_batch(conn: &Connection, source: &str) -> Result<(), AppError> {
    let value: serde_json::Value = serde_json::from_str(&read_json_source(source)?)?;
    let rows = value
        .as_array()
        .ok_or_else(|| AppError::Other("review batch must be a JSON array".into()))?;
    for row in rows {
        let id = row
            .as_i64()
            .or_else(|| row.get("line").and_then(|v| v.as_i64()))
            .or_else(|| row.get("id").and_then(|v| v.as_i64()))
            .ok_or_else(|| AppError::Other("review item needs a line id".into()))?;
        crate::synthesis::set_reviewed(conn, id, true)?;
    }
    println!("reviewed: {}", rows.len());
    Ok(())
}

fn export_corpus(conn: &Connection, args: &[String]) -> Result<(), AppError> {
    let dir = value_after(args, "--dir")
        .map(PathBuf::from)
        .ok_or_else(|| AppError::Other("export requires --dir <path>".into()))?;
    let project = integer_after(args, "--project")?;
    let chunk_size = integer_after(args, "--chunk-size")?
        .unwrap_or(500usize)
        .clamp(1, 10_000);
    fs::create_dir_all(&dir)?;
    let mut after = 0i64;
    let mut chunk = 1usize;
    let mut total = 0usize;
    loop {
        let rows = crate::synthesis::undecided_corpus(conn, project, after, chunk_size, false)?;
        if rows.is_empty() {
            break;
        }
        after = rows.last().map(|r| r.line_id).unwrap_or(after);
        let items: Vec<ExportItem> = rows
            .into_iter()
            .map(|row| ExportItem {
                id: row.line_id,
                original: row.text.clone(),
                mapped: row.mapped_text.clone(),
                text: None,
                flags: crate::synthesis_corpus_audit::audit_source_and_mapped_text(
                    &row.text,
                    &row.mapped_text,
                    true,
                ),
            })
            .collect();
        let path = dir.join(format!("synthesis-{chunk:04}.json"));
        fs::write(path, serde_json::to_string_pretty(&items)?)?;
        total += items.len();
        chunk += 1;
    }
    println!("exported: {total}");
    Ok(())
}

fn import_file(conn: &Connection, path: &Path) -> Result<usize, AppError> {
    let items: Vec<ImportItem> = serde_json::from_str(&fs::read_to_string(path)?)?;
    for item in &items {
        if let Some(text) = item.text.as_deref().filter(|text| !text.trim().is_empty()) {
            crate::synthesis::write_override(conn, item.id, text)?;
        } else {
            crate::synthesis::set_reviewed(conn, item.id, true)?;
        }
    }
    Ok(items.len())
}

fn import_path(conn: &Connection, path: &Path) -> Result<(), AppError> {
    let mut total = 0;
    if path.is_dir() {
        let mut files = fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect::<Vec<_>>();
        files.sort();
        for file in files {
            total += import_file(conn, &file)?;
        }
    } else {
        total = import_file(conn, path)?;
    }
    println!("imported: {total}");
    Ok(())
}

fn preset_name(preset: Option<AgentRenderPreset>) -> &'static str {
    match preset {
        Some(AgentRenderPreset::Inherit) => "inherit",
        Some(AgentRenderPreset::AutoPace) => "auto_pace",
        Some(AgentRenderPreset::Deliberate) => "deliberate",
        Some(AgentRenderPreset::Natural) => "natural",
        Some(AgentRenderPreset::Brisk) => "brisk",
        Some(AgentRenderPreset::VeryBrisk) => "very_brisk",
        None => "manual_ui_only",
    }
}

fn print_preset_state(state: &AgentRenderPresetState) {
    println!("line: {}", state.line_id);
    println!("effective_preset: {}", preset_name(state.preset));
    if state.has_manual_pacing {
        println!("diagnostic: line has manual pacing; agents cannot edit its raw value");
    }
    if state.has_manual_render_settings {
        println!("diagnostic: line has manual render settings; preset changes preserve them");
    }
}

fn clone_settings_for_line(
    conn: &Connection,
    line_id: i64,
) -> Result<crate::models::OmniVoiceRenderSettings, AppError> {
    let speaker_id: Option<i64> = conn
        .query_row("SELECT speaker_id FROM line WHERE id=?1", [line_id], |r| r.get(0))
        .optional()?
        .flatten();
    let speaker_id = speaker_id
        .ok_or_else(|| AppError::Other(format!("line {line_id} has no attributed speaker")))?;
    let clone = crate::db::generation::clone_for_speaker(conn, speaker_id)?.ok_or_else(|| {
        AppError::Other(format!("line {line_id} has no bound clone"))
    })?;
    crate::db::generation::render_settings_for_clone(&clone)
}

fn workspace_dir(db_path: &Path, project_id: i64) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("workspaces")
        .join(project_id.to_string())
}

fn remove_if_expected(recorded: Option<&str>, expected: &Path) {
    if recorded.is_some_and(|path| Path::new(path) == expected) {
        let _ = fs::remove_file(expected);
    }
}

fn set_preset(
    conn: &mut Connection,
    db_path: &Path,
    line_id: i64,
    preset: AgentRenderPreset,
) -> Result<AgentRenderPresetWriteResult, AppError> {
    let project_id: i64 = conn
        .query_row("SELECT project_id FROM line WHERE id=?1", [line_id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| AppError::Other(format!("line {line_id} not found")))?;
    let clone_settings = clone_settings_for_line(conn, line_id)?;
    let change = crate::db::generation::write_agent_render_preset(
        conn,
        line_id,
        preset,
        &clone_settings,
    )?;
    let workspace = workspace_dir(db_path, project_id);
    remove_if_expected(
        change.output_path.as_deref(),
        &crate::generator::run::output_path_for(&workspace, line_id),
    );
    remove_if_expected(
        change.candidate_path.as_deref(),
        &crate::generator::run::candidate_output_path_for(&workspace, line_id),
    );
    Ok(AgentRenderPresetWriteResult {
        state: crate::db::generation::agent_render_preset_state(conn, line_id)?,
        reset_generations: change.reset_generations,
        candidate_discarded: change.candidate_path.is_some(),
    })
}

fn set_preset_batch(conn: &mut Connection, db_path: &Path, source: &str) -> Result<(), AppError> {
    let items: Vec<PresetInput> = serde_json::from_str(&read_json_source(source)?)?;
    let mut applied = 0usize;
    let mut reset = 0usize;
    let mut discarded = 0usize;
    let mut failures = Vec::new();
    for item in items {
        match set_preset(conn, db_path, item.line, item.preset) {
            Ok(result) => {
                applied += 1;
                reset += result.reset_generations;
                discarded += usize::from(result.candidate_discarded);
            }
            Err(error) => failures.push(format!("line {}: {error}", item.line)),
        }
    }
    println!(
        "preset batch applied: {applied}; reset generations: {reset}; candidates discarded: {discarded}"
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "preset batch had {} failure(s); successful lines were kept:\n{}",
            failures.len(),
            failures.join("\n")
        )))
    }
}

fn preset_command(conn: &mut Connection, db_path: &Path, args: &[String]) -> Result<(), AppError> {
    let action = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| AppError::Other("preset requires list, show, set, or clear".into()))?;
    match action {
        "list" => {
            println!("inherit: use clone/application pacing; clears only the line pacing override");
            println!("auto_pace: let OmniVoice estimate pacing");
            println!("deliberate: slower named pacing");
            println!("natural: neutral named pacing");
            println!("brisk: faster named pacing");
            println!("very_brisk: fastest named pacing");
            Ok(())
        }
        "show" => {
            let id = line_id(args)?;
            print_preset_state(&crate::db::generation::agent_render_preset_state(conn, id)?);
            Ok(())
        }
        "set" if args.iter().any(|arg| arg == "--batch") => set_preset_batch(
            conn,
            db_path,
            &value_after(args, "--batch")
                .ok_or_else(|| AppError::Other("missing --batch source".into()))?,
        ),
        "set" => {
            let id = line_id(args)?;
            let preset = value_after(args, "--preset")
                .ok_or_else(|| AppError::Other("missing --preset <name>".into()))?
                .parse::<AgentRenderPreset>()
                .map_err(AppError::Other)?;
            let result = set_preset(conn, db_path, id, preset)?;
            print_preset_state(&result.state);
            println!("reset generations: {}", result.reset_generations);
            println!("candidate discarded: {}", result.candidate_discarded);
            Ok(())
        }
        "clear" => {
            let id = line_id(args)?;
            let result = set_preset(conn, db_path, id, AgentRenderPreset::Inherit)?;
            print_preset_state(&result.state);
            println!("reset generations: {}", result.reset_generations);
            println!("candidate discarded: {}", result.candidate_discarded);
            Ok(())
        }
        _ => Err(AppError::Other("preset requires list, show, set, or clear".into())),
    }
}

fn parse_bool(value: &str) -> Result<bool, AppError> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::Other(format!("expected true or false, got {value:?}"))),
    }
}

fn print_dictionary_rule(rule: &crate::models::DictionaryRule) {
    println!(
        "id={} default={} enabled={} match={}\n  find:     {}\n  speak_as: {}",
        rule.id,
        rule.is_default,
        rule.enabled,
        rule.match_kind.as_str(),
        rule.find_text,
        rule.speak_as
    );
}

fn dict_command(conn: &mut Connection, args: &[String]) -> Result<(), AppError> {
    let action = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| AppError::Other("dict requires list, show, add, set, remove, import, export, test, or scan".into()))?;
    match action {
        "list" => {
            let enabled_only = args.iter().any(|arg| arg == "--enabled-only");
            for rule in crate::dictionary::list_rules(conn)?
                .into_iter()
                .filter(|rule| !enabled_only || rule.enabled)
            {
                print_dictionary_rule(&rule);
            }
            Ok(())
        }
        "show" => {
            let id = integer_after(args, "--id")?
                .ok_or_else(|| AppError::Other("dict show requires --id <id>".into()))?;
            let rule = crate::dictionary::rule_by_id(conn, id)?
                .ok_or_else(|| AppError::Other(format!("dictionary rule {id} not found")))?;
            print_dictionary_rule(&rule);
            Ok(())
        }
        "add" => {
            let (find, speak_as) = crate::dictionary::validate_rule_text(
                &value_after(args, "--find")
                    .ok_or_else(|| AppError::Other("dict add requires --find <text>".into()))?,
                &value_after(args, "--speak-as")
                    .ok_or_else(|| AppError::Other("dict add requires --speak-as <text>".into()))?,
            )?;
            conn.execute(
                "INSERT INTO dictionary_rule(find_text,speak_as,match_kind,enabled,is_default,updated_at) \
                 VALUES(?1,?2,'whole_word',1,0,?3)",
                params![find, speak_as, chrono::Utc::now().to_rfc3339()],
            )?;
            let id = conn.last_insert_rowid();
            let reset = crate::dictionary::reset_completed_generations(conn)?;
            print_dictionary_rule(&crate::dictionary::rule_by_id(conn, id)?.unwrap());
            println!("reset generations: {reset}");
            Ok(())
        }
        "set" => {
            let id = integer_after(args, "--id")?
                .ok_or_else(|| AppError::Other("dict set requires --id <id>".into()))?;
            let existing = crate::dictionary::rule_by_id(conn, id)?
                .ok_or_else(|| AppError::Other(format!("dictionary rule {id} not found")))?;
            if existing.is_default
                && (value_after(args, "--find").is_some()
                    || value_after(args, "--speak-as").is_some())
            {
                return Err(AppError::Other(
                    "built-in rules may only be enabled or disabled".into(),
                ));
            }
            let find = value_after(args, "--find").unwrap_or(existing.find_text);
            let speak_as = value_after(args, "--speak-as").unwrap_or(existing.speak_as);
            let (find, speak_as) = crate::dictionary::validate_rule_text(&find, &speak_as)?;
            let enabled = value_after(args, "--enabled")
                .map(|value| parse_bool(&value))
                .transpose()?
                .unwrap_or(existing.enabled);
            conn.execute(
                "UPDATE dictionary_rule SET find_text=?1,speak_as=?2,enabled=?3,updated_at=?4 \
                 WHERE id=?5",
                params![find, speak_as, enabled, chrono::Utc::now().to_rfc3339(), id],
            )?;
            let reset = crate::dictionary::reset_completed_generations(conn)?;
            print_dictionary_rule(&crate::dictionary::rule_by_id(conn, id)?.unwrap());
            println!("reset generations: {reset}");
            Ok(())
        }
        "remove" => {
            let id = integer_after(args, "--id")?
                .ok_or_else(|| AppError::Other("dict remove requires --id <id>".into()))?;
            let existing = crate::dictionary::rule_by_id(conn, id)?
                .ok_or_else(|| AppError::Other(format!("dictionary rule {id} not found")))?;
            if existing.is_default {
                return Err(AppError::Other(
                    "built-in rules cannot be removed; disable them instead".into(),
                ));
            }
            conn.execute("DELETE FROM dictionary_rule WHERE id=?1", [id])?;
            let reset = crate::dictionary::reset_completed_generations(conn)?;
            println!("removed rule {id}; reset generations: {reset}");
            Ok(())
        }
        "import" => {
            let source = value_after(args, "--file")
                .ok_or_else(|| AppError::Other("dict import requires --file <file|->".into()))?;
            let inputs: Vec<DictionaryInput> = serde_json::from_str(&read_json_source(&source)?)?;
            let tx = conn.transaction()?;
            for input in &inputs {
                let kind = DictionaryMatchKind::parse(&input.r#match).map_err(AppError::Other)?;
                let (find, speak_as) =
                    crate::dictionary::validate_rule_text(&input.find, &input.speak_as)?;
                tx.execute(
                    "INSERT INTO dictionary_rule(find_text,speak_as,match_kind,enabled,is_default,updated_at) \
                     VALUES(?1,?2,?3,?4,0,?5) \
                     ON CONFLICT(lower(find_text),match_kind,is_default) DO UPDATE SET \
                     speak_as=excluded.speak_as,enabled=excluded.enabled,updated_at=excluded.updated_at \
                     WHERE dictionary_rule.is_default=0",
                    params![
                        find,
                        speak_as,
                        kind.as_str(),
                        input.enabled,
                        chrono::Utc::now().to_rfc3339()
                    ],
                )?;
            }
            let reset = crate::dictionary::reset_completed_generations(&tx)?;
            tx.commit()?;
            println!("imported: {}; reset generations: {reset}", inputs.len());
            Ok(())
        }
        "export" => {
            let inputs: Vec<_> = crate::dictionary::list_rules(conn)?
                .into_iter()
                .map(|rule| DictionaryInput {
                    find: rule.find_text,
                    speak_as: rule.speak_as,
                    r#match: rule.match_kind.as_str().into(),
                    enabled: rule.enabled,
                })
                .collect();
            let json = serde_json::to_string_pretty(&inputs)?;
            match value_after(args, "--file").as_deref() {
                Some("-") | None => println!("{json}"),
                Some(path) => fs::write(path, format!("{json}\n"))?,
            }
            Ok(())
        }
        "test" => {
            let text = value_after(args, "--text")
                .ok_or_else(|| AppError::Other("dict test requires --text <sentence>".into()))?;
            let preview = crate::dictionary::preview_dictionary(
                &text,
                &crate::dictionary::load_enabled_rules(conn)?,
            );
            println!("before: {}", preview.before);
            println!("after:  {}", preview.after);
            for rule in preview.applied_rules {
                println!("applied: {} -> {}", rule.find_text, rule.speak_as);
            }
            Ok(())
        }
        "scan" => {
            let project = integer_after::<i64>(args, "--project")?;
            let rules = crate::dictionary::load_enabled_rules(conn)?;
            let mut stmt = conn.prepare(
                "SELECT DISTINCT text FROM line WHERE (?1 IS NULL OR project_id=?1) ORDER BY id",
            )?;
            let texts = stmt
                .query_map([project], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for rule in rules {
                let matches = texts
                    .iter()
                    .filter(|text| {
                        !crate::dictionary::apply_dictionary_rules(text, std::slice::from_ref(&rule))
                            .1
                            .is_empty()
                    })
                    .count();
                if matches > 0 {
                    println!("id={} matches={} find={}", rule.id, matches, rule.find_text);
                }
            }
            Ok(())
        }
        _ => Err(AppError::Other(
            "dict requires list, show, add, set, remove, import, export, test, or scan".into(),
        )),
    }
}

fn execute(conn: &mut Connection, db_path: &Path, command: &str, args: &[String]) -> Result<(), AppError> {
    match command {
        "catalog" => {
            for tag in crate::omnivoice_tags::SUPPORTED_INLINE_TAGS {
                println!("{tag}");
            }
            println!();
            println!("Stage-direction mapper (*...* cues):");
            for (cue, tag) in [
                ("sigh/sighs", "[sigh]"),
                ("laugh/chuckle/giggle", "[laughter]"),
                ("gasp/gasps", "[surprise-ah]"),
                ("surprise/surprised", "[surprise-oh]"),
                ("hmm/hmph/grumble", "[dissatisfaction-hnn]"),
            ] {
                println!("  {cue} -> {tag}");
            }
            println!();
            println!("TTS-unfriendly spelling rewrites (flagged lines only):");
            println!("  B-b-b-but -> But");
            println!("  Nooooo -> No");
            println!("  wwaaAAAAHHHH -> Wah");
            println!("  Preserve every ordinary word. Add only tags listed above.");
            Ok(())
        }
        "list-untagged" => list_untagged(conn, args),
        "audit-corpus" => {
            let project = integer_after(args, "--project")?.ok_or_else(|| {
                AppError::Other("audit-corpus requires --project <id>".into())
            })?;
            let summary = crate::synthesis::corpus_audit_summary(conn, project, true)?;
            println!("unique={}", summary.unique_strings);
            println!("plain_ok={}", summary.plain_ok);
            println!("mapped_ok={}", summary.mapped_ok);
            println!("stripped_unknown_cue={}", summary.stripped_unknown_cue);
            println!("unterminated_asterisk={}", summary.unterminated_asterisk);
            println!("placement_candidate={}", summary.placement_candidate);
            println!("interpretive_candidate={}", summary.interpretive_candidate);
            println!(
                "tts_unfriendly_spelling={}",
                summary.tts_unfriendly_spelling
            );
            println!("non_speakable={}", summary.non_speakable);
            println!("flagged_undecided={}", summary.flagged_undecided);
            println!("stale_reviews_cleared={}", summary.stale_reviews_cleared);
            Ok(())
        }
        "list-flagged" => {
            let project = integer_after(args, "--project")?.ok_or_else(|| {
                AppError::Other("list-flagged requires --project <id>".into())
            })?;
            let limit = integer_after(args, "--limit")?
                .unwrap_or(500usize)
                .clamp(1, 10_000);
            let after = integer_after(args, "--after")?.unwrap_or(0i64);
            let undecided_only = !args.iter().any(|arg| arg == "--include-decided");
            let result = crate::synthesis::list_flagged(
                conn,
                project,
                after,
                limit,
                true,
                undecided_only,
            )?;
            for row in &result.rows {
                println!(
                    "id={} strref={} shared={} flags={:?}\n  original: {}\n  mapped:   {}",
                    row.line_id,
                    row.strref,
                    row.shared_line_count,
                    row.flags,
                    row.source_text,
                    row.mapped_text
                );
            }
            if let Some(next) = result.next_after {
                println!("next: --after {next}");
            }
            println!("returned: {}", result.rows.len());
            Ok(())
        }
        "auto-review-plain" => {
            let project = integer_after(args, "--project")?.ok_or_else(|| {
                AppError::Other("auto-review-plain requires --project <id>".into())
            })?;
            let result = crate::synthesis::auto_review_plain(conn, project, true)?;
            println!("reviewed: {}", result.reviewed);
            Ok(())
        }
        "show" => show(conn, line_id(args)?),
        "tag" if args.iter().any(|arg| arg == "--batch") => tag_batch(
            conn,
            &value_after(args, "--batch")
                .ok_or_else(|| AppError::Other("missing --batch source".into()))?,
        ),
        "tag" => {
            let id = line_id(args)?;
            let text = value_after(args, "--text")
                .ok_or_else(|| AppError::Other("missing --text <synthesis-text>".into()))?;
            let result = crate::synthesis::write_override(conn, id, &text)?;
            println!(
                "tagged line {id}; reset generations: {}",
                result.reset_generations
            );
            Ok(())
        }
        "clear" => {
            let id = line_id(args)?;
            let result = crate::synthesis::clear_override(conn, id)?;
            println!(
                "cleared line {id}; reset generations: {}",
                result.reset_generations
            );
            Ok(())
        }
        "review" if args.iter().any(|arg| arg == "--batch") => review_batch(
            conn,
            &value_after(args, "--batch")
                .ok_or_else(|| AppError::Other("missing --batch source".into()))?,
        ),
        "review" | "unreview" => {
            let id = line_id(args)?;
            crate::synthesis::set_reviewed(conn, id, command == "review")?;
            println!("{command}ed line {id}");
            Ok(())
        }
        "progress" => {
            let summary =
                crate::synthesis::tagging_summary(conn, integer_after(args, "--project")?)?;
            println!(
                "unique={} overridden={} reviewed={} remaining={}",
                summary.unique_strings, summary.overridden, summary.reviewed, summary.remaining
            );
            Ok(())
        }
        "audit" => {
            let issues = crate::synthesis_validation::audit_project_overrides(
                conn,
                integer_after(args, "--project")?,
            )?;
            println!("suspicious: {}", issues.len());
            for issue in &issues {
                println!(
                    "line={} hash={} reason={}\n  synthesis: {}",
                    issue.line_id, issue.text_hash, issue.reason, issue.synthesis_text
                );
            }
            Ok(())
        }
        "export" => export_corpus(conn, args),
        "import" => {
            let path = args
                .first()
                .map(PathBuf::from)
                .ok_or_else(|| AppError::Other("import requires a file or directory".into()))?;
            import_path(conn, &path)
        }
        "preset" => preset_command(conn, db_path, args),
        "dict" => dict_command(conn, args),
        _ => Err(AppError::Other(format!("unknown command: {command}"))),
    }
}

pub fn run(mut args: Vec<String>) -> ExitCode {
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let db_arg = value_after(&args, "--db");
    if let Some(index) = args.iter().position(|arg| arg == "--db") {
        if index + 1 >= args.len() {
            eprintln!("bg2-synthesis: --db requires a path\n\n{HELP}");
            return ExitCode::from(2);
        }
        args.drain(index..=index + 1);
    }
    let Some(command) = args.first().cloned() else {
        eprintln!("bg2-synthesis: missing command\n\n{HELP}");
        return ExitCode::from(2);
    };
    args.remove(0);
    let path = match db_arg
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(default_db_path)
    {
        Ok(path) => path,
        Err(error) => {
            eprintln!("bg2-synthesis: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut conn = match open_db(&path) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("bg2-synthesis: {error}");
            return ExitCode::FAILURE;
        }
    };
    match execute(&mut conn, &path, &command, &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bg2-synthesis: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn preset_db() -> (tempfile::TempDir, PathBuf, Connection, i64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bg2vg.db");
        let mut conn = Connection::open(&path).unwrap();
        schema::run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO project(game_root,edition,active_language,generator_version,created_at) VALUES('r','BG2EE','en_US','0.1.0','now')",
            [],
        ).unwrap();
        let project_id = conn.last_insert_rowid();
        conn.execute("INSERT INTO speaker(project_id,cre_resref) VALUES(?1,'IMOEN')", [project_id]).unwrap();
        let speaker_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO line(project_id,strref,speaker_id) VALUES(?1,7,?2)",
            params![project_id, speaker_id],
        ).unwrap();
        let line_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO clone(speaker_id,binding_source,status) VALUES(?1,'default','ready')",
            [speaker_id],
        ).unwrap();
        (dir, path, conn, line_id)
    }

    #[test]
    fn option_parser_reads_values() {
        let args = vec![
            "--line".into(),
            "42".into(),
            "--text".into(),
            "hello".into(),
        ];
        assert_eq!(line_id(&args).unwrap(), 42);
        assert_eq!(value_after(&args, "--text").as_deref(), Some("hello"));
    }

    #[test]
    fn preset_cli_sets_clears_and_rejects_unknown_tokens() {
        let (_dir, path, mut conn, line_id) = preset_db();
        execute(
            &mut conn,
            &path,
            "preset",
            &["set".into(), "--line".into(), line_id.to_string(), "--preset".into(), "brisk".into()],
        ).unwrap();
        let state = crate::db::generation::agent_render_preset_state(&conn, line_id).unwrap();
        assert_eq!(state.preset, Some(AgentRenderPreset::Brisk));
        execute(
            &mut conn,
            &path,
            "preset",
            &["clear".into(), "--line".into(), line_id.to_string()],
        ).unwrap();
        let state = crate::db::generation::agent_render_preset_state(&conn, line_id).unwrap();
        assert_eq!(state.preset, Some(AgentRenderPreset::Inherit));
        assert!("unsafe".parse::<AgentRenderPreset>().is_err());
    }

    #[test]
    fn preset_batch_keeps_successful_lines_when_another_line_fails() {
        let (_dir, path, mut conn, line_id) = preset_db();
        let batch = path.parent().unwrap().join("presets.json");
        fs::write(
            &batch,
            format!(r#"[{{"line":{line_id},"preset":"deliberate"}},{{"line":999,"preset":"brisk"}}]"#),
        ).unwrap();
        let error = execute(
            &mut conn,
            &path,
            "preset",
            &["set".into(), "--batch".into(), batch.to_string_lossy().to_string()],
        ).unwrap_err().to_string();
        assert!(error.contains("successful lines were kept"));
        assert!(error.contains("line 999"));
        let state = crate::db::generation::agent_render_preset_state(&conn, line_id).unwrap();
        assert_eq!(state.preset, Some(AgentRenderPreset::Deliberate));
    }
}
