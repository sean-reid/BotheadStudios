# Changelog

All notable changes to `greenfield-engine` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
See [`docs/03-versioning.md`](docs/03-versioning.md) for our versioning policy — it matters
because **we are our own first customers** and pin exact engine versions in our games.

## [Unreleased]

- **GPU Barnes–Hut solver, built + verified; measured NOT worth wiring in-browser** (`tools/gpu-bh-verify` +
  `shaders/bh_gravity.wgsl`, `docs/36`→`docs/37`) — the full LBVH self-gravity pipeline (adaptive bbox → Morton
  → interim CPU sort → Karras tree → atomic-free bottom-up COM → robust-MAC θ-traversal) as verified WGSL
  compute kernels. Correctness proven stage-by-stage against CPU references (bbox exact, Morton bit-exact, tree
  structural, COM <1e-6, θ=0.5 RMS 0.70 %, θ→0 recovers the exact direct sum). **Finding:** on the RTX 2070 GPU
  direct-sum beats Barnes–Hut until **N≈128k** (BH is 0.6–0.9× at N≤32k); asymptotics are correct (direct
  O(N²), BH O(N log N)) but the crossover sits far above the browser (N≤20k) and offline (N≈35k) regimes, so BH
  would *reduce* in-browser fps. Recommendation: keep direct-sum for N≤~100k; BH's niche is high-N offline
  convergence (N≳128k). No engine code changed; the GPU radix sort (docs/36 stage 3) is deferred pending the
  high-N decision. See `docs/37`.
- **GPU impact read-back + live disk stats** (`gpu_sph.rs`, `docs/35` — the GPU-path migration) — `GpuSph`
  gained two-phase async GPU→CPU read-back, and the browser birth scene now shows the live orbiting-disk
  provenance (mass, Earth %, remnant radius, largest self-bound clump) from the read-back particle field. The
  first increment of unifying the scenes onto the one GPU SPH path (retiring the CPU `Aggregate`).
- **EOS abstraction — one pressure law across air and rock** (`eos.rs`, `docs/33` stage 5) — a new `Eos` enum
  (`Tillotson` | `IdealGas`) with `pressure`/`sound_speed_sq`, so the shared SPH machinery is parameterized by
  the equation of state instead of hardcoding it. `hydrostatic::HydroBody` now carries `Vec<Eos>` (was
  `Vec<Tillotson>`) — EOS-agnostic, the seam to fold the duplicated `AirField`/vapor SPH loops onto one code
  path. Byte-identical to the old Tillotson path (verified: differentiated planet settles to the same central
  pressure; new ideal-gas dispatch test).
- **GPU SPH-EOS-gravity kernel, verified** (`shaders/sph_step.wgsl` + `tools/sph-verify`, `docs/33` stage 4a)
  — the space-band self-gravitating condensed-matter force step (SPH density + Tillotson pressure + Monaghan
  artificial viscosity + direct self-gravity + du/dt) ported to WGSL compute, for the giant impact at N~10⁵.
  Verified headless on the RTX 2070 (native Vulkan wgpu) against an independent f64 CPU computation of the
  same equations to f32 precision (RMS rel error 1.9e-6). Stage 4b adds a **spatial-hash neighbour grid** for
  the short-range SPH (O(N) not O(N²)), also verified exact — with a cell-membership guard that defeats the
  hash-collision double-counting. Stage 4c.1 adds the **KDK leapfrog integration loop** (`cs_kick_drift` +
  `cs_kick`, energy-conserving, matching the CPU `HydroBody::step`), verified over 50 fixed-dt steps against
  an f64 CPU reference (final-state RMS pos 3.1e-4 / vel 5.7e-4 / u 5.1e-4 — tracking, not diverging). Stage
  4c.2 adds `tools/impact-run` (GPU relaxation `cs_relax` + adaptive-dt KDK impact + provenance) and runs the
  deformable-Earth giant impact at N up to 35 000 on the RTX 2070 (minutes, vs the CPU's ~2100-particle cap):
  energy conserved to 0.3–0.5 % over ~10 h of aftermath, disk mass (~0.13–0.19 M☾), remnant radius, and
  escape speed robust across runs. The disk's Earth-derived *fraction* (28–50 % in samples, vs the CPU's
  58 %) has large run-to-run scatter — two identical N=35000 runs gave 50 % and 29 % (GPU-non-determinism ×
  chaotic amplification) — so it reproduces the deformable-Earth mechanism (Earth material orbits, docs/31)
  but the precise fraction remains an IOU pending an ensemble average + deterministic reduction + higher N.
  Stage 4c.3 adds the
  **accretion / growth operator** (`accretion.rs`): friends-of-friends bound-clump detection gated on genuine
  self-boundedness AND the remnant's Roche limit, promoting each qualifying clump to one body at its COM —
  conserving mass, momentum, and centre of mass exactly (TDD-verified to <1e-12), the growth law a round Moon
  needs. Stage 4c.4 completes stage 4c: the deformable-Earth giant impact now **runs live in the browser**
  (`gpu_sph.rs` hosts `sph_step.wgsl` on the space-band WebGPU device; `sph_render.wgsl` draws the particle
  field instanced; `OrbitDemo::start_gpu_impact()` / a "GPU Impact" button triggers it) — rig-watch verified
  on the RTX 2070: two differentiated bodies collide into a remnant + a two-provenance debris disk, stable at
  interactive frame rates.
