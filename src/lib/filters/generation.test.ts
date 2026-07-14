import { describe, expect, test } from "vitest";
import type { DemographicGroup, EffectiveSpeakerBinding, Line, Speaker } from "$lib/types";
import {
  activeGenerationScopeCount,
  emptyGenerationScope,
  filterGenerationScope,
  generationScopeChips,
  matchesGenerationScope,
  removeGenerationScopeChip,
  type GenerationScope,
  type GenerationScopeItem,
  type GenerationRenderState,
} from "./generation";

const group: DemographicGroup = {
  sex: 1,
  race: 6,
  creature_category: 1,
  sex_label: "Male",
  race_label: "Human",
  creature_category_label: "Humanoid",
  speaker_count: 2,
  line_count: 2,
  pool_size: 1,
  configured: true,
  unvoiced_count: 2,
  ready_clone_count: 2,
};

function makeSpeaker(id: number, name: string, race = 6): Speaker {
  return {
    id,
    project_id: 1,
    cre_resref: name.toLocaleLowerCase(),
    display_name: name,
    long_name_strref: 22000 + id,
    sex: 1,
    race,
    class: 1,
    kit: 0,
    alignment: 0,
    creature_category: 1,
    dialogue_resref: `${name.toLocaleLowerCase()}dlg`,
    provenance_json: "{}",
    confidence: 1,
  };
}

function makeLine(
  id: number,
  speakerId: number,
  text: string,
  extra: Partial<Line> = {},
): Line {
  return {
    id,
    project_id: 1,
    strref: 22000 + id,
    dlg_resref: id === 1 ? "xzardlg" : "montdlg",
    state_index: id,
    text,
    original_text: text,
    flags: 0,
    existing_sound_resref: null,
    kind: "state",
    is_voiced: false,
    has_tokens: false,
    token_mask: 0,
    shared_group_id: null,
    speaker_id: speakerId,
    attribution_confidence: 1,
    status: "ready",
    ...extra,
  };
}

function binding(speakerId: number, donorId: number, inherited: boolean): EffectiveSpeakerBinding {
  return {
    speaker_id: speakerId,
    line_count: 1,
    clone_id: speakerId,
    binding_source: inherited ? "generic" : "override",
    clone_status: "ready",
    sample_id: speakerId,
    sample_path: `voice-${speakerId}.wav`,
    donor_speaker_id: donorId,
    donor_display_name: donorId === 1 ? "Xzar" : "Montaron",
    inherited,
  };
}

function item(
  id: number,
  name: string,
  text: string,
  inherited: boolean,
  donorId: number,
  renderState: GenerationRenderState = "missing",
  lineExtra: Partial<Line> = {},
): GenerationScopeItem {
  const speaker = makeSpeaker(id, name, id === 1 ? 6 : 5);
  return {
    line: makeLine(id, id, text, lineExtra),
    speaker,
    demographic: { ...group, race: speaker.race, race_label: speaker.race === 6 ? "Human" : "Halfling" },
    binding: binding(id, donorId, inherited),
    renderState,
  };
}

const rows = [
  item(1, "Xzar", "Short warning", true, 1),
  item(2, "Montaron", "A considerably longer response", false, 1, "generated", {
    status: "exported",
    is_voiced: true,
    existing_sound_resref: "PACK0001",
  }),
  item(3, "Jaheira", "Failure", false, 3, "failed"),
];

function scoped(patch: Partial<GenerationScope>): GenerationScope {
  return { ...emptyGenerationScope(), ...patch };
}

describe("generation scope", () => {
  test("an empty scope includes every line and search covers identity, strref, DLG/state, and text", () => {
    expect(filterGenerationScope(rows, emptyGenerationScope())).toEqual(rows);
    expect(filterGenerationScope(rows, scoped({ search: "xzar" }))).toEqual([rows[0]]);
    expect(filterGenerationScope(rows, scoped({ search: "22002" }))).toEqual([rows[1]]);
    expect(filterGenerationScope(rows, scoped({ search: "montdlg:2" }))).toEqual([rows[1]]);
    expect(filterGenerationScope(rows, scoped({ search: "longer response" }))).toEqual([rows[1]]);
  });

  test("uses OR within categories and AND between categories", () => {
    const orScope = scoped({ speakers: ["1", "2"] });
    expect(filterGenerationScope(rows, orScope).map((row) => row.line.id)).toEqual([1, 2]);

    const andScope = scoped({ speakers: ["1", "2"], races: ["5"] });
    expect(filterGenerationScope(rows, andScope).map((row) => row.line.id)).toEqual([2]);
  });

  test("matches demographics, personal/default voice source, and effective donor", () => {
    expect(filterGenerationScope(rows, scoped({ sexes: ["1"], creatureCategories: ["1"] }))).toHaveLength(3);
    expect(filterGenerationScope(rows, scoped({ races: ["6"] }))).toEqual([rows[0]]);
    expect(filterGenerationScope(rows, scoped({ bindingModes: ["demographic"] }))).toEqual([rows[0]]);
    expect(filterGenerationScope(rows, scoped({ bindingModes: ["personal"], donors: ["1"] }))).toEqual([rows[1]]);
  });

  test("matches render, line, pack-audio, and DLG states", () => {
    expect(filterGenerationScope(rows, scoped({ renderStates: ["failed", "generated"] }))).toEqual([rows[1], rows[2]]);
    expect(filterGenerationScope(rows, scoped({ lineStates: ["exported"] }))).toEqual([rows[1]]);
    expect(filterGenerationScope(rows, scoped({ packAudio: ["present"] }))).toEqual([rows[1]]);
    expect(filterGenerationScope(rows, scoped({ packAudio: ["absent"], dlgs: ["xzardlg"] }))).toEqual([rows[0]]);
  });

  test("applies inclusive text-length bounds and ignores invalid numeric bounds", () => {
    const exactLength = String(rows[0].line.text.length);
    expect(matchesGenerationScope(rows[0], scoped({ minLength: exactLength, maxLength: exactLength }))).toBe(true);
    expect(matchesGenerationScope(rows[0], scoped({ minLength: String(rows[0].line.text.length + 1) }))).toBe(false);
    expect(matchesGenerationScope(rows[0], scoped({ maxLength: String(rows[0].line.text.length - 1) }))).toBe(false);
    expect(matchesGenerationScope(rows[0], scoped({ minLength: "not-a-number" }))).toBe(true);
  });

  test("counts selections, builds labelled chips, and removes one chip without mutating the source", () => {
    const scope = scoped({ search: "voice", speakers: ["1", "2"], renderStates: ["missing"], minLength: "5" });
    expect(activeGenerationScopeCount(scope)).toBe(5);
    const chips = generationScopeChips(scope, { speakers: { "1": "Xzar", "2": "Montaron" } });
    expect(chips.map((chip) => chip.label)).toEqual([
      "Search: voice",
      "Xzar",
      "Montaron",
      "missing",
      "Length ≥ 5",
    ]);
    const next = removeGenerationScopeChip(scope, chips[1]);
    expect(next.speakers).toEqual(["2"]);
    expect(scope.speakers).toEqual(["1", "2"]);
  });
});
