import type { InvokeArgs } from "@tauri-apps/api/core";
import type { SynthesisDecisionKind } from "../../src/lib/types";
import {
  attributionCounts,
  blockedLines,
  clones,
  demographicGroups,
  defaultRenderSettings,
  dictionaryRules,
  previewDictionary,
  upsertDictionaryRule,
  setDictionaryRuleEnabled,
  deleteDictionaryRule,
  effectiveBindings,
  engineStatus,
  FIXTURE_GAME_DIR,
  gameLanguages,
  generatableLines,
  healthReport,
  metadataBindings,
  referenceSamples,
  settings,
  speakerGroups,
  speakers,
  synthesisSummary,
  synthesisAuditSummary,
  listSynthesisDecisions,
  listSynthesisFlagged,
  listSynthesisRemaining,
  autoReviewSynthesisPlain,
  getSynthesisPreview,
  clearSynthesisOverride,
  setSynthesisOverride,
  markSynthesisReview,
  unmarkSynthesisReview,
  resetSynthesisAgentState,
  resetSynthesisFixtures,
} from "./data";

function arg<T>(args: InvokeArgs, key: string): T {
  return (args as Record<string, unknown>)[key] as T;
}

function requireGameDir(args: InvokeArgs): string {
  const gameDir = arg<string | undefined>(args, "gameDir");
  if (gameDir !== FIXTURE_GAME_DIR) {
    throw new Error(`E2E mock: unexpected gameDir ${String(gameDir)}`);
  }
  return gameDir;
}

/**
 * Central mock IPC handler for browser E2E (`VITE_E2E_MOCK=1`). Unknown commands
 * throw so missing fixtures are obvious when a screen starts calling a new command.
 */
