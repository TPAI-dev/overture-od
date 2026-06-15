// techtree.js — the TECHS tab: an interactive replica of the round-50 research graph.
//
// Renders meta.techs (key, name, x, y, perks, requires) as an SVG node graph colored by the
// per-hour state at the playhead (researched / available / locked), with a detail panel and
// click-to-research wired to the SAME per-hour research action the editor uses. This is a pure
// VIEW: app.js supplies the data + an onResearch(key) callback (which records undo, commits the
// {type:research} action, and re-simulates). The engine remains the source of truth for tech
// cost, prerequisites, and perk effects — the graph never re-derives game math, it only reflects
// the engine's row state and schedules the legal research action.
//
// Layout: rendered inside the (wide) action-editor popover, so it uses the tree's NATURAL
// orientation — tech.x → horizontal (21 columns), tech.y → vertical (11 rows) — as a landscape
// graph. opts.width sizes the coordinate space; opts.transpose swaps the axes for a narrow host.

const int = (n) => Math.round(n || 0).toLocaleString("en-US");
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

// Perks whose value is a FLAT amount (acres/hours/units), not a percent. Everything else in the
// round-50 tech data is a percentage modifier. Display-only heuristic — it does not touch the
// engine math, which applies each perk's real semantics regardless of how we label it here.
const FLAT_PERKS = new Set([
  "enemy_burning_duration", "boat_capacity", "boat_production", "barracks_housing",
  "extra_barren_max_population", "mana_production_raw", "wartime_mana_production_raw",
]);
function perkLine(key, val) {
  const label = key.replace(/_/g, " ");
  const sign = val > 0 ? "+" : "−";
  const mag = Math.abs(val);
  const unit = FLAT_PERKS.has(key) ? "" : "%";
  return `<span class="tt-pk-v ${val >= 0 ? "pos" : "neg"}">${sign}${mag}${unit}</span> ${esc(label)}`;
}

// Selection survives re-renders (playhead scrub / post-research re-render) so the detail panel
// keeps inspecting the same node. Reset implicitly when the key isn't in the current race's tree.
let lastSelected = null;

export function renderTechTree(container, opts) {
  const { techs, researched, points, cost, targetHour, onResearch, width = 680, transpose = false } = opts;
  const byKey = new Map(techs.map((t) => [t.key, t]));
  const haxis = (t) => (transpose ? t.y : t.x); // horizontal coordinate
  const vaxis = (t) => (transpose ? t.x : t.y); // vertical coordinate
  const hvals = [...new Set(techs.map(haxis))].sort((a, b) => a - b);
  const vmin = Math.min(...techs.map(vaxis)), vmax = Math.max(...techs.map(vaxis));
  const vSpan = (vmax - vmin) || 1;
  const nH = hvals.length;

  const W = width, padX = 22, padY = 20, vUnit = 14, R = 8;
  const H = padY * 2 + vSpan * vUnit;
  const cx = (t) => padX + (nH <= 1 ? 0 : (hvals.indexOf(haxis(t)) / (nH - 1)) * (W - 2 * padX));
  const cy = (t) => padY + ((vaxis(t) - vmin) / vSpan) * (H - 2 * padY);

  const stateOf = (t) =>
    researched.has(t.key) ? "done"
      : (!t.requires || !t.requires.length || t.requires.some((r) => researched.has(r))) ? "open"
        : "lock";

  let edges = "";
  for (const t of techs) {
    for (const req of t.requires || []) {
      const p = byKey.get(req);
      if (!p) continue;
      const lit = researched.has(req); // a satisfied prerequisite edge is highlighted
      edges += `<line x1="${cx(p).toFixed(1)}" y1="${cy(p).toFixed(1)}" x2="${cx(t).toFixed(1)}" y2="${cy(t).toFixed(1)}" class="tt-edge ${lit ? "on" : ""}"/>`;
    }
  }
  let nodes = "";
  for (const t of techs) {
    const st = stateOf(t);
    nodes += `<circle data-k="${t.key}" cx="${cx(t).toFixed(1)}" cy="${cy(t).toFixed(1)}" r="${R}" class="tt-node tt-${st}"><title>${esc(t.name)}</title></circle>`;
  }

  const hourLabel = targetHour != null
    ? `research @ hour ${String(targetHour).padStart(2, "0")}`
    : "scrub to an hour 1+ to research";
  container.innerHTML = `
    <div class="tt-wrap">
      <div class="tt-head">
        <span class="tt-hour">${hourLabel}</span>
        <span class="tt-pts"><b>${int(points)}</b> pts · next ${int(cost)}</span>
      </div>
      <div class="tt-legend">
        <span class="tt-l"><i class="tt-sw tt-done"></i>researched</span>
        <span class="tt-l"><i class="tt-sw tt-open"></i>available</span>
        <span class="tt-l"><i class="tt-sw tt-lock"></i>locked</span>
      </div>
      <div class="tt-graph">
        <svg viewBox="0 0 ${W} ${Math.round(H)}" width="100%" role="img" aria-label="Tech tree graph">${edges}${nodes}</svg>
      </div>
      <div class="tt-detail" id="ttDetail"><div class="tt-detail-empty">click a tech node to inspect it</div></div>`;

  const detail = container.querySelector("#ttDetail");
  function showDetail(key) {
    const t = byKey.get(key);
    if (!t) return;
    lastSelected = key;
    container.querySelectorAll(".tt-node").forEach((n) => n.classList.toggle("sel", n.dataset.k === key));
    const st = stateOf(t);
    const perks = Object.entries(t.perks || {}).map(([k, v]) => `<div class="tt-perk">${perkLine(k, v)}</div>`).join("")
      || `<div class="tt-perk dim">no perks</div>`;
    const reqHtml = (t.requires || []).map((r) => {
      const p = byKey.get(r);
      return `<span class="tt-req ${researched.has(r) ? "met" : ""}">${p ? esc(p.name) : esc(r)}</span>`;
    }).join("");
    const canAfford = points >= cost;
    const status =
      st === "done" ? `<span class="tt-status done">✓ researched</span>`
        : st === "lock" ? `<span class="tt-status lock">locked</span>`
          : targetHour == null ? `<span class="tt-status lock">no research hour</span>`
            : canAfford ? `<span class="tt-status open">available</span>`
              : `<span class="tt-status warn">need ${int(cost - points)} more</span>`;
    const canResearch = st === "open" && targetHour != null && canAfford;
    detail.innerHTML = `
      <div class="tt-d-head"><b>${esc(t.name)}</b>${status}</div>
      <div class="tt-d-perks">${perks}</div>
      ${reqHtml
        ? `<div class="tt-d-req"><span class="tt-d-lab">requires any one</span><div class="tt-reqs">${reqHtml}</div></div>`
        : `<div class="tt-d-req dim">starting tech — no prerequisite</div>`}
      <button class="tt-research" id="ttGo" ${canResearch ? "" : "disabled"}>⌁ research · ${int(cost)} pts</button>`;
    const go = detail.querySelector("#ttGo");
    if (go && canResearch) go.onclick = () => onResearch(key);
  }

  container.querySelectorAll(".tt-node").forEach((n) => n.addEventListener("click", () => showDetail(n.dataset.k)));
  if (lastSelected && byKey.has(lastSelected)) showDetail(lastSelected);
}
