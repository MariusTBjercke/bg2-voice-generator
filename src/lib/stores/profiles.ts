// Active profile + registry cache for the shell switcher. UI state only; all
// persistence goes through Tauri profile commands.

import { get, writable } from "svelte/store";
import { invoke } from "$lib/utils/invoke";
import type { ProfileInfo, ProfileRegistry } from "$lib/types";
import { project } from "$lib/stores/project";
import { ensureGameDir } from "$lib/stores/results";
import { resetSpeakerGroups } from "$lib/stores/speakerGroups";

export interface ProfilesState {
  active: ProfileInfo | null;
  registry: ProfileRegistry | null;
  loading: boolean;
  error: string | null;
}

export const profiles = writable<ProfilesState>({
  active: null,
  registry: null,
  loading: false,
  error: null,
});

/** Bumped on profile switch/import so the shell remounts the active route. */
export const profileGeneration = writable(0);

function bumpProfileGeneration(): void {
  profileGeneration.update((n) => n + 1);
}

function clearProfileCaches(): void {
  ensureGameDir(null);
  resetSpeakerGroups();
  project.set({ gameDir: null, locale: null });
}

export async function refreshProfiles(): Promise<void> {
  profiles.update((p) => ({ ...p, loading: true, error: null }));
  try {
    const registry = await invoke<ProfileRegistry>("list_profiles");
    const active =
      registry.profiles.find((p) => p.id === registry.active_id) ??
      registry.profiles[0] ??
      null;
    profiles.set({ active, registry, loading: false, error: null });
  } catch (e) {
    profiles.update((p) => ({
      ...p,
      loading: false,
      error: String(e),
    }));
  }
}

/** Re-hydrate `game_dir` from the active profile DB into the project store. */
async function hydrateGameDir(): Promise<void> {
  try {
    const gameDir = (await invoke<string | null>("get_setting", { key: "game_dir" })) ?? null;
    project.update((p) => ({ ...p, gameDir }));
    if (gameDir) ensureGameDir(gameDir);
  } catch {
    // Setup will surface errors
  }
}

/** Switch profile, clear result caches, and re-hydrate game_dir from the new DB. */
export async function switchToProfile(id: string): Promise<void> {
  const current = get(profiles).active?.id;
  if (current === id) return;
  await invoke<ProfileInfo>("switch_profile", { id });
  clearProfileCaches();
  await refreshProfiles();
  bumpProfileGeneration();
  await hydrateGameDir();
}

/** After import (or any out-of-band active-profile change): drop caches and remount routes. */
export async function adoptActiveProfile(): Promise<void> {
  clearProfileCaches();
  await refreshProfiles();
  bumpProfileGeneration();
  await hydrateGameDir();
}

export async function createProfile(name?: string): Promise<ProfileInfo> {
  const info = await invoke<ProfileInfo>("create_profile", { name: name ?? null });
  await refreshProfiles();
  return info;
}

export async function renameProfile(id: string, name: string): Promise<ProfileInfo> {
  const info = await invoke<ProfileInfo>("rename_profile", { id, name });
  await refreshProfiles();
  return info;
}

export async function duplicateProfile(
  sourceId?: string,
  name?: string,
): Promise<ProfileInfo> {
  const info = await invoke<ProfileInfo>("duplicate_profile", {
    sourceId: sourceId ?? null,
    name: name ?? null,
  });
  await refreshProfiles();
  return info;
}

export async function deleteProfile(id: string): Promise<void> {
  await invoke("delete_profile", { id });
  await refreshProfiles();
}
