// Minimal shared client state for the active install. Screens read/write the
// selected game_dir + locale here so navigation preserves the active project.
// This holds UI state only; all backend reads/writes go through Tauri commands
// (see src/lib/utils/invoke.ts and docs/adr/0003-repo-module-layout.md).

import { writable } from "svelte/store";

export interface ProjectState {
  gameDir: string | null;
  locale: string | null;
}

export const project = writable<ProjectState>({ gameDir: null, locale: null });
