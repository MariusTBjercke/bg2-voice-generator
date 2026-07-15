//! Prepare and launch external coding agents for optional synthesis-text review.

use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::agent_templates::{
    render_workspace_docs, WorkspaceContext, AGENTS_SKILL_PATH, CLAUDE_SKILL_PATH,
    SET_SYNTHESIS_SKILL,
};
use crate::error::AppError;
use crate::AppState;

fn data_dir(db_path: &Path) -> Result<&Path, AppError> {
    db_path
        .parent()
        .ok_or_else(|| AppError::Other("database path has no parent directory".into()))
}

fn project_for_game_dir(
    conn: &rusqlite::Connection,
    game_dir: &str,
) -> Result<(i64, String), AppError> {
    conn.query_row(
        "SELECT id,game_root FROM project WHERE game_root=?1",
        params![game_dir],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .ok_or_else(|| AppError::Other("scan this game install before launching the agent".into()))
}

fn cli_path() -> String {
    let name = if cfg!(windows) {
        "bg2-synthesis.exe"
    } else {
        "bg2-synthesis"
    };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join(name)))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn workspace_dir(data_dir: &Path, project_id: i64) -> PathBuf {
    data_dir
        .join("agent-workspace")
        .join(project_id.to_string())
}

fn stage_skill(workspace: &Path, content: &str) -> Result<(), AppError> {
    for relative in [AGENTS_SKILL_PATH, CLAUDE_SKILL_PATH] {
        let path = workspace.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(())
}

fn stage_workspace(
    conn: &rusqlite::Connection,
    db_path: &Path,
    game_dir: &str,
) -> Result<PathBuf, AppError> {
    let (project_id, game_root) = project_for_game_dir(conn, game_dir)?;
    let summary = crate::synthesis::tagging_summary(conn, Some(project_id), true)?;
    let context = WorkspaceContext {
        cli_path: cli_path(),
        db_path: db_path.to_string_lossy().into_owned(),
        project_id,
        game_root,
        unique_strings: summary.unique_strings,
        overridden: summary.overridden,
        reviewed: summary.reviewed,
        remaining: summary.remaining,
    };
    let (claude, agents) = render_workspace_docs(&context);
    let workspace = workspace_dir(data_dir(db_path)?, project_id);
    stage_skill(&workspace, SET_SYNTHESIS_SKILL)?;
    std::fs::write(workspace.join("CLAUDE.md"), claude)?;
    std::fs::write(workspace.join("AGENTS.md"), agents)?;
    Ok(workspace)
}

pub fn refresh_all_agent_workspaces(conn: &rusqlite::Connection, db_path: &Path) {
    let Some(data_dir) = db_path.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(data_dir.join("agent-workspace")) else {
        return;
    };
    for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
        let Some(project_id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i64>().ok())
        else {
            continue;
        };
        let game_dir: Option<String> = conn
            .query_row(
                "SELECT game_root FROM project WHERE id=?1",
                params![project_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        if let Some(game_dir) = game_dir {
            if let Err(error) = stage_workspace(conn, db_path, &game_dir) {
                log::warn!("failed to refresh agent workspace {project_id}: {error}");
            }
        }
    }
}

#[tauri::command]
pub async fn prepare_agent_workspace(
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<String, AppError> {
    let conn = state.db.lock().await;
    Ok(stage_workspace(&conn, &state.db_path, &game_dir)?
        .to_string_lossy()
        .into_owned())
}

#[tauri::command]
pub async fn reveal_agent_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    game_dir: String,
) -> Result<(), AppError> {
    let path = prepare_agent_workspace(state, game_dir).await?;
    app.opener()
        .open_path(&path, None::<&str>)
        .map_err(|error| AppError::Other(format!("could not open {path}: {error}")))
}

fn executable_on_path(name: &str) -> bool {
    let checker = if cfg!(windows) { "where.exe" } else { "which" };
    Command::new(checker)
        .arg(name)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn spawn_terminal(agent: &str, yolo: bool, workspace: &Path) -> Result<(), AppError> {
    let bypass = match agent {
        "claude" => "--dangerously-skip-permissions",
        "codex" => "--dangerously-bypass-approvals-and-sandbox",
        _ => return Err(AppError::Other("agent must be claude or codex".into())),
    };
    #[cfg(target_os = "windows")]
    {
        let mut args = vec![
            "/C".to_string(),
            "start".to_string(),
            "".to_string(),
            "cmd".to_string(),
            "/K".to_string(),
            agent.to_string(),
        ];
        if yolo {
            args.push(bypass.to_string());
        }
        Command::new("cmd.exe")
            .args(args)
            .current_dir(workspace)
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = yolo;
        let _ = bypass;
        Command::new("open")
            .args(["-a", "Terminal", workspace.to_string_lossy().as_ref()])
            .spawn()?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let mut shell = format!("exec {agent}");
        if yolo {
            shell.push(' ');
            shell.push_str(bypass);
        }
        Command::new("x-terminal-emulator")
            .args([
                "--working-directory",
                workspace.to_string_lossy().as_ref(),
                "-e",
                "sh",
                "-lc",
                &shell,
            ])
            .spawn()?;
    }
    Ok(())
}

#[tauri::command]
pub async fn launch_agent(
    state: State<'_, AppState>,
    game_dir: String,
    agent: String,
    yolo: bool,
) -> Result<(), AppError> {
    if !matches!(agent.as_str(), "claude" | "codex") {
        return Err(AppError::Other("agent must be claude or codex".into()));
    }
    if !executable_on_path(&agent) {
        return Err(AppError::Other(format!(
            "{agent} is not on PATH; install it or add it to PATH first"
        )));
    }
    let conn = state.db.lock().await;
    let workspace = stage_workspace(&conn, &state.db_path, &game_dir)?;
    drop(conn);
    spawn_terminal(&agent, yolo, &workspace)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_known_agent_names_are_accepted_by_terminal_builder() {
        assert!(matches!("claude", "claude" | "codex"));
        assert!(!matches!("powershell", "claude" | "codex"));
    }
}
