// Live per-operation progress, fed by the backend's `operation://progress` event
// stream (item-06b). The backend is the SOLE source of progress: the frontend
// never polls the filesystem or pipeline (UI-only; see AGENTS.md frontend architecture).
//
// The store maps each running operation id (`harvest`, `attribution`, ...) to its
// latest `OperationProgress`. A terminal phase (`done` / `cancelled` / `error`)
// removes the entry so a finished bar disappears. A single module-level listener is
// installed lazily on first subscribe, so the shell (and every screen) shares one
// event subscription regardless of how many components read the store.

import { writable } from "svelte/store";
import { listen } from "$lib/utils/invoke";
import type { OperationProgress } from "$lib/types";

/** op id -> its latest progress update (only currently-running operations). */
export type ProgressMap = Record<string, OperationProgress>;

const store = writable<ProgressMap>({});

// The terminal phases that clear an operation from the map.
const TERMINAL = new Set(["done", "cancelled", "error"]);

let started = false;

/**
 * Install the single backend-event listener (idempotent). Call once from the app
 * shell (`+layout.svelte`) on mount; safe to call again (subsequent calls no-op).
 * Returns the store so callers can `subscribe`/`$progress` as usual.
 */
export function startProgressListener(): typeof store {
  if (started) return store;
  started = true;
  void listen<OperationProgress>("operation://progress", (p) => {
    store.update((m) => {
      const next = { ...m };
      if (TERMINAL.has(p.phase)) {
        delete next[p.op];
      } else {
        next[p.op] = p;
      }
      return next;
    });
  });
  return store;
}

/** The reactive progress map (subscribe to react to any operation's updates). */
export const progress = store;
