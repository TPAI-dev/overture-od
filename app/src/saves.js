// saves.js — named build library. Persists to localStorage, which works in BOTH the
// browser preview and the Tauri webview (separate stores; use export/import JSON to
// move builds between them or back them up). A build = the whole plan object
// {race, dpTarget, opening, hours, oopActions}.

const KEY = "overture.builds.v1";
const read = () => { try { return JSON.parse(localStorage.getItem(KEY)) || []; } catch { return []; } };
const write = (arr) => { try { localStorage.setItem(KEY, JSON.stringify(arr)); } catch (e) { console.warn("save failed:", e); } };
const clone = (o) => JSON.parse(JSON.stringify(o));
const uid = () => Math.random().toString(36).slice(2, 9) + Date.now().toString(36);
const int = (n) => Math.round(n || 0).toLocaleString("en-US");
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const fmtDate = (ts) => { const d = new Date(ts); return d.toLocaleDateString(undefined, { month: "short", day: "numeric" }) + " " + d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }); };

// Light structural check on a plan before we apply it. The engine does the REAL validation on
// simulate (rejects bad opening builds, unknown buildings, etc. — see plan::opening_build_error);
// this only rejects obvious garbage — wrong top-level shape, or `hours` that isn't a list of
// action-lists — so a malformed/hand-edited *.overture.json can't wedge the UI before the engine
// ever runs. Anything structurally plausible is passed through and validated for real downstream.
const isPlanShape = (p) =>
  !!p && typeof p === "object" && !Array.isArray(p) &&
  Array.isArray(p.hours) && p.hours.every((h) => Array.isArray(h)) &&
  (p.race == null || typeof p.race === "string") &&
  (p.oopActions == null || Array.isArray(p.oopActions)) &&
  (p.opening == null || (typeof p.opening === "object" && !Array.isArray(p.opening)));