- **Deformable-Earth giant impact — the isotopic crisis, re-measured** (`hydrostatic.rs`, `docs/33` stage
  3) — a full thermodynamic SPH giant impact between two real EOS particle bodies: the SPH internal-energy
  equation + Monaghan artificial viscosity (shock capture) + an energy-conserving KDK leapfrog with an
  adaptive Courant timestep (verified: a relaxed head-on collision conserves total energy to ~3% and
  shock-heats, IE up 4.9×). Then the payoff: a differentiated Theia into a **deformable differentiated
  proto-Earth** yields an orbiting disk that is **58% EARTH-derived** — versus the rigid-boundary ceiling of
  7–12% (docs/31). With Earth as real matter that sheds its own mantle, Earth material dominates the disk —
  the direction the isotopic crisis demands (docs/28 root-cause #1 dissolved). Sub-Earth scale + coarse N:
  the direction, not a converged number (the value awaits the GPU N, stage 4).
- **Self-gravitating EOS body — a particle planet in hydrostatic equilibrium** (`hydrostatic.rs`, `docs/33`
  stage 2) — composes the shared kernels (`eos::Tillotson` + the SPH kernel + `bhtree` self-gravity) into a
  cloud of particles that holds itself up under its own gravity via EOS pressure, instead of the rigid
  analytic boundary the impact scene uses (docs/28 #1). **Single-material (2a):** a 1500 km basalt body
  settles with pointwise hydrostatic balance dP/dr=−ρg (rel 0.00–0.01). **Differentiated (2b):** an
  Earth-mass iron-core + basalt-mantle body (equal-mass particles + adaptive smoothing length, the Genda
  2012 method) COMPRESSES (RMS 5709→3973 km, no puff-up), stays stratified (core ρ 15,591 vs mantle 5534
  kg/m³), holds balance (rel 6%), and reaches a central pressure of 572 GPa — the order of Earth's real 364
  GPa. Iron EOS uses the verified Wissing & Hobbs 2020 compressed-branch refit. The prerequisite for a
  deformable Earth; folds into `Aggregate` at unification.
- **Tillotson condensed-matter EOS** (`eos.rs`, `docs/33` stage 1) — `P(ρ, u)` across cold / shock-
  compressed / decompressed / vapor states in one closure (Tillotson 1962; Melosh 1989; Benz et al. 1989),
  with cited parameters for granite, basalt, peridotite (dunite/olivine), and iron. `pressure`,
  `sound_speed_sq`, `for_material`. The missing "matter under its own pressure" law — solids previously
  resisted compression only via a linear-elastic contact penalty and planet densities were declared
  constants. Verified: cold reference P≈0, cold compression yields the real bulk modulus A, monotonic
  stiffening to GPa scale, hot expansion → the ideal-gas limit, continuity across vaporization, km/s sound
  speed. Not yet wired into a scene (stage 2 builds the self-gravitating planet on it).
- **Architecture map, CLAUDE.md, and realignment plan** (`docs/32`, `docs/33`, `CLAUDE.md`) — a durable
  orientation for future sessions (module-by-module with `file:line` anchors; the shared-laws-vs-forked-
  solvers map; the EOS inventory confirming NO condensed-matter EOS exists) plus a staged plan to realign
  the engine to its principles: one particle/material module, one Tillotson EOS spanning solid→liquid→vapor,
  and energy-tiered calculations (T0 bulk → T3 full-EOS-shock, selected by energy density). No code change.
- **Proto-Earth spin + the isotopic crisis** (`docs/31`) — the excavated Earth cap is surface mantle that
  co-rotated before the impact, so `build_impact_debris_scaled` now takes an `earth_omega` and gives each
  target grain its co-rotating velocity `ω × (pos − centre)` before the ploughing loft (`ω = 0` is
  byte-identical to before); the scene converts `spin_l → ω = L/I` and passes it, default zero (unknown
  IC, flagged — no on-screen change). MEASURED (`a_fast_spinning_protoearth_makes_the_disk_earth_derived`):
  a fast-spinning proto-Earth (2.3 h day, Ćuk & Stewart 2012) does NOT Earth-enrich the disk — it grows the
  whole bound disk (1.40 → 2.59 M☾) but the Earth fraction falls (12 % → 7 %), because Earth is a rigid
  boundary (docs/28 #1) and only the small excavated cap can reach the disk. The honest resolution needs
  Earth-as-matter or vapor-phase mixing, not target spin. Physics deciding against the hypothesis, recorded.
- **Accelerated particle compute module** (`docs/30`) — a reusable O(N log N) substrate for ANY particle
  system (weather, clouds, fluids, not just impact), each stage proven against its exact/θ-bounded
  reference so speed never changes the answer. **Neighbour grid** (`neighbors.rs`): O(N) short-range pair
  finding, wired into the contact + SPH loops (`grid_finds_exactly_the_brute_force_pairs`,
  `contact_grid_matches_brute_force`). **Barnes–Hut self-gravity** (`bhtree.rs`): octree COM grouping at
  θ=0.5 turns O(N²) gravity into O(N log N), same softening as the direct sum
  (`barnes_hut_matches_brute_force_within_theta_bound` — RMS < 1%, θ→0 exact). **Block timesteps**
  (`aggregate.rs`): per-particle timestep criterion + hierarchical block-KDK `step_block` — the quiescent
  disk coasts while the shocked/vapor core sub-steps, with a subset-force pass
  (`accelerations_masked` + `BarnesHut::accelerations_active`) recomputing gravity only for the bodies
  kicked this sub-step, and full thermo (extracted to `apply_thermo`) run each sub-step. **5.5× faster**
  on an aftermath-shape cloud (`step_block_speedup_bench`) while reproducing the coupled impact disk
  (`birth_impact_with_step_block_reproduces_the_disk` — global 0.772 vs block 0.788 M☾). Wired into the
  space scene and running at high N (512 debris + 1024 cap, cap-mass summed from real per-grain masses).
- **Agent-watches-the-scene tooling** — `rig/birth_shot.mjs` screenshots birth.html under headless
  Chromium at timed marks so the agent can see the disk form; a "📷 Share view" button on the space band
  POSTs the live canvas to a receiver. (Public-site receiver `tools/shot-server.mjs` staged, not installed.)
- **Vapor SPH pressure field + latent-heat reservoir** (`docs/26`/`27`, `docs/28` item 5) — impact vapor
  now expands and self-cools as a real gas: cubic-spline SPH density, `P=ρ·R_s·T`, a symmetric
  momentum-conserving pressure force, and a PdV energy equation (expansion does work → the gas cools).
  Pressure reads the *thermal* temperature `T − L_v/c` so the vaporization latent heat is not
  double-counted as pressure. Replaces the vapor "overlap hack" (a docs/23 fudge). Test:
  `vapor_sph_expands_and_cools_conserving_energy` (80k → 18.5k K, energy conserved).
- **Momentum-conserving loft in the shared particle physics** (`granular::plough_loft`, `docs/28` step 3)
  — when a fast body ploughs slower target matter, the along-track momentum is shared inelastically toward
  the impactor↔cap centre-of-mass velocity (the physical maximum drag, no dial) and Σ(m·v) is exactly
  conserved. This is what makes the Moon **Earth-derived** — target material now lofts into the bound disk
  (Earth 0.083 M☾ aloft, up from a dead 0.000 at every resolution) once the cap mass is physical. One law
  for every band (space wired; terrain a flagged follow-up). Tests:
  `plough_loft_conserves_momentum_and_lofts_the_lighter_target`, and the disk provenance guardrails.
- **Materials-honest per-grain contact** (`docs/23`) — the aggregate contact law reads each grain's
  material (`Contact::mix` per pair: radius arithmetic-mean, stiffness harmonic-mean, damping/friction
  geometric-mean, cohesion min), so iron collides as iron and peridotite as peridotite instead of every
  grain being bulk basalt. Fixes the over-massed excavation cap — grain mass is now real `ρ·V` at the
  local density (`furrow_target_grains`), ≈0.31× the impactor rather than a bookkeeping 2×. Tests:
  `contact_mix_is_idempotent_and_bounded`, `mixed_material_contact_conserves_momentum`.
- **Bodies as particle aggregates** (`docs/21`) — the gravitational skeleton for making celestial
  destruction a *simulation, not a mock*. `aggregate.rs`: a body is a cloud of particles bound by
  softened N-body self-gravity; `binding_energy`, `kinetic_energy_com`, `rms_radius`, `com`. A cold
  cloud holds together (emergent cohesion/roundness) and an energy kick above its binding energy
  disrupts it (emergent shatter). Material/thermal per particle, impact coupling, and rendering staged.
  Tests: `aggregate::a_self_gravitating_cloud_holds_together`, `energy_above_binding_disrupts_it`.
- **Phase classes integrated into `matter::impact`** (`docs/20`) — each ejecta is classified via
  `damage::classify` (Fractured / Melted / Vaporized) from the thermodynamic thresholds; vaporized
  matter expands away fast (gas/plasma). Crater extent unchanged (LOD bridge intact). Test:
  `matter::a_colossal_impact_vaporizes_the_core`.
- **Moon-speed HUD readout** (km/s relative to Earth) in the space band — confirms there's no drag /
  terminal velocity in vacuum (a true Drop climbs to ~11 km/s at impact; a partial brake slows at
  apogee by Kepler's 2nd law).
- **Glowing molten ejecta + a Meteor control** (`docs/20`) — the first visual of impact damage. Impact
  ejecta carry `temp_k`; heat peaks at the contact and falls to cold at the crater rim (centre melts,
  rim is cold rubble). `emission::incandescence` maps temperature → a black-body glow (red→white) that
  the particle shader *adds*, so molten debris self-illuminates even on the dark side. Fire it with the
  `☄`/`m` **Meteor** button in the terrain slice (`Engine::meteor`). Tests:
  `emission::cold_matter_does_not_glow_and_hotter_glows_brighter_and_whiter`,
  `matter::a_big_impact_melts_the_centre_and_leaves_the_rim_cold`. (Crater extent is physical; ejecta
  temperature is a first visual model, not yet energy-conserved — the celestial→voxel fly-in stays staged.)
- **Impact thermodynamics — fracture/melt/vaporize** (`docs/20`). One data-driven response: an impact
  deposits energy density (J/m³), and `damage::classify` compares it to a material's own thresholds —
  fracture strength → melt energy `ρ(cΔT+L_f)` → vaporization energy — returning
  `Intact/Fractured/Melted/Vaporized`. Because the density falls with distance, one event yields all
  four at different radii (a scale-of-detail test too). Added optional `Material.thermal` (specific
  heat, melt/boil points, latent heats) with cited data for basalt, granite, iron, water; materials
  without it can only fracture. Test: `damage::impact_fractures_then_melts_then_vaporizes_by_energy_density`.
  Integration into the impact operator and the visual (incandescent melt, vapor plume, fly-in crater)
  are staged.
- **Two-moon stress-test scene** (`/twomoons.html`). Two moons on the same orbit, opposite sides of the
  Earth, de-orbited both at once. `OrbitDemo` generalized from one moon to N (per-moon uniforms,
  lighting, framing; Earth-vs-each-moon collision with both impact energies summed); `brake_moon` /
  `drop_moon` act on all moons; focus cycles Earth → Moon A → Moon B. Added to the scene picker; the
  moon count comes from `<body data-moons>` so both space pages share one script.
- **LOD-adaptive damage — the crater bridge** (`docs/19`). A damage event is the same event at every
  scale: the coarse **summary** (`damage::crater_volume` = `E/σ`) and the fine **voxel crater**
  (`matter::impact`) use the same `σ·V` accounting and agree — proven by
  `matter::voxel_crater_matches_the_coarse_damage_summary`. Honest regimes: strength crater, gravity
  (flagged), and **disruption** past a body's binding energy. The Moon impact (~4.5e30 J) is ~36× the
  Moon's binding energy (Moon shatters) but ~2% of Earth's (Earth survives → planet-scale crater); the
  space-band HUD now reports this. The *visual* zoom-in to materialise the crater is designed and
  staged (`docs/19`).
- **Unified deformation & damage — design + first slice** (`docs/18`). One operator for a bullet, a
  pebble in a pond, and a Moon-into-Earth impact: response governed by material data (material
  invariance) at the resolution the observer's frame can perceive (scale/frame invariance). Concrete
  steps: (1) parse material `phase` and fix the liquid fudge — water's `fracture_strength` no longer
  falls back to an unbreakable `1e12` (it was stronger than granite!); a fluid now yields at ~0. (2)
  `MatterSim::impact(site, direction, energy)` — the generalized energy-driven impact: spends the
  impact energy fracturing voxels nearest-first (σ·V each), so bigger energy → bigger crater, stronger
  material → smaller crater, a liquid splashes; a bullet and the Moon are the same call. Tests:
  `materials::a_liquid_yields_where_a_solid_resists`, `matter::impact_is_material_and_scale_invariant`.
- **Orbital-decay control + real collision** in the space band (`docs/17`). `Brake Moon ½×` halves the
  Moon's velocity relative to Earth (a single halving still misses — real orbital mechanics), `Drop
  Moon` cancels it for a radial plunge, `Reset` restores. `orbit::resolve_contact` gives the bodies
  **surface collision** (they stop when their surfaces meet instead of tunnelling through as point
  masses); `orbit::perigee` drives a live closest-approach readout that reddens before a crash. The
  impact's energy is measured and reported (`orbit::inelastic_dissipation` vs `binding_energy`): a
  dropped Moon releases ~4.5e30 J ≈ 36× the Moon's binding energy — the HUD says plainly both bodies
  would be destroyed (actual fragmentation is future, flagged not faked). Variable **time multiplier**
  now exposed in the HUD.
- **Live real-Sun lighting + selectable focus frame** in the space band (`docs/17`). The demo now
  simulates `[Sun, Earth, Moon]` with the Earth on its true heliocentric orbit; the shader lights each
  body from the Sun's *actual position* (per-body, so phases are geometric), and the Sun — far
  off-frame at this zoom — is the light source, not a drawn disk. A focus toggle (`cycle_focus`) makes
  the viewport a physical frame of reference, re-centring on Earth or the Moon.
- **Scene picker** (`web/src/scene-nav.ts`) — a small nav injected on both pages to switch between the
  terrain slice and the space band; the scene list lives in one place.

### Changed
- **Honest space-band appearance** (`docs/17`) — removed the hardcoded ocean-blue/grey body tints
  (fudge) in favour of colour derived from a **real material composition**, aggregated by the new
  `materials::aggregate_albedo` operator (Earth = ocean water + continental rock + polar ice; Moon =
  basalt). The space shader now computes **illumination × reflectance** + Reinhard tone-map, so a
  physically dark body (basalt albedo ~0.05) reads correctly bright under a bright sun, instead of
  being faked bright. Deliberately no atmospheric "blue-marble" blue (unmodelled → not faked).

### Added
- `materials::aggregate_albedo` — the scale-relative summary operator (fraction-weighted mean albedo of
  a composition); the same reduction for any object at any zoom. Tested.
- `orbit::sun_earth_moon_system_is_bound` — a real Sun (proper mass/distance) plus the Earth's
  **appropriate heliocentric velocity**, verifying the Moon stays bound to the Earth while the Earth
  orbits the Sun (3-body, energy-conserving).
- Operating principle / candidate engine name: **"Integrity"** — every rendered value traces to
  something real or is openly flagged as a placeholder (`docs/17`).

### Changed (prior)
- **Unified awake-set dynamics** (`docs/16`) — the probe and the debris are now one system: every
  not-at-rest body feels the same gravity field and resolves contacts against the world *and each
  other*. Debris↔body impulses are momentum-conserving (a thrown clod shoves the probe; the probe
  scatters debris), settling debris never deposits inside a body (piles on it, matter conserved), and
  sleep/wake is structural (a body wakes the instant its support is removed or it's touched). Fixes the
  probe appearing to "rest on nothing" and not truly reacting to debris. New native tests cover
  momentum transfer, no-deposit-inside-body, and wake-on-unsupport.

### Notes
- **Physical-honesty debt flagged:** no atmosphere is modelled, so the per-step `DRAG` in `matter.rs`
  is a numerical stabilizer, not real air drag (documented as debt in `docs/16`).
- **Compute-budget policy** (`docs/16`): favour larger/more massive objects; massive bodies are
  budget-exempt, and debris coarsening will merge into mass-carrying clumps (conserving mass on spawn
  *and* settle) rather than dropping particles — deferred to the `docs/08` clumping work.

### Added
- **Representation invariant** (`docs/15`) — written down as canonical: *a voxel is a sampling cell,
  never a unit of matter.* The cubic grid is a coordinate lattice we sample continuous fields on (like
  pixels), not an ontology of blocks; all physical state lives on matter with continuous coordinates,
  and the grid dissolves into particles the moment physics touches it. Roundness (planets, spheres) is
  emergent from isotropic gravity, exactly as in nature — so building on a cubic lattice is not a
  foundational mistake. Also captures the "feels right in VR" corollary: behaviour is a natural
  property of the world and the object (leave it unsupported, it falls), never per-object fakery.
- **Grid-isotropy regression suite** (`crates/engine/src/isotropy.rs`) enforcing that invariant:
  gravity on a symmetric ball is radial and equal-magnitude in every direction (axes + diagonals), and
  `dig` carves a true Euclidean sphere (right volume, equal reach per axis, no lateral ejection bias).
  Each guard was verified non-vacuous by confirming it goes red under a deliberately anisotropic mutant.

## [0.9.0] — 2026-07-09

**Space band — you can now *watch* the Moon orbit.** The first rung of the scale-relative ladder
(`docs/13`, Step A): a spectator view of the real Earth + Moon, positioned by the validated N-body
physics from `orbit.rs` (v0.8.0). Physics runs in real SI units (f64); metres map to display units
(Earth radius → 1) only for drawing. Separate page, so the terrain slice is untouched.

### Added
- `OrbitDemo` (wasm) + `shaders/space.wgsl` — two lit spheres (ocean-blue Earth, grey Moon) with a
  directional "sun" (so you see phases), driven by `orbit::verlet_step` each frame, time-scaled so a
  full ~27.3-day orbit plays in ~20 s. HUD shows live Earth–Moon separation (hovers near 384,400 km).
- `web/orbit.html` + `web/src/orbit.ts` — camera-only host (drag orbit, pinch/wheel zoom); Vite
  multi-page build now emits both the terrain slice and the space band.
- `docs/13-scale-relative-simulation.md` — the north-star architecture (observer-relative fidelity).
- `docs/14-validation-demonstrations.md` — catalogue mapping each physics test to what it proves and
  how it becomes a visible demonstration for the full build.

### Notes
- The physics is verified natively (`orbit::moon_orbits_earth`); the *visuals* are confirmed on-device
  (headless WebGPU can't render the pipeline here). Next: Step B — refine the planet surface into the
  voxel terrain as you zoom in.

## [0.8.0] — 2026-07-09

**Orbital-mechanics validation (N-body).** The gravity law is now validated against real celestial
motion, not just voxel self-gravity.

### Added
- `orbit.rs` — N-body point-mass gravity with a symplectic **velocity-Verlet** integrator, plus
  energy/angular-momentum helpers. Native test: the **real Earth + Moon** (masses, 384,400 km,
  ~1.022 km/s) produce a **bound orbit** — the Moon completes ≥1 revolution, its distance stays
  within 15% of the real value, and energy + angular momentum are conserved to <1%. "If the Moon
  orbits the planet, the simulator is good" — it does.

### Notes
- Foundation for a future planet-scale demo. The validation itself needs **no rendering** (a pure
  native test), which sidesteps the headless-WebGPU limitation entirely.

## [0.7.2] — 2026-07-09

### Fixed
- **Probe clipped into crater walls — looked duplicated and rested at the wrong height.** The sphere
  only collided with the terrain column directly beneath it, so near a dig it embedded in the wall
  (visible through the thin smoothed surface as a "second ball"). Replaced with proper **sphere-vs-
  voxel collision**: it's pushed out of *any* solid voxel it overlaps (floor, walls, corners), with
  restitution + friction. Solid objects act solid now. Native tests: rests on a voxel floor without
  penetrating; doesn't clip into a wall.

## [0.7.1] — 2026-07-08

**Phase 6 fixes** (from an iPad play-test).

### Fixed
- **Terrain was hollow / open on some sides.** Surface Nets had only one cell of boundary padding, so
  the outer walls sat at the grid edge where closing quads can't form → holes. Padded by two cells;
  new `surface_nets_mesh_is_closed` test verifies the mesh is **watertight** (0 boundary edges).
- **"Eroded cubes" / poor shading.** Feed Surface Nets a **smoothed** (box-blurred) occupancy field so
  the iso-surface rounds properly, and use its own **consistently-outward** normals (a binary field's
  gradient is blocky and my geometry-normal recompute could invert walls).
- **Long-press blast "grew" mounds.** Debris used a center-of-mass gravity approximation that pulls
  off-center matter inward, so it drifted to the middle and piled up. Debris now uses the **full**
  aggregated field (near-straight-down on the slab); the field is coarsened (block 8) to keep the
  per-particle queries cheap.

### Added
- `web/screenshot.mjs` — a headless-Chromium (Playwright) visual-check harness for verifying the
  WebGPU render. Needs GPU render-node access; without it, Chromium falls back to software (SwiftShader),
  which can't run the texture-array pipeline.

## [0.7.0] — 2026-07-08

**Phase 6 — smooth surface meshing.** Terrain and craters render as smooth surfaces instead of
Minecraft-style cubes. The voxel grid stays the physics substrate; only the *visual* changes.

### Added
- `mesher::build_surface_nets` — Surface Nets (via the `fast-surface-nets` crate) over the voxel
  occupancy field, with **smooth normals recomputed from the geometry** (the binary field's own
  gradient is blocky) and oriented outward. Each vertex is tagged with the nearest solid voxel's
  material, so triplanar texturing (Phase 4) and specular shine still apply. Native-tested (valid,
  finite, and genuinely smooth — non-axis-aligned normals).
- The renderer uses it for the initial terrain and every dig re-mesh. The blocky `build` mesher is
  kept as a reference/fallback.

### Notes
- Sim/visual decoupling: physics (mass, gravity, fracture, collapse) is unchanged — the world is
  still "voxels all the way down"; the renderer just presents it smoothly.
- Binary field ⇒ mildly-rounded geometry + smooth shading. Further realism (a smoothed/SDF field for
  rounder geometry, normal maps, finer debris) is future work.

## [0.6.0] — 2026-07-08

**Phase 5 — structural collapse.** Matter that a dig undercuts or isolates no longer floats: anything
not connected to the ground falls. Removes the Phase-3 "floating voxels" limitation.

### Added
- `world.find_unsupported()` — flood-fill from the anchored base (`y = 0`); returns every solid voxel
  not connected to it (6-connectivity). Handles overhangs, undercuts, and blasted-off chunks uniformly.
- `MatterSim::collapse()` — detaches unsupported voxels into falling particles (from rest); one pass
  suffices (the remainder is fully supported). Triggered after every dig.
- Native tests: intact terrain has zero unsupported voxels; an isolated voxel collapses, conserves
  matter, and re-settles into the grid.

### Notes
- Collapse is O(voxels) per edit (a user action, not per-frame). If a collapse would exceed the
  particle budget it caps (a few voxels may remain floating) — noted as a bound, not a silent drop.

## [0.5.0] — 2026-07-08

**Phase 4 — emergent textures.** Completes the vertical-slice roadmap. Materials get a distinct look
generated *from their own physical properties* — no bundled image files, zero licensing exposure.

### Added
- `texture.rs` — procedural texture generator: high-res (512²) RGBA with a full mip chain, synthesized
  from `albedo` + `color_variance` + `metallic` (grain/mottle from tileable multi-octave noise,
  mineral flecks, metal sparkle specks). Seamless (wrapping lattice). Native tests: size + mip chain,
  mean color tracks albedo, materials differ, non-flat variation.
- World shader: **triplanar** sampling of a per-material procedural texture array (no UVs), plus a
  **specular highlight (shine)** driven by per-material `roughness`/`metallic` (metals get a tighter,
  tinted highlight). Material id per vertex; the probe renders as textured iron.
- `materials.rs` loads `roughness`/`metallic`/`color_variance`. HUD adds an **FPS** counter.
- `docs/12` — texture approach + verified CC0 sources (ambientCG/Poly Haven) for optional
  user-supplied real textures via the module system.

### Notes
- Mipmapping is the "client can scale it down" mechanism; `TEX_SIZE` is one constant to raise for
  more detail. The engine bundles **no images** — a material *module* may later drop in a CC0 photo.
- This closes the initial Phase 0–4 vertical slice: layered voxel matter · self-gravity · dig &
  fracture · emergent texture — all from the cited material database.

## [0.4.0] — 2026-07-08

**Phase 3 — dig & material-driven fracture.** Click to dig; matter breaks apart according to each
material's own strength, falls under gravity, and settles back into the world.

### Added
- `matter.rs` — CPU matter solver: spherical dig via voxel raycast; a voxel detaches into a particle
  only if the tool's stress exceeds its material's `fracture_strength` (granite resists a tool that
  shreds soil/grass — no per-material special-casing, just the numbers). Debris falls under the
  Phase-2 field and, on rest, deposits back into the voxel grid (piling; matter-conserving). Native
  tests: soft-vs-hard selectivity, and matter conservation through dig + settle.
- `world.rs` — voxel raycast (Amanatides–Woo DDA) for picking, `set_voxel`, `solid_count`.
- `materials.rs` — loads `fracture_strength` (tensile strength, falling back to cohesion).
- Renderer — instanced debris cubes (`particles.wgsl`), terrain re-mesh on edit; HUD shows debris
  count. Controls: **click** to dig soil/grass, **shift-click** to blast rock.

### Notes
- This is the CPU-tested **foundation** for full continuum MLS-MPM, not the full method yet — it
  delivers dig/fracture/granular behavior emergent from material data. MLS-MPM (deformation gradient +
  constitutive stress, then a WGSL port) is the planned evolution (`docs/06`/`08`).
- Micro-gravity again: ejection is capped below the world's ~7 cm/s escape velocity so debris stays
  bound and re-settles (correct physics, viewed via the time-scale).
- Digging a mid-column hole can leave voxels above "floating" — structural collapse is future work.

## [0.3.0] — 2026-07-08

**Phase 2 — self-gravity & the falling probe.** Density stops being decorative and starts doing
physics: the world's summed voxel mass produces a real Newtonian gravitational field, and a sphere
falls under it (`F = ma`) and rests on the surface.

### Added
- `gravity.rs` — aggregate voxel-mass gravity field (voxels lumped into blocks; direct-sum
  `g(p) = ΣG·mᵢ·(cᵢ−p)/|cᵢ−p|³`, f64 accumulation). Native tests: point-mass `G·M/r²`, far-field,
  mass conservation.
- `body.rs` — rigid sphere integrated with semi-implicit Euler under the field, with ground contact,
  restitution/friction, and a scale-relative rest threshold (works from Earth-g to micro-g). Native
  tests: free-fall kinematics, fall-and-rest.
- Renderer draws the probe (a second mesh with a per-object model matrix); live HUD shows world mass,
  local gravity, probe altitude/speed, rest state, and time-scale. Controls: `Space`/`R` re-drop,
  `[`/`]` time-scale.
- End-to-end native test: the probe falls toward the generated world and rests on it.

### Notes
- Real `G` is used, so the ~96 m test world has asteroid-scale micro-g (~1e-5 m/s²) — correct
  physics. A **time-scale** (default 250×) fast-forwards the sim for viewing; it is time-lapse, not
  amplified gravity.
- The probe is hand-integrated (one body); Rapier is deferred until many bodies / arbitrary contacts
  justify it. The rendered sphere is enlarged for visibility (free-fall is size/mass-independent).

## [0.2.0] — 2026-07-08

**Phase 1 — layered voxel world.** The cited material data becomes a rendered, orbitable world.

### Added
- `data/materials.json` — 19 cited materials (density, moduli, strengths, hardness, albedo, …) as
  the physical single source of truth (`docs/04`).
- Engine sim modules (natively unit-tested): `materials` (loads the database), `world` (chunked
  voxel store + layered rock/dirt/grass generator with a value-noise heightfield, using real
  densities), `mesher` (face-culling mesh, per-material albedo colors).
- Real 3D renderer: depth buffer, perspective orbit camera, directional + hemispheric lighting;
  `Engine.set_orbit(yaw, pitch, zoom)`. Host adds drag-to-orbit / scroll-to-zoom.
- `cargo test` suite (material load, layer ordering, mesh validity) — TDD is canonical; wgpu/wasm
  code is gated to the wasm target so the sim logic tests natively.
- Design docs `05`–`10`: Postgres→JSON data pipeline, material modules, taxonomy/finishes/object
  composition, adaptive clumping/LOD, agentic authoring + interaction, and robustness principles.
- CI: fmt + clippy + native tests + wasm build on every push.

### Notes
- Face-culling (blocky) mesher for now; smooth surface-nets meshing is a planned upgrade.
- Density is stored per material but not yet physically active — it drives self-gravity in Phase 2.

## [0.1.0] — 2026-07-08

First milestone: **Phase 0 — scaffold & first pixel.** The full Rust → WASM → `wgpu` → canvas
pipeline is live, driven by a thin Vite/TypeScript host.

### Added
- Cargo workspace with the `engine` crate (`cdylib` + `rlib`) compiled to WASM via `wasm-pack`.
- `Engine` WASM API: `Engine.create(canvas)`, `render()`, `resize(w, h)` — a `wgpu` WebGPU
  device that clears the canvas with a pulsing color each frame.
- Vite + TypeScript host (`web/`) that loads the WASM, sizes the canvas, and pumps
  `requestAnimationFrame`, with a graceful "WebGPU unavailable" message.
- Project meta: MIT license, `README`, `CONTRIBUTING`, `JOURNAL`, this changelog, and two
  research reports under `docs/` surveying prior art and reusable OSS building blocks.

### Notes
- Pinned to `wgpu` 24.0.5. WebGPU-only backend to keep the WASM small.
- **Public API is unstable** while we're pre-1.0 (see versioning policy).

[Unreleased]: https://example.invalid/compare/v0.7.1...HEAD
[0.7.1]: https://example.invalid/releases/tag/v0.7.1
[0.7.0]: https://example.invalid/releases/tag/v0.7.0
[0.6.0]: https://example.invalid/releases/tag/v0.6.0
[0.5.0]: https://example.invalid/releases/tag/v0.5.0
[0.4.0]: https://example.invalid/releases/tag/v0.4.0
[0.3.0]: https://example.invalid/releases/tag/v0.3.0
[0.2.0]: https://example.invalid/releases/tag/v0.2.0
[0.1.0]: https://example.invalid/releases/tag/v0.1.0
