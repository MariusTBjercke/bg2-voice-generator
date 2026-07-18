// Active profile + registry cache for the shell switcher. UI state only; all
// persistence goes through Tauri profile commands.

import { get, writable } from "svelte/store";
import { invoke } from "$lib/utils/invoke";
import type { ProfileInfo, ProfileRegistry } from "$lib/types";
import { project } from "$lib/stores/project";
import { ensureGameDir } from "$lib/stores/results";

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

/** Switch profile, clear result caches, and re-hydrate game_dir from the new DB. */
export async function switchToProfile(id: string): Promise<void> {
  const current = get(profiles).active?.id;
  if (current === id) return;
  await invoke<ProfileInfo>("switch_profile", { id });
  // Drop UI caches keyed by the previous install/profile.
  ensureGameDir(null);
  project.set({ gameDir: null, locale: null });
  await refreshProfiles();
  try {
    const gameDir = (await invoke<string | null>("get_setting", { key: "game_dir" })) ?? null;
    project.update((p) => ({ ...p, gameDir }));
    if (gameDir) ensureGameDir(gameDir);
  } catch {
    // Setup will surface errors
  }
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