export function createSaves(deps) {
  // deps: { getPlan(), applyPlan(plan), getStats(plan)->Promise<{committed,dp,feasible}>, live:bool,
  //         saveBuild(name,plan)->path|null, listSaves()->[{name,path,race,savedAt,dpTarget}]|null,
  //         loadBuild(name)->plan|null, deleteSave(name) }
  //   The save*/listSaves/loadBuild fns return null in the browser → localStorage fallback; in the
  //   desktop app they read/write real files under ~/Documents/OVERTURE/saves.
  const btn = document.getElementById("buildsBtn");
  const pop = document.getElementById("buildsPop");
  const scrim = document.getElementById("buildsScrim");
  const fileInput = document.getElementById("buildsImport");
  let note = "";
  let savedBuilds = [];         // normalized SAVED list — filesystem (desktop) or localStorage (browser)

  // Populate `savedBuilds` from the filesystem (desktop) or localStorage (browser). FS entries are
  // normalized to the same shape, tagged `_fs` so load/delete route to the backend.
  async function loadSaves() {
    let fs = null;
    if (deps.listSaves) { try { fs = await deps.listSaves(); } catch (_) { fs = null; } }
    savedBuilds = fs
      ? fs.map((e) => ({ id: e.path, path: e.path, name: e.name, race: e.race, dpTarget: e.dpTarget, savedAt: e.savedAt, _fs: true }))
      : read(); // localStorage entries: {id, name, race, savedAt, stats, plan}
  }

  async function render() {
    const plan = deps.getPlan();
    pop.innerHTML = `
      <div class="bl-head"><span>BUILD LIBRARY</span><button class="bl-x" id="blX" aria-label="close">✕</button></div>
      <div class="bl-save">
        <input id="blName" placeholder="name this ${esc(plan.race)} build…" autocomplete="off">
        <button id="blSave">save</button>
      </div>
      ${note ? `<div class="bl-note">${esc(note)}</div>` : ""}
      <div class="bl-section">SAVED BUILDS</div>
      <div class="bl-list">${savedBuilds.length ? savedBuilds.map((b, i) => `
        <div class="bl-item">
          <div class="bl-meta"><b>${esc(b.name)}</b><span>${esc(b.race || "")}${b.stats ? ` · ${int(b.stats.committed)} land · ${b.stats.feasible ? "✓ " : ""}${int(b.stats.dp)} DP` : (b.dpTarget ? ` · ${int(b.dpTarget)} DP target` : "")} · ${fmtDate(b.savedAt)}</span></div>
          <div class="bl-act"><button data-load="${i}">load</button><button class="bl-del" data-del="${i}" title="delete">✕</button></div>
        </div>`).join("") : '<div class="bl-empty">no saved builds yet — name one above and hit save</div>'}
      </div>
      <div class="bl-foot"><button id="blExport">⇩ export current</button><button id="blImport">⇧ import file</button></div>`;
    pop.querySelector("#blX").onclick = close;
    pop.querySelector("#blSave").onclick = saveCurrent;
    const nameI = pop.querySelector("#blName");
    nameI.onkeydown = (e) => { if (e.key === "Enter") saveCurrent(); };
    nameI.focus();
    pop.querySelectorAll("[data-load]").forEach((b) => (b.onclick = () => doLoad(+b.dataset.load)));
    pop.querySelectorAll("[data-del]").forEach((b) => (b.onclick = () => doDelete(+b.dataset.del)));
    pop.querySelector("#blExport").onclick = exportCurrent;
    pop.querySelector("#blImport").onclick = () => fileInput.click();
  }

  function defaultName() {
    const race = deps.getPlan().race;
    return `${race} build ${savedBuilds.filter((b) => b.race === race).length + 1}`;
  }

  async function saveCurrent() {
    const raw = (pop.querySelector("#blName").value || "").trim();
    const name = raw || defaultName();
    const plan = clone(deps.getPlan());
    let savedFs = false;
    if (deps.saveBuild) { try { savedFs = !!(await deps.saveBuild(name, plan)); } catch (_) { savedFs = false; } }
    if (!savedFs) {
      // localStorage fallback (browser preview)
      let stats = null;
      try { stats = await deps.getStats(plan); } catch (_) {}
      const builds = read();
      const existing = builds.find((b) => b.name === name);
      const entry = { id: existing ? existing.id : uid(), name, race: plan.race, savedAt: Date.now(), stats, plan };
      write(existing ? builds.map((b) => (b.name === name ? entry : b)) : [entry, ...builds]);
    }
    note = `saved “${name}”`;
    await loadSaves();
    render();
  }

  async function doLoad(i) {
    const b = savedBuilds[i];
    if (!b) return;
    let plan = b.plan;
    if (b._fs && deps.loadBuild) { try { plan = await deps.loadBuild(b.name); } catch (_) { plan = null; } }
    if (isPlanShape(plan)) { await deps.applyPlan(clone(plan)); close(); }
    else { note = "could not load — file is not a valid OVERTURE build"; render(); }
  }
  async function doDelete(i) {
    const b = savedBuilds[i];
    if (!b) return;
    if (b._fs) { if (deps.deleteSave) { try { await deps.deleteSave(b.name); } catch (_) {} } }
    else { write(read().filter((x) => x.id !== b.id)); }
    note = "";
    await loadSaves();
    render();
  }

  function exportCurrent() {
    const plan = deps.getPlan();
    const blob = new Blob([JSON.stringify({ overture: 1, savedAt: Date.now(), plan }, null, 2)], { type: "application/json" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${plan.race || "build"}-overture.json`;
    document.body.appendChild(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
    note = "exported current build to file";
    render();
  }

  fileInput.addEventListener("change", async () => {
    const f = fileInput.files[0];
    if (!f) return;
    try {
      const data = JSON.parse(await f.text());
      const plan = data && data.plan ? data.plan : data;
      if (isPlanShape(plan)) { await deps.applyPlan(plan); close(); }
      else note = "import failed — not an OVERTURE build file";
    } catch (e) { note = "import failed — invalid JSON"; }
    fileInput.value = "";
    if (!pop.hidden) render();
  });

  async function open() { pop.hidden = false; scrim.hidden = false; note = ""; await loadSaves(); render(); }
  function close() { pop.hidden = true; scrim.hidden = true; }
  function toggle() { pop.hidden ? open() : close(); }
  btn.onclick = toggle;
  scrim.onclick = close;
  return { toggle, close };
}
