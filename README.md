# OVERTURE

A desktop **build studio** for the protection period of [OpenDominion](https://github.com/OpenDominion/OpenDominion) — plan your dominion's first 48 in-game hours (and the out-of-protection economy that follows) against a **bit-exact** simulation of the round-50 game.

OVERTURE pairs a deterministic Rust engine — a clean-room reimplementation of OpenDominion's per-tick rules, validated bit-for-bit against golden vectors emitted by the real game — with a Tauri desktop app. Edit your build hour by hour and watch land, resources, population, defense, and the leave-protection gate update instantly.

> **Unofficial.** OVERTURE is a fan-made tool. It is not affiliated with or endorsed by the OpenDominion project. See [NOTICE](NOTICE).

---

## What it is — and isn't

OVERTURE is the **simulator and planner**: you make the decisions, it tells you exactly what the game would do. It is **not** an auto-solver — there is no automated build optimizer in this release. It models the **protection period + post-OOP economy**, not the full multiplayer game.

### Modeled — and golden-validated against the real game

- Per-tick **production & decay**, **population / housing**, hour-0 opening fill, **employment**
- **Peasant & draftee growth**, **prestige**, **morale**
- All **building types** and **land types**
- **Explore**, **rezone**, **construct**, **train**, **daily bonuses**
- Common **and racial economic self-spells**
- **Improvements**, platinum↔ore **banking**, **draft rate**
- **Tech research**, **late-start** bonuses, **starvation** casualties
- The **leave-protection defense gate** and **defensive power** (raw + modded, incl. temple reduction)
- Standalone **combat / range calculators** (`engine/src/combat.rs`), golden-tested

### Not modeled (out of scope for this release)

- End-to-end **invasion / attacker** game loop
- **Espionage** / spy operations
- **Heroes**
- **Wonders**
- **Realm / war / government** and any multiplayer mechanics

---

## Install (prebuilt)

Download the latest build for your platform from the [Releases](../../releases) page.

These binaries are **unsigned** (no paid Apple/Microsoft signing certificate), so the OS will warn you the first time you open them. This is expected — unblock once and the app opens normally thereafter:

- **macOS** — right-click (or Control-click) the app → **Open** → **Open** in the dialog. (Or: System Settings → Privacy & Security → "Open Anyway".)
- **Windows** — on the SmartScreen prompt, click **More info** → **Run anyway**.

---

## Build from source

**Prerequisites**

- [Rust](https://rustup.rs) (stable)
- Tauri v2 system prerequisites for your OS — see <https://v2.tauri.app/start/prerequisites/> (macOS: Xcode command-line tools; Windows: WebView2 + MSVC build tools; Linux: webkit2gtk etc.)
- [Node.js](https://nodejs.org) — only to run the Tauri CLI via `npx` (there is no JS bundler; the frontend is hand-authored ES modules)

**Run the engine's bit-exact test suite** (fast, no system deps):

```sh
cargo test --manifest-path engine/Cargo.toml --tests
```

**Run the app in development:**

```sh
cd app
npx @tauri-apps/cli dev
```

**Build a release bundle** (`.app` / `.exe` / installer under `app/src-tauri/target/release/bundle/`):

```sh
cd app
npx @tauri-apps/cli build
```

---

## Architecture

```
engine/   bit-exact round-50 simulation (Rust library) — the single source of truth.
          Game data in data/ is embedded at compile time via include_dir, so the
          shipped binary needs no external files and can't be accidentally altered.
app/      Tauri v2 desktop app:
            src-tauri/src/main.rs  thin Rust adapter — calls the engine's public API only
            src/*.js               vanilla ES-module frontend (no bundler)
data/     round-50 game tables (races, techs, spells), mirrored from OpenDominion
```

The frontend never reaches around the engine: every number it shows comes from a `simulate` call into the Rust engine (or, in a browser preview without Tauri, from `app/src/mock.js`, which mirrors the same shapes for design work). The engine is treated as read-only — the adapter calls it, it never modifies engine logic.

---

## Fidelity & testing

The engine is validated **bit-for-bit** against golden vectors emitted by the real OpenDominion game (the oracle), checked in under `engine/tests/golden/`. Every mechanic has a golden vector before it's trusted. CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the engine's bit-exact suite on every change and type-checks the desktop app on macOS and Windows.

If a contribution changes engine behavior, it must keep these tests green — or add new golden vectors that justify the change.

---

## Updates

There is **no in-app auto-updater**. New versions are announced in the community Discord with a link to a [GitHub release](../../releases); download the new build and replace your copy.

*Maintainers:* cutting a release is `git tag vX.Y.Z && git push origin vX.Y.Z` — the [release workflow](.github/workflows/release.yml) builds the unsigned macOS (universal `.dmg`) and Windows (`.exe`/`.msi`) bundles and attaches them to a **draft** release for you to review and publish.

---

## Contributing

- The **engine is the source of truth.** UI and adapter code call it; they don't re-implement game math.
- Keep `cargo test --manifest-path engine/Cargo.toml --tests` green; add golden vectors for new mechanics.
- OVERTURE is **AGPL-3.0** — contributions are accepted under the same license. If you run a modified version as a network service, the AGPL requires you to offer users its source.

---

## Maintainer

Built and maintained by **lethal5808** — reach me on Discord (handle `lethal5808`); version announcements and support live in the community Discord.

---

## License & attribution

OVERTURE is licensed under the **GNU Affero General Public License v3** — see [LICENSE](LICENSE).

It derives from OpenDominion (also AGPL-3.0): the engine clean-room reimplements the game's round-50 rules, and `data/` mirrors its round-50 tables. Full provenance and third-party-component notes are in [NOTICE](NOTICE).
