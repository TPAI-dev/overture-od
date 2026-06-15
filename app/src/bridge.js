// bridge.js — single seam between the UI and the engine.
// In Tauri: calls Rust commands that drive the UNTOUCHED engine crate.
// In a browser: falls back to the reactive mock so the design previews live.
import { simulate as mockSimulate, meta as mockMeta } from "./mock.js";

const TAURI = typeof window !== "undefined" && !!(window.__TAURI__ && window.__TAURI__.core);
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

export const engine = {
  live: TAURI,
  source: TAURI ? "engine (bit-exact)" : "preview mock",

  // In Tauri (desktop), the engine is the source of truth: any command error must
  // FAIL VISIBLY rather than silently degrade to mock data behind a "bit-exact" badge.
  // The mock fallback is browser-preview-only (!TAURI).
  async races() {
    if (TAURI) return await invoke("races");
    // Browser-preview-only fallback. Mirror the LIVE round-50 roster (21 races):
    // reworked races use their `*-rework` key (the live variant), classics/legacy
    // are excluded, and Planewalker is disabled this round (see data/round50.json).
    return [
      "human", "dwarf", "goblin", "halfling", "orc", "sylvan", "lizardfolk",
      "firewalker", "icekin", "gnome", "troll", "merfolk", "demon", "lycanthrope",
      "vampire", "dark-elf-rework", "kobold-rework", "nomad-rework", "spirit-rework",
      "undead-rework", "wood-elf-rework",
    ];
  },

  // race -> { units:[…], techs:[…], buildingLand:{…} }  (static labels for the editor)
  async meta(race) {
    if (TAURI) return await invoke("meta", { race });
    return mockMeta(race);
  },

  // plan -> { rows:[49], final:{...} }
  async simulate(plan) {
    if (TAURI) return await invoke("simulate", { plan });
    return mockSimulate(plan);
  },

  // ───────── build storage + autosave (desktop filesystem under ~/Documents/OVERTURE) ─────────
  // These return null in the browser preview so saves.js falls back to localStorage. The desktop
  // app reads/writes real *.overture.json files via the Rust backend (no filesystem in a webview).
  async saveBuild(name, plan) { return TAURI ? await invoke("save_build", { name, plan }) : null; },
  async listSaves() { return TAURI ? await invoke("list_saves") : null; },
  // load/delete take the save NAME (not a path): the backend resolves it under
  // ~/Documents/OVERTURE/saves so a caller can never reach an arbitrary file.
  async loadBuild(name) { return TAURI ? await invoke("load_build", { name }) : null; },
  async deleteSave(name) { return TAURI ? await invoke("delete_save", { name }) : null; },
  async autosave(plan) { if (!TAURI) return; try { await invoke("autosave", { plan }); } catch (_) {} },
  async latestAutosave() { return TAURI ? await invoke("latest_autosave") : null; },
};
