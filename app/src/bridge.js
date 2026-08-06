// bridge.js — single seam between the UI and the engine.
// In Tauri: calls Rust commands that drive the UNTOUCHED engine crate.
// In a browser: falls back to the reactive mock so the design previews live.
import {
  simulate as mockSimulate,
  meta as mockMeta,
  invasionLandGain as mockInvasionLandGain,
} from "./mock.js";

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
    // Browser-preview-only fallback. Mirror the LIVE round-51 roster (21 races):
    // reworked races use their `*-rework` key (the live variant), classics/legacy
    // are excluded, and Planewalker is disabled by source data this round.
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

  async capabilities() {
    if (TAURI) return await invoke("capabilities");
    return {
      swarm: false,
      ruleset: {
        id: "round51", round: 51, sourceTag: "1.51.0",
        sourceCommit: "35b977df6b47fd24636f920657b5c4edb46bbff7",
      },
    };
  },

  // plan -> { rows:[49], final:{...} }
  async simulate(plan) {
    if (TAURI) return await invoke("simulate", { plan });
    return mockSimulate(plan);
  },

  // Preview a draft scenario event against the exact state at its scheduled
  // hour. The backend works on a temporary plan copy; nothing is committed.
  async previewEvent(plan, event) {
    if (TAURI) return await invoke("preview_event", { plan, event });
    const draft = typeof structuredClone === "function"
      ? structuredClone(plan)
      : JSON.parse(JSON.stringify(plan));
    draft.events = draft.events || [];
    const existingIndex = draft.events.findIndex((x) => x.id === event.id);
    if (existingIndex >= 0) draft.events[existingIndex] = event;
    else draft.events.push(event);
    const out = mockSimulate(draft);
    const row = out.rows.find((r) => (r.events || []).some((x) => x.id === event.id));
    const outcome = row && row.events.find((x) => x.id === event.id);
    if (!outcome) throw new Error("event preview did not reach its scheduled hour");
    return { outcome, row };
  },

  // Exact desktop estimate for a successful no-war, non-repeat invasion. The
  // Rust combat module owns the live piecewise formula and generated-land bonus.
  async invasionLandGain(attackerLand, targetLand) {
    if (TAURI) return await invoke("invasion_land_gain", { attackerLand, targetLand });
    return mockInvasionLandGain(attackerLand, targetLand);
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
  async exportExcel(plan) {
    if (!TAURI) throw new Error("Excel export is available in the desktop app");
    return await invoke("export_excel", { plan });
  },

  async autosave(plan) { if (!TAURI) return; try { await invoke("autosave", { plan }); } catch (_) {} },
  async latestAutosave() { return TAURI ? await invoke("latest_autosave") : null; },
};
