import { writable } from "svelte/store";
import { invoke } from "$lib/utils/invoke";
import type { SpeakerGroup } from "$lib/types";

export interface SpeakerGroupsState {
  gameDir: string | null;
  groups: SpeakerGroup[];
  loading: boolean;
  error: string | null;
}

function empty(): SpeakerGroupsState {
  return { gameDir: null, groups: [], loading: false, error: null };
}

export const speakerGroups = writable<SpeakerGroupsState>(empty());

/** Load groups for `gameDir`, skipping when already cached unless `force`. */
export async function loadSpeakerGroups(
  gameDir: string,
  force = false,
): Promise<SpeakerGroup[]> {
  let cached: SpeakerGroup[] = [];
  speakerGroups.update((s) => {
    if (!force && s.gameDir === gameDir && s.groups.length > 0) {
      cached = s.groups;
    }
    return { ...s, gameDir, loading: cached.length === 0, error: null };
  });
  if (cached.length > 0) return cached;

  try {
    const groups = await invoke<SpeakerGroup[]>("list_speaker_groups", { gameDir });
    speakerGroups.set({ gameDir, groups, loading: false, error: null });
    return groups;
  } catch (e) {
    const message = String(e);
    speakerGroups.set({ gameDir, groups: [], loading: false, error: message });
    throw e;
  }
}

export function invalidateSpeakerGroups(gameDir: string | null): void {
  speakerGroups.update((s) =>
    s.gameDir === gameDir ? { ...s, groups: [], error: null } : s,
  );
}

export function resetSpeakerGroups(): void {
  speakerGroups.set(empty());
}
