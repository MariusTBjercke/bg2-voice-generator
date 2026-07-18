import type { InvokeArgs } from "@tauri-apps/api/core";
import type { DesignVoiceAttributes, SynthesisDecisionKind } from "../../src/lib/types";
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
  tagRules,
  supportedInlineTags,
  previewTagRules,
  upsertTagRule,
  setTagRuleEnabled,
  deleteTagRule,
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
  voiceProfiles,
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

function optionallyDelayed<T>(storageKey: string, value: T): T | Promise<T> {
  const delay = Number(localStorage.getItem(storageKey) ?? "0");
  return delay > 0
    ? new Promise((resolve) => window.setTimeout(() => resolve(value), delay))
    : value;
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

    case "get_peak_normalize_default": {
      const raw = settings.get("omnivoice_peak_normalize_dbfs");
      if (raw == null || raw === "") return -1;
      if (raw.toLowerCase() === "off") return null;
      return Number(raw);
    }

    case "set_peak_normalize_default": {
      const value = args && "value" in args ? (args.value as number | null) : null;
      if (value === null) settings.set("omnivoice_peak_normalize_dbfs", "off");
      else if (value === -1) settings.set("omnivoice_peak_normalize_dbfs", null);
      else settings.set("omnivoice_peak_normalize_dbfs", String(value));
      return 0;
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

    case "list_tag_rules":
      return [...tagRules];

    case "list_supported_inline_tags":
      return [...supportedInlineTags];

    case "preview_tag_rules_text":
      return previewTagRules(arg<string>(args, "text"));

    case "upsert_tag_rule":
      return upsertTagRule({
        id: arg<number | null>(args, "id"),
        findText: arg<string>(args, "findText"),
        tag: arg<string>(args, "tag"),
        matchKind: arg<"stage_cue" | "whole_word">(args, "matchKind"),
        enabled: arg<boolean>(args, "enabled"),
      });

    case "set_tag_rule_enabled":
      return setTagRuleEnabled(arg<number>(args, "id"), arg<boolean>(args, "enabled"));

    case "delete_tag_rule":
      return deleteTagRule(arg<number>(args, "id"));

    case "reset_tag_rule_defaults":
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

    case "binding_audit_progress":
      requireGameDir(args);
      return {
        personal_ready: 3,
        flagged: 1,
        reviewed: 1,
        remaining_personal: 2,
        generic_skipped: 5,
        unbound: 10,
      };

    case "list_marked_bindings":
      requireGameDir(args);
      return [
        {
          speaker_id: 101,
          display_name: "Boy",
          cre_resref: "BOY01",
          sex: 1,
          display_identity_key: "100:1",
          binding_source: "default",
          sample_id: 55,
          sample_sound_resref: "jaheir62",
          sample_owner_cre_resref: "BOY01",
          sample_text_excerpt: "It is a path of conscience.",
          review_status: arg(args, "status") === "reviewed" ? "reviewed" : "flagged",
          review_reason: "agent noted foreign VO",
          heuristic_hints: [],
        },
      ];

    case "list_suspicious_bindings":
      requireGameDir(args);
      return [
        {
          speaker_id: 101,
          display_name: "Boy",
          cre_resref: "BOY01",
          sex: 1,
          display_identity_key: "100:1",
          binding_source: "default",
          sample_id: 55,
          sample_sound_resref: "jaheir62",
          sample_owner_cre_resref: "BOY01",
          sample_text_excerpt: "It is a path of conscience.",
          review_status: null,
          review_reason: "",
          heuristic_hints: [
            {
              code: "crowd_with_companion_stem",
              detail: "crowd display name `Boy` bound to companion-like stem `jaheir`",
            },
          ],
        },
      ];

    case "list_personal_bindings":
      requireGameDir(args);
      return [
        {
          speaker_id: 101,
          display_name: "Boy",
          cre_resref: "BOY01",
          sex: 1,
          display_identity_key: "100:1",
          operational_identity_key: "ungrouped:101",
          binding_source: "default",
          clone_status: "ready",
          sample_id: 55,
          sample_sound_resref: "jaheir62",
          sample_owner_cre_resref: "BOY01",
          sample_eligibility: "automatic",
          sample_shared_source_count: 1,
          sample_text_excerpt: "It is a path of conscience.",
          review_status: null,
          review_reason: "",
          heuristic_hints: [],
        },
      ];

    case "list_binding_groups":
      requireGameDir(args);
      return [];

    case "show_binding_detail":
      requireGameDir(args);
      return {
        speaker_id: 101,
        display_name: "Boy",
        cre_resref: "BOY01",
        sex: 1,
        display_identity_key: "100:1",
        operational_identity_key: "ungrouped:101",
        binding_source: "default",
        clone_status: "ready",
        sample_id: 55,
        review: null,
        personal: null,
        samples: [],
        display_group_siblings: [],
        shares_voice_with_display_group: false,
      };

    case "flag_binding_review":
    case "mark_binding_reviewed":
      requireGameDir(args);
      return {
        project_id: 1,
        cre_resref: String(arg(args, "creResref") ?? "BOY01"),
        status: cmd === "flag_binding_review" ? "flagged" : "reviewed",
        reason: String(arg(args, "reason") ?? ""),
        updated_at: "now",
      };

    case "clear_binding_review_marker":
    case "clear_personal_binding":
      requireGameDir(args);
      return true;

    case "reject_binding_sample":
      requireGameDir(args);
      return null;

    case "list_synthesis_flagged": {
      requireGameDir(args);
      return optionallyDelayed("e2e.delay-review-ms", listSynthesisFlagged(
        arg<string | undefined>(args, "query"),
        arg<string | undefined>(args, "flag"),
      ));
    }

    case "list_synthesis_remaining":
      requireGameDir(args);
      return listSynthesisRemaining(
        arg<string | undefined>(args, "query"),
        arg<string | undefined>(args, "flag"),
      );

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
      return listSynthesisDecisions(kind, arg<string | undefined>(args, "query"));
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

    case "list_blocked_lines_page": {
      requireGameDir(args);
      const reasonFor = (line: (typeof blockedLines)[number]) => {
        if (line.is_voiced) return "already voiced";
        if (line.has_tokens || line.kind === "token") return "dynamic token";
        if (line.kind === "transition" || line.kind === "script") return "not a state line";
        if (line.shared_group_id !== null) return "shared (different voice)";
        if (line.speaker_id === null) return "unattributed";
        return "other";
      };
      const query = String(arg<string | undefined>(args, "query") ?? "").trim().toLowerCase();
      const reason = String(arg<string | undefined>(args, "reason") ?? "all");
      const filtered = blockedLines.filter((line) =>
        (reason === "all" || reasonFor(line) === reason)
        && (!query || [line.strref, `${line.dlg_resref ?? ""}:${line.state_index ?? ""}`, line.text]
          .some((value) => String(value).toLowerCase().includes(query))),
      );
      const offset = Number(arg<number | undefined>(args, "offset") ?? 0);
      const limit = Number(arg<number | undefined>(args, "limit") ?? 100);
      return {
        rows: filtered.slice(offset, offset + limit),
        total: filtered.length,
        token_total: blockedLines.filter((line) => reasonFor(line) === "dynamic token").length,
      };
    }

    case "list_speakers":
      requireGameDir(args);
      return speakers;

    case "list_speaker_groups":
      requireGameDir(args);
      return speakerGroups;

    case "count_speaker_group_generations": {
      requireGameDir(args);
      return 0;
    }

    case "set_speaker_group_excluded": {
      requireGameDir(args);
      const identityKey = arg<string>(args, "identityKey");
      const excluded = arg<boolean>(args, "excluded");
      const group = speakerGroups.find((g) => g.identity_key === identityKey);
      if (group) {
        group.excluded = excluded;
        for (const variant of group.variants) {
          const speaker = speakers.find((s) => s.id === variant.speaker_id);
          if (speaker) speaker.excluded = excluded;
        }
      }
      return {
        speakers_updated: group?.variant_count ?? 0,
        generations_cleared: 0,
        files_deleted: 0,
      };
    }

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

    case "list_generatable_lines": {
      requireGameDir(args);
      const completed = new Set(
        JSON.parse(localStorage.getItem("e2e.completed-generation-ids") ?? "[]") as number[],
      );
      // Mirror backend: blocked/skipped only appear when they still have a done clip.
      const lines = generatableLines.filter(
        (line) =>
          line.status === "ready" ||
          line.status === "exported" ||
          completed.has(line.id),
      );
      const delay = Number(localStorage.getItem("e2e.delay-generatable-ms") ?? "0");
      if (delay > 0) {
        return new Promise((resolve) => window.setTimeout(() => resolve(lines), delay));
      }
      return lines;
    }

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
          text_changed: false,
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

    case "list_voice_profiles":
      requireGameDir(args);
      return voiceProfiles;

    case "select_voice_reference_files":
      return ["C:\\fixture\\imports\\custom-voice.wav"];

    case "create_imported_voice_profile": {
      requireGameDir(args);
      const id = Math.max(...voiceProfiles.map((profile) => profile.id), 100) + 1;
      const clips = arg<Array<{ path: string; transcript: string }>>(args, "clips");
      const profile = {
        id, project_id: 1, display_name: arg<string>(args, "displayName"), origin: "imported" as const,
        harvested_speaker_id: null, design: null, availability: "available" as const,
        reference_fingerprint: `fixture-${id}`, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z",
        references: clips.map((clip, index) => ({ id: id * 10 + index, voice_profile_id: id, reference_sample_id: null, managed_path: `C:\\fixture\\profiles\\${id}\\reference-${index}.wav`, resolved_audio_path: `C:\\fixture\\profiles\\${id}\\reference-${index}.wav`, source_strref: null, source_sound_resref: null, transcript: clip.transcript, sort_order: index, fingerprint: `fixture-ref-${id}-${index}` })),
      };
      voiceProfiles.push(profile);
      return profile;
    }

    case "generate_designed_voice_candidates":
      requireGameDir(args);
      return {
        candidates: [42, 137, 911].map((seed, index) => ({ preview_id: `fixture-${seed}`, output_path: `C:\\fixture\\previews\\${index}.wav`, seed, duration_secs: 6.2 + index / 10 })),
        quality_warning: null,
      };

    case "save_designed_voice_profile": {
      requireGameDir(args);
      const id = Math.max(...voiceProfiles.map((profile) => profile.id), 100) + 1;
      const profile = {
        id, project_id: 1, display_name: arg<string>(args, "displayName"), origin: "designed" as const,
        harvested_speaker_id: null, design: arg<DesignVoiceAttributes>(args, "attributes"), availability: "available" as const,
        reference_fingerprint: `fixture-${id}`, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z",
        references: [{ id: id * 10, voice_profile_id: id, reference_sample_id: null, managed_path: `C:\\fixture\\profiles\\${id}\\reference-0.wav`, resolved_audio_path: `C:\\fixture\\profiles\\${id}\\reference-0.wav`, source_strref: null, source_sound_resref: null, transcript: arg<string>(args, "text"), sort_order: 0, fingerprint: `fixture-ref-${id}` }],
      };
      voiceProfiles.push(profile);
      return profile;
    }

    case "bind_speaker_voice_profile": {
      requireGameDir(args);
      const profile = voiceProfiles.find((row) => row.id === arg<number>(args, "voiceProfileId"));
      const speakerId = arg<number>(args, "speakerId");
      const effective = effectiveBindings.find((row) => row.speaker_id === speakerId);
      if (profile && effective) {
        effective.binding_source = "override";
        effective.inherited = false;
        effective.follow_speaker_id = null;
        effective.follow_display_name = null;
        effective.voice_profile_id = profile.id;
        effective.voice_profile_name = profile.display_name;
        effective.voice_profile_origin = profile.origin;
        effective.donor_speaker_id = null;
        effective.donor_display_name = null;
        effective.sample_id = null;
        effective.sample_path = profile.references[0]?.resolved_audio_path ?? null;
      }
      const clone = clones.find((row) => row.speaker_id === speakerId);
      if (clone && profile) {
        clone.voice_profile_id = profile.id;
        clone.primary_sample_id = null;
        clone.follow_speaker_id = null;
        clone.binding_source = "override";
      }
      return profile;
    }

    case "rename_voice_profile": {
      requireGameDir(args);
      const profile = voiceProfiles.find((row) => row.id === arg<number>(args, "voiceProfileId"));
      if (!profile) throw new Error("E2E mock: voice profile not found");
      profile.display_name = arg<string>(args, "displayName");
      return profile;
    }

    case "delete_voice_profile": {
      requireGameDir(args);
      const id = arg<number>(args, "voiceProfileId");
      const impact = { affected_speakers: 1, affected_pools: 1, reset_generations: 1, files_deleted: 1 };
      if (!arg<boolean | undefined>(args, "dryRun")) {
        const index = voiceProfiles.findIndex((profile) => profile.id === id);
        if (index >= 0) voiceProfiles.splice(index, 1);
      }
      return impact;
    }

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
        files_deleted: 0,
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
        files_deleted: 0,
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

    case "follow_speaker_voice": {
      requireGameDir(args);
      const speakerId = arg<number>(args, "speakerId");
      const followSpeakerId = arg<number>(args, "followSpeakerId");
      const target = effectiveBindings.find((b) => b.speaker_id === followSpeakerId);
      const followName =
        speakers.find((s) => s.id === followSpeakerId)?.display_name ??
        String(followSpeakerId);
      const idx = effectiveBindings.findIndex((b) => b.speaker_id === speakerId);
      if (idx >= 0) {
        effectiveBindings[idx] = {
          ...effectiveBindings[idx],
          binding_source: "follow",
          inherited: false,
          follow_speaker_id: followSpeakerId,
          follow_display_name: followName,
          clone_status: target?.clone_status ?? "ready",
          sample_id: target?.sample_id ?? null,
          sample_path: target?.sample_path ?? null,
          voice_profile_id: target?.voice_profile_id ?? null,
          voice_profile_name: target?.voice_profile_name ?? null,
          voice_profile_origin: target?.voice_profile_origin ?? null,
          donor_speaker_id: target?.donor_speaker_id ?? null,
          donor_display_name: target?.donor_display_name ?? null,
        };
        const clone = clones.find((row) => row.speaker_id === speakerId);
        if (clone) {
          clone.binding_source = "follow";
          clone.follow_speaker_id = followSpeakerId;
          clone.primary_sample_id = null;
          clone.voice_profile_id = null;
          clone.status = "ready";
        }
        return effectiveBindings[idx];
      }
      return {
        speaker_id: speakerId,
        line_count: 0,
        clone_id: 99,
        binding_source: "follow",
        clone_status: "ready",
        sample_id: target?.sample_id ?? null,
        sample_path: target?.sample_path ?? null,
        voice_profile_id: target?.voice_profile_id ?? null,
        voice_profile_name: target?.voice_profile_name ?? null,
        voice_profile_origin: target?.voice_profile_origin ?? null,
        donor_speaker_id: target?.donor_speaker_id ?? null,
        donor_display_name: target?.donor_display_name ?? null,
        inherited: false,
        follow_speaker_id: followSpeakerId,
        follow_display_name: followName,
        sample_voice_sex: null,
      };
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

    case "add_metadata_donor": {
      const speakerId = arg<number>(args, "donorSpeakerId");
      if (!metadataBindings[0].donor_speaker_ids.includes(speakerId)) metadataBindings[0].donor_speaker_ids.push(speakerId);
      const profile = voiceProfiles.find((row) => row.origin === "harvested" && row.harvested_speaker_id === speakerId);
      if (profile && !metadataBindings[0].voice_profile_ids.includes(profile.id)) metadataBindings[0].voice_profile_ids.push(profile.id);
      return undefined;
    }

    case "remove_metadata_donor": {
      const speakerId = arg<number>(args, "donorSpeakerId");
      metadataBindings[0].donor_speaker_ids = metadataBindings[0].donor_speaker_ids.filter((id) => id !== speakerId);
      const harvestedIds = new Set(voiceProfiles.filter((row) => row.origin === "harvested" && row.harvested_speaker_id === speakerId).map((row) => row.id));
      metadataBindings[0].voice_profile_ids = metadataBindings[0].voice_profile_ids.filter((id) => !harvestedIds.has(id));
      return undefined;
    }

    case "clear_metadata_binding":
      return undefined;

    case "add_metadata_profile": {
      requireGameDir(args);
      const id = arg<number>(args, "voiceProfileId");
      if (!metadataBindings[0].voice_profile_ids.includes(id)) metadataBindings[0].voice_profile_ids.push(id);
      const profile = voiceProfiles.find((row) => row.id === id);
      if (profile?.harvested_speaker_id !== null && profile?.harvested_speaker_id !== undefined && !metadataBindings[0].donor_speaker_ids.includes(profile.harvested_speaker_id)) {
        metadataBindings[0].donor_speaker_ids.push(profile.harvested_speaker_id);
      }
      return undefined;
    }

    case "remove_metadata_profile": {
      requireGameDir(args);
      const id = arg<number>(args, "voiceProfileId");
      metadataBindings[0].voice_profile_ids = metadataBindings[0].voice_profile_ids.filter((profileId) => profileId !== id);
      const profile = voiceProfiles.find((row) => row.id === id);
      if (profile?.harvested_speaker_id !== null && profile?.harvested_speaker_id !== undefined) {
        metadataBindings[0].donor_speaker_ids = metadataBindings[0].donor_speaker_ids.filter((speakerId) => speakerId !== profile.harvested_speaker_id);
      }
      return undefined;
    }

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
