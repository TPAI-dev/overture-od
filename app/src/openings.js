// openings.js — saveable OPENING templates (the hour-0 building placement).
//
// A template is a named snapshot of `plan.opening` ({ building: acres }) — just the starting
// placement, NOT a whole build. It's stampable onto any build, on any race (the engine validates
// the buildings on simulate). Distinct from the full build library (saves.js): a build is the entire
// 48h+ plan; a template is only the opening you reuse across builds.
//
// Persisted to localStorage, which works in BOTH the browser preview and the Tauri webview — so this
// needs NO backend command and ships identically in the public (no-swarm) and private builds.

const KEY = "overture.openings.v1";
const read = () => { try { return JSON.parse(localStorage.getItem(KEY)) || []; } catch { return []; } };
const write = (arr) => { try { localStorage.setItem(KEY, JSON.stringify(arr)); } catch (e) { console.warn("opening template save failed:", e); } };
const uid = () => Math.random().toString(36).slice(2, 9) + Date.now().toString(36);

// Keep only buildings actually placed (drop zero/garbage), so a template stores a clean placement.
function compact(opening) {
  const out = {};
  for (const [k, v] of Object.entries(opening || {})) {
    const n = Math.max(0, Math.floor(+v || 0));
    if (n > 0) out[k] = n;
  }
  return out;
}
export const openingAcres = (opening) => Object.values(compact(opening)).reduce((a, b) => a + b, 0);

// Newest first.
export function listOpenings() { return read().slice().sort((a, b) => (b.savedAt || 0) - (a.savedAt || 0)); }

// Upsert by (name, race): re-saving the same name for the same race overwrites it.
export function saveOpening(name, race, opening) {
  const arr = read();
  const clean = compact(opening);
  const existing = arr.find((o) => o.name === name && o.race === race);
  const entry = { id: existing ? existing.id : uid(), name, race: race || "", opening: clean, acres: openingAcres(clean), savedAt: Date.now() };
  write(existing ? arr.map((o) => (o === existing ? entry : o)) : [entry, ...arr]);
  return entry;
}
export function deleteOpening(id) { write(read().filter((o) => o.id !== id)); }