export function handleMockCommand(cmd: string, args: InvokeArgs): unknown {
  switch (cmd) {
    case "health_check":
      return healthReport;

    case "get_setting": {
      const key = arg<string>(args, "key");
      return settings.get(key) ?? null;
    }

    case "set_setting": {
      const key = arg<string>(args, "key");
      const value = arg<string>(args, "value");
      settings.set(key, value === "" ? null : value);
      return undefined;
    }

    case "reapply_token_standins":
      requireGameDir(args);
      return {
        updated: 0,
        newly_ready: 0,
        newly_blocked: 0,
        newly_skipped: 0,
        reset_generations: 0,
      };

    case "list_dictionary_rules":
      return [...dictionaryRules];

    case "preview_dictionary_text":
      return previewDictionary(arg<string>(args, "text"));

    case "upsert_dictionary_rule":
      return upsertDictionaryRule({
        id: arg<number | null>(args, "id"),
        findText: arg<string>(args, "findText"),
        speakAs: arg<string>(args, "speakAs"),
        enabled: arg<boolean>(args, "enabled"),
      });

    case "set_dictionary_rule_enabled":
      return setDictionaryRuleEnabled(arg<number>(args, "id"), arg<boolean>(args, "enabled"));

    case "delete_dictionary_rule":
      return deleteDictionaryRule(arg<number>(args, "id"));

    case "reset_dictionary_defaults":
      return { rule: null, reset_generations: 0 };

    case "get_game_languages":
      requireGameDir(args);
      return gameLanguages;

    case "synthesis_tagging_summary":
      requireGameDir(args);
      return { ...synthesisSummary };

    case "synthesis_corpus_audit_summary":
      requireGameDir(args);
      return { ...synthesisAuditSummary };

    case "list_synthesis_flagged":
      requireGameDir(args);
      return listSynthesisFlagged();

    case "list_synthesis_remaining":
      requireGameDir(args);
      return listSynthesisRemaining();

    case "auto_review_synthesis_plain":
      requireGameDir(args);
      return autoReviewSynthesisPlain();

    case "get_line_synthesis_preview": {
      const lineId = arg<number>(args, "lineId");
      return getSynthesisPreview(lineId);
    }

    case "list_synthesis_decisions": {
      requireGameDir(args);
      const kind = arg<SynthesisDecisionKind>(args, "kind");
      return listSynthesisDecisions(kind);
    }

    case "clear_line_synthesis_override": {
      const lineId = arg<number>(args, "lineId");
      return clearSynthesisOverride(lineId);
    }

    case "unmark_synthesis_reviewed": {
      const lineId = arg<number>(args, "lineId");
      unmarkSynthesisReview(lineId);
      return undefined;
    }

    case "reset_synthesis_agent_state":
      requireGameDir(args);
      return resetSynthesisAgentState();

    case "prepare_agent_workspace":
      requireGameDir(args);
      return "C:\\fixture\\agent-workspace\\1";

    case "reveal_agent_workspace":
    case "launch_agent":
      requireGameDir(args);
      return undefined;

    case "get_attribution_counts":
      requireGameDir(args);
      return attributionCounts;

    case "list_blocked_lines":
      requireGameDir(args);
      return blockedLines;

    case "list_speakers":
      requireGameDir(args);
      return speakers;

    case "list_speaker_groups":
      requireGameDir(args);
      return speakerGroups;

    case "list_group_reference_samples": {
      requireGameDir(args);
      const identityKey = arg<string>(args, "identityKey");
      const group = speakerGroups.find((g) => g.identity_key === identityKey);
      if (!group) return [];
      const variantIds = new Set(group.variants.map((v) => v.speaker_id));
      return referenceSamples.filter((s) => variantIds.has(s.speaker_id));
    }

    case "set_line_synthesis_override": {
      const lineId = arg<number>(args, "lineId");
      const synthesisText = arg<string>(args, "synthesisText");
      return setSynthesisOverride(lineId, synthesisText);
    }

    case "mark_synthesis_reviewed": {
      const lineId = arg<number>(args, "lineId");
      markSynthesisReview(lineId);
      return undefined;
    }

    case "auto_approve_manual_gaps_samples":
      requireGameDir(args);
      return {
        speakers_considered: 2,
        speakers_skipped: 3,
        samples_approved: 2,
        samples_rejected: 0,
      };

    case "bind_clone":
    case "reconcile_identity_group_bindings":
    case "auto_bind_all":
      requireGameDir(args);
      if (cmd === "reconcile_identity_group_bindings") {
        return { groups_reconciled: 1, clones_propagated: 1, groups_skipped: 1 };
      }
      if (cmd === "auto_bind_all") {
        return { speakers_bound: 1, speakers_skipped: 1, speakers_failed: 0 };
      }
      return {
        clone: clones[0],
        reference_duration_secs: 1.2,
        duration_warning: null,
      };

    case "list_generatable_lines":
      requireGameDir(args);
      return generatableLines;

    case "list_render_candidates":
      requireGameDir(args);
      return JSON.parse(localStorage.getItem("e2e.render-candidates") ?? "[]");

    case "list_generation_diagnostics":
      requireGameDir(args);
      return [{ line_id: 1, diagnostics: { duration_secs: 0.2, voiced_fraction: 0.1, speech_ratio: null, silence_fraction: 0.8, clipping_fraction: 0, flags: ["short", "mostly_silent", "low_speech"] } }];

    case "get_line_render_override": {
      const lineId = arg<number>(args, "lineId");
      const settings = JSON.parse(localStorage.getItem(`e2e.line-override-${lineId}`) ?? "null");
      return settings ? { line_id: lineId, settings, resolved_settings: { ...defaultRenderSettings, ...settings } } : null;
    }

    case "set_line_render_override": {
      const lineId = arg<number>(args, "lineId");
      const settings = arg<Record<string, unknown>>(args, "settings");
      if (Object.keys(settings).length) localStorage.setItem(`e2e.line-override-${lineId}`, JSON.stringify(settings));
      else localStorage.removeItem(`e2e.line-override-${lineId}`);
      localStorage.setItem("e2e.render-candidates", "[]");
      return { override_state: Object.keys(settings).length ? { line_id: lineId, settings, resolved_settings: { ...defaultRenderSettings, ...settings } } : null, reset_generations: 1, candidate_discarded: false };
    }

    case "clear_line_render_override": {
      const lineId = arg<number>(args, "lineId");
      localStorage.removeItem(`e2e.line-override-${lineId}`);
      localStorage.setItem("e2e.render-candidates", "[]");
      return { override_state: null, reset_generations: 1, candidate_discarded: false };
    }

    case "generate_render_candidate": {
      const lineId = arg<number>(args, "lineId");
      const settings = JSON.parse(localStorage.getItem(`e2e.line-override-${lineId}`) ?? "{}") as Record<string, unknown>;
      const candidate = {
        line_id: lineId, status: "done", output_path: `C:\\fixture\\candidates\\${lineId}.ogg`,
        text_snapshot: generatableLines.find((line) => line.id === lineId)?.text ?? "fixture line",
        clone_id: 1, reference_sample_id: 1, reference_fingerprint: "fixture-reference",
        render_settings_json: JSON.stringify({ ...defaultRenderSettings, ...settings }),
        render_settings_hash: "fixture-settings", state_json: "{}",
      };
      localStorage.setItem("e2e.render-candidates", JSON.stringify([candidate]));
      return candidate;
    }

    case "accept_render_candidate": {
      const lineId = arg<number>(args, "lineId");
      localStorage.setItem("e2e.render-candidates", "[]");
      const completed = JSON.parse(localStorage.getItem("e2e.completed-generation-ids") ?? "[]") as number[];
      localStorage.setItem("e2e.completed-generation-ids", JSON.stringify([...new Set([...completed, lineId])]));
      return { generation_id: 1, output_path: `C:\\fixture\\generated\\${lineId}.ogg`, resumed: false };
    }

    case "discard_render_candidate":
      localStorage.setItem("e2e.render-candidates", "[]");
      return true;

    case "list_completed_generations":
      requireGameDir(args);
      return (JSON.parse(localStorage.getItem("e2e.completed-generation-ids") ?? "[]") as number[])
        .map((lineId) => ({
          line_id: lineId,
          output_path: `C:\\fixture\\generated\\${lineId}.ogg`,
          voice_changed: (JSON.parse(localStorage.getItem("e2e.voice-changed-generation-ids") ?? "[]") as number[]).includes(lineId),
        }));

    case "remove_generations": {
      requireGameDir(args);
      const lineIds = arg<number[]>(args, "lineIds");
      const completed = JSON.parse(localStorage.getItem("e2e.completed-generation-ids") ?? "[]") as number[];
      localStorage.setItem("e2e.completed-generation-ids", JSON.stringify(completed.filter((id) => !lineIds.includes(id))));
      return { records_removed: lineIds.length, files_deleted: lineIds.length, files_missing: 0 };
    }

    case "list_clones":
      requireGameDir(args);
      return clones;

    case "get_clone_render_settings": {
      const cloneId = arg<number>(args, "cloneId");
      const clone = clones.find((row) => row.id === cloneId);
      if (!clone) throw new Error(`E2E mock: no clone ${cloneId}`);
      return JSON.parse(clone.render_settings_json);
    }

    case "set_clone_render_settings": {
      const cloneId = arg<number>(args, "cloneId");
      const settings = arg<typeof defaultRenderSettings>(args, "settings");
      const clone = clones.find((row) => row.id === cloneId);
      if (!clone) throw new Error(`E2E mock: no clone ${cloneId}`);
      clone.render_settings_json = JSON.stringify(settings);
      return {
        clone,
        reset_generations: 2,
        files_deleted: 2,
        files_missing: 0,
      };
    }

    case "preview_clone_voice": {
      const text = arg<string>(args, "text");
      const reference = arg<"current" | "single" | "composite">(args, "reference");
      if (text.includes("[preview error]")) {
        throw new Error("Fixture preview failed");
      }
      const resolved = reference === "composite" ? "composite" : "single";
      return new Promise((resolve) => {
        window.setTimeout(
          () =>
            resolve({
              output_path: `C:\\fixture\\preview-${resolved}.wav`,
              reference: resolved,
              sample_ids:
                resolved === "composite" ? [2, 3] : [arg<number | null>(args, "sampleId") ?? 1],
              reference_duration_secs: resolved === "composite" ? 6.05 : 1.2,
              settings_fingerprint: "fixture-settings-fingerprint",
            }),
          100,
        );
      });
    }

    case "set_clone_references": {
      const cloneId = arg<number>(args, "cloneId");
      const sampleIds = arg<number[]>(args, "sampleIds");
      const clone = clones.find((row) => row.id === cloneId);
      if (!clone) throw new Error(`E2E mock: no clone ${cloneId}`);
      clone.primary_sample_id = sampleIds[0] ?? null;
      clone.binding_source = "override";
      clone.status = "ready";
      return {
        clone,
        references: sampleIds.map((sampleId, sortOrder) => ({
          clone_id: cloneId,
          sample_id: sampleId,
          sort_order: sortOrder,
        })),
        reset_generations: 2,
        files_deleted: 2,
        files_missing: 0,
      };
    }

    case "list_effective_speaker_bindings":
      requireGameDir(args);
      return effectiveBindings;

    case "use_demographic_default": {
      requireGameDir(args);
      const speakerId = arg<number>(args, "speakerId");
      return effectiveBindings.find((b) => b.speaker_id === speakerId) ?? effectiveBindings[0];
    }

    case "list_demographic_groups":
      requireGameDir(args);
      return demographicGroups;

    case "list_metadata_bindings":
      requireGameDir(args);
      return metadataBindings;

    case "suggest_metadata_donors":
      requireGameDir(args);
      return speakers.find((s) => s.id === 1) ?? null;

    case "list_eligible_metadata_donors": {
      requireGameDir(args);
      const cross = arg<boolean>(args, "crossDemographic");
      return speakers.filter((s) => (cross ? s.id !== 1 : s.id === 1));
    }

    case "auto_configure_metadata_pools":
      requireGameDir(args);
      return {
        groups_configured: 1,
        groups_skipped_no_donor: 0,
        groups_skipped_already_set: 0,
      };

    case "add_metadata_donor":
    case "remove_metadata_donor":
    case "clear_metadata_binding":
      return undefined;

    case "clear_all_metadata_pools":
      requireGameDir(args);
      return { cleared: 1 };

    case "clear_speaker_clones":
      requireGameDir(args);
      return { cleared: 1 };

    case "apply_metadata_bindings":
      requireGameDir(args);
      return {
        speakers_pool_bound: 0,
        speakers_auto_bound: 0,
        speakers_failed: 0,
        speakers_skipped: 0,
        assignments: [],
      };

    case "list_reference_samples": {
      const speakerId = arg<number>(args, "speakerId");
      return referenceSamples.filter((s) => s.speaker_id === speakerId);
    }

    case "engine_status":
      return localStorage.getItem("e2e.engine-running") === "true"
        ? { ...engineStatus, running: true, ready: true, owned: true }
        : engineStatus;

    case "start_engine":
      localStorage.setItem("e2e.engine-running", "true");
      return { ...engineStatus, running: true, ready: true, owned: true };

    case "generate_lines_batched": {
      const lineIds = arg<number[]>(args, "lineIds");
      localStorage.setItem("e2e.last-generation-batch", JSON.stringify(lineIds));
      const completed = JSON.parse(localStorage.getItem("e2e.completed-generation-ids") ?? "[]") as number[];
      localStorage.setItem("e2e.completed-generation-ids", JSON.stringify([...new Set([...completed, ...lineIds])]));
      const stale = JSON.parse(localStorage.getItem("e2e.voice-changed-generation-ids") ?? "[]") as number[];
      localStorage.setItem("e2e.voice-changed-generation-ids", JSON.stringify(stale.filter((id) => !lineIds.includes(id))));
      return {
        total: lineIds.length,
        generated: lineIds.length,
        resumed: 0,
        failed: 0,
        outcomes: lineIds.map((lineId) => ({
          line_id: lineId,
          status: "done",
          output_path: `C:\\fixture\\generated\\${lineId}.ogg`,
          error: null,
        })),
      };
    }

    case "cancel_operation":
      return false;

    default:
      throw new Error(`E2E mock: unhandled command "${cmd}"`);
  }
}
