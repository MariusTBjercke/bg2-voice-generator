//! Compile-time templates for the external synthesis-tagging agent workspace.
//!
//! The staged workspace follows the emerging cross-agent conventions:
//! - [`AGENTS.md`](https://developers.openai.com/codex/guides/agents-md) is the single
//!   maintained instruction file (Codex reads it directly; other agents can import it).
//! - `.agents/skills/` holds the repo skill Codex discovers automatically.
//! - `CLAUDE.md` is a thin pointer that imports `AGENTS.md` for Claude Code.
//! - `.claude/skills/` mirrors the same skill so Claude auto-loads it.

pub const SET_SYNTHESIS_SKILL: &str = include_str!("agent_templates/set-synthesis-skill.md");

pub const AGENTS_SKILL_PATH: &str = ".agents/skills/set-synthesis/SKILL.md";
pub const CLAUDE_SKILL_PATH: &str = ".claude/skills/set-synthesis/SKILL.md";

pub struct WorkspaceContext {
    pub cli_path: String,
    pub db_path: String,
    pub project_id: i64,
    pub game_root: String,
    pub unique_strings: usize,
    pub overridden: usize,
    pub reviewed: usize,
    pub remaining: usize,
}

/// Render `(CLAUDE.md, AGENTS.md)` for a prepared agent workspace. Pure: no IO.
pub fn render_workspace_docs(ctx: &WorkspaceContext) -> (String, String) {
    let cli = if ctx.cli_path.is_empty() {
        "bg2-synthesis".to_string()
    } else {
        format!("\"{}\"", ctx.cli_path)
    };

    let agents = format!(
        "# BG2 synthesis-text agent workspace\n\n\
         This folder is a prepared workspace the BG2 Voice Generator opened for you. Work only \
         on project {project_id} (`{game_root}`). Use the companion CLI for every read and \
         write; never edit SQLite directly.\n\n\
         The `set-synthesis` workflow skill is in this workspace at:\n\
         - `{agents_skill}` (Codex discovers this automatically)\n\
         - `{claude_skill}` (same content; Claude Code auto-loads this)\n\n\
         Read the skill in full before you run any CLI commands.\n\n\
         CLI: `{cli}`\n\
         Database: `{db}`\n\n\
         Always include `--db \"{db}\"` in CLI calls. Start with `audit-corpus --project \
         {project_id}`, run `auto-review-plain --project {project_id}`, then work \
         `list-flagged --project {project_id}` pages. Default to `review` when mapper output \
         is acceptable. Use `tag` only for mapper fixes or rare, high-confidence delivery \
         tweaks with allowed OmniVoice inline tags (run `bg2-synthesis catalog`). Overrides \
         may normalize only tokens flagged as TTS-unfriendly (stutters, elongated spellings, \
         or written screams); preserve every ordinary dialogue word. Global Dictionary rule \
         curation is a separate task and is not part of this review workflow. Overrides \
         must be final generation text with `[catalog tags]`, not `*stage directions*`. Do not \
         change subtitle text. For rare, clearly contextual pacing issues, agents may use only \
         `bg2-synthesis preset` named choices; start with `preset list` and keep `inherit` unless \
         a named change is strongly justified. Never set raw render fields, render or audition \
         audio, or accept candidates.\n\n\
         Current progress: {unique} unique, {overridden} overridden, {reviewed} reviewed, \
         {remaining} remaining.\n",
        project_id = ctx.project_id,
        game_root = ctx.game_root,
        agents_skill = AGENTS_SKILL_PATH,
        claude_skill = CLAUDE_SKILL_PATH,
        cli = cli,
        db = ctx.db_path,
        unique = ctx.unique_strings,
        overridden = ctx.overridden,
        reviewed = ctx.reviewed,
        remaining = ctx.remaining,
    );

    let claude = format!(
        "# BG2 synthesis-text workspace\n\n\
         The full instructions live in `AGENTS.md` in this folder — read it before you touch \
         the database. The `set-synthesis` skill under `{claude_skill}` loads automatically.\n\n\
         @AGENTS.md\n",
        claude_skill = CLAUDE_SKILL_PATH,
    );

    (claude, agents)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ctx() -> WorkspaceContext {
        WorkspaceContext {
            cli_path: r"C:\app\bg2-synthesis.exe".into(),
            db_path: r"C:\data\bg2vg.db".into(),
            project_id: 7,
            game_root: r"C:\Games\BG2".into(),
            unique_strings: 10,
            overridden: 2,
            reviewed: 3,
            remaining: 5,
        }
    }

    #[test]
    fn skill_is_embedded() {
        assert!(SET_SYNTHESIS_SKILL.contains("name: set-synthesis"));
        assert!(SET_SYNTHESIS_SKILL.contains("bg2-synthesis"));
        assert!(SET_SYNTHESIS_SKILL.contains("bg2-synthesis catalog"));
        assert!(SET_SYNTHESIS_SKILL.contains("preset list"));
        assert!(SET_SYNTHESIS_SKILL.contains("accept a candidate"));
        assert!(SET_SYNTHESIS_SKILL.contains("Decision tree"));
    }

    #[test]
    fn agents_md_is_the_maintained_instruction_file() {
        let (claude_md, agents_md) = render_workspace_docs(&sample_ctx());
        assert!(agents_md.contains("bg2-synthesis.exe"));
        assert!(agents_md.contains("5 remaining"));
        assert!(agents_md.contains(AGENTS_SKILL_PATH));
        assert!(agents_md.contains(CLAUDE_SKILL_PATH));
        assert!(agents_md.contains("never edit SQLite directly"));
        assert!(agents_md.contains("Never set raw render fields"));
        assert!(!agents_md.contains("@AGENTS.md"));

        assert!(claude_md.contains("@AGENTS.md"));
        assert!(claude_md.contains(CLAUDE_SKILL_PATH));
        assert!(
            !claude_md.contains("never edit SQLite directly"),
            "CLAUDE.md stays a pointer"
        );
    }
}
