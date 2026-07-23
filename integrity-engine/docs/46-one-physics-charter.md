# docs/46 — One physics: the charter, and the conformance ledger

> **The promise.** REAL physics — one law, at every scale, in every scene. A world is a world is a world.
> This is the product, not an aesthetic preference about code structure. An engine that answers the same
> physical question two different ways in two different scenes has broken its promise, however good each
> answer looks on its own.
>
> **The span that promise commits to, stated as the acceptance test:** simulate a **star's photosphere
> and generate a solar flare**, or **a raindrop on a flower petal**, with the *same engine* — without
> reinventing or re-coding anything. It should fall out of the scale a scene is viewed at and the action
> occurring in it. **And the player must be able to SEE both.** That is ~15 orders of magnitude with no
> scene-specific branch on either the physics side or the render side.
>
> Judge every change against it: **if it would need re-coding to work at another scale, it is the wrong
> change** — however good the result looks in the scene it was written for. A new per-scene code path is
> a failure even when it ships something beautiful.

The mechanism for this is `docs/13-scale-relative-simulation.md` (*"both simulation and rendering cost
should scale with what is observable from the current viewpoint … detail emerges continuously: star
field → planet disk → landscape → terrain → the rock → its grains"*), with `docs/44` (resolution
EXTENT — how much to resolve) and `docs/47` §1 (resolution GRANULARITY — how fine). This charter states
the promise; those state how it is met, and the split is deliberate — restating their content here would
itself be two answers to one question. **Neither axis is built yet**, and that is the single largest gap
between this promise and the code: docs/44 self-audits as unimplemented (`MATERIALIZE_CAP = 14.0` still
clips the derived footprint at `lib.rs:878`), and granularity is design-only — the GPU `Particle` struct
has no size field and `part_half` is a per-dispatch uniform, so two grain scales cannot coexist in one
scene. A tyre contact patch needs centimetre grains in the same world a meteor uses metre grains; until
that lands, the span above is a promise the code cannot yet keep.

This doc exists so the ledger below is **read, not rediscovered**. Every session so far has re-derived
some part of it from scratch. `docs/32 §4` maps the forks structurally; this doc states the *rule* that
makes a fork acceptable or not, and keeps the running list of open violations with their evidence.

---

## 1. The rule: specialization vs violation

Not every difference is a breach. The criterion is sharp:

- **Legitimate specialization** — the *physics itself* differs, so the numerical treatment differs.
  Stiff granular contacts need an implicit/semi-implicit integrator to stay stable; self-gravitating SPH
  needs a symplectic leapfrog to conserve energy over orbits (docs/38 §"what unified GPU path does NOT
  mean"). Forcing one integrator on both is unstable or ruinously slow. Boundary conditions likewise:
  the terrain band models Earth as a rigid heightfield because you are standing on a 96 m patch, not
  simulating a planet's interior.
- **DECLARED (a resolution IOU)** — the resolved physics is real and correct but **unaffordable at the
  timescale humans and models need in order to play with it and learn from it**. So the outcome is
  computed from the material's own data and rendered, instead of being allowed to emerge. This is
  legitimate, and it is where most of the interesting engineering will live for years.
- **Violation (a fudge)** — the *same physical question* gets two different answers. Two implementations
  of one law that can drift. A law implemented completely in one place and partially in another. A
  quantity conserved in one scene and not in the other. Or a number chosen because the result looked right.

### Declared is not fudged — the distinction that keeps it honest

A drive-shaft *should* shear when torque exceeds what its material can carry, and in a perfect world it
would do so because the bonds actually failed. That is far too expensive today. So we compute whether it
shears — real torque against the material's real shear strength — and render that outcome.

**That is not a second answer to the same question.** It is the same physics evaluated analytically
because the resolved evaluation sits below the resolution we can afford. The test that separates it from
a fudge:

> **Can you name the resolved computation this replaces, and would the declared answer converge to it as
> resolution rises?**

If yes, it is a declared model carrying an IOU. If you cannot name it, it is a fudge wearing a physics
coat.

The codebase already does this well, and those are the pattern to copy: `plough_loft` declares an
excavation shock finer than a grain as a **conserved momentum transfer**, says so outright, and notes
that co-motion is the physical maximum so there is no coefficient to tune; `Furrow::ejection` marks its
velocity SCALE honest and its distribution SHAPE an explicit resolution IOU *"to be DELETED once particle
count is high enough for the flow to emerge on its own."*

Compare the fudges: `MAX_EJECT = 0.045`, `steep_drop = 3` (retired 2026-07-19, docs/45), a hardcoded
damping ζ. None names a resolved computation; none would converge to anything.

### The horizon — write IOUs so a descendant can delete them

This layer is a **compute-era limitation, not a design preference**. The purpose is to render real
physics on the timescale and compute humans and LLM models need to enjoy and learn from it. If compute
becomes cheap enough — quantum or otherwise — descendants of this engine should be able to delete the
calculate-and-render step and simply let the drive-shaft shear.

That only works if IOUs are written *for deletion*: each declared model must state the resolved
computation it stands in for, so retiring it is a substitution rather than an archaeology project.
**A declared model with no stated resolved counterpart is a fudge, because nobody can ever retire it.**

The test any change must pass: **does this reduce the number of places where one physical question has
two answers?** A declared model does not add an answer — it defers one, in writing. If a change adds a
genuine second answer, it needs a physics reason, written down, or it is debt.

## 2. What is already unified (do not fork these)

From docs/32 §4, and it is a real achievement worth protecting:

`granular::Contact` + `contact_accel` · the SPH kernel `sph_w`/`sph_dw` (one kernel for `AirField` AND
aggregate vapor) · `Furrow` + `ExcavSurface` + `ejection` + `ejecta_energy_scale` (ONE excavation
primitive for the terrain meteor and the giant impact) · `plough_loft` · `deposit_resting_grain` (one
de-resolution primitive, CPU and GPU) · `Body` as the universal particle · `Material`/`LayeredBody` as
the universal matter description · `zeta_for_restitution` (bond damping and contact damping from one
derivation).

## 3. The conformance ledger — open violations

Each entry: what has two answers, the evidence, and where it is tracked. Structural forks 1–4 are
docs/32 §4's; the rest were found since and are recorded here so they are not rediscovered.

The **consumers** column (added 2026-07-22) lists the scenes/modules that actually INSTANTIATE the
physics in question, verified by grep on that date, never assumed. Where it reads **none**, that is
the point: a verified law with zero consumers stays OPEN no matter how green its tests are, because
its tests pass whether or not anything calls it (see §7).

| # | one question, two answers | evidence | tracked | consumers |
|---|---|---|---|---|
| 1 | **Two container universes** — `Aggregate` (`Vec<Body>`, f64) vs voxel `World` + GPU f32 | docs/32 §4.1 | docs/33 | `Aggregate`: `OrbitDemo`'s moon-drop debris (`impact::build_impact_debris_scaled`, `lib.rs:2665`). Voxel `World` + `MatterSim`: `simulation::Simulation` (the Ground scene, `bin/run-definition`). The GPU f32 granular container `gpu_particles::GpuParticles`: `GpuProbe` only, a compute diagnostic with no canvas |
| 2 | **Four integrators over one law** | docs/32 §4.2 | docs/38 (partly legitimate — see §1) | GPU KDK (`gpu_sph::encode_kdk`): `OrbitDemo` birth (`lib.rs:2435`). CPU KDK block (`Aggregate::step_block`): `OrbitDemo` moon-drop debris (`lib.rs:2759`). CPU Euler settle (`MatterSim::step`): `Simulation` (Ground). GPU trapezoidal-implicit (`particle_step.wgsl`): `GpuProbe` only. `AirField` SPH relaxation: **none**. CPU KDK + adaptive dt (`hydrostatic::step`/`relax_step`): **none** (production builds `HydroBody` seeds and relaxes on the GPU via `cs_relax`) |
| 3 | **Rigid-boundary fork** — in an impact Earth is simultaneously materialized grains AND a rigid boundary | docs/32 §4.3 | docs/33 | `OrbitDemo`, both arms in one scene: the moon-drop path sets `boundary` + `boundary_hole` + `gravity_source` (`impact.rs:561-577`, adjusted live at `lib.rs:2727`, `lib.rs:2830`) while the same planet's debris are particles; the SPH birth path has Earth as particles with no rigid boundary |
| 4 | **Two rigid-body reps** — `body::Sphere` vs the cohesive-`Aggregate` probe | docs/32 §4.4 | docs/38 4c′ | **none**: `body::Sphere` is built only in tests and `Simulation::step` passes an empty body slice (`simulation.rs:164`); the cohesive-`Aggregate` probe lost its scene with the terrain deletion (docs/50), leaving `PROBE_LATTICE`/`PROBE_STIFFNESS_CAP` (`lib.rs:291`) consumerless |
| 5 | ~~**Slope stability is half a law.**~~ **CLOSED 2026-07-19.** Terrain and grains now read the same `friction_coefficient` through one law, `granular::face_stable`; `steep_drop` is retired | was: `granular.rs:73` vs `matter.rs:538` (`h_crit = c/ρg` alone), non-convergent at 106→622 grains/pass. Now: fixpoint on the second pass, settled slope asserted against the DB μ, pristine terrain a no-op (470→0 grains) | **docs/45 §7** | closed |
| 6 | **The de-resolution ladder stops one rung short.** grain→voxel works; voxel→field is built but never triggered. **NARROWED 2026-07-19:** the mechanism is now SAFE — one authoritative `World::ground_top_voxel` answers "where is the ground" for voxels and field alike, and the GPU heightfield, the CPU bilinear surface and the rendered cap all read it. What remains is the TRIGGER, plus `patch_resolved` being one bool for the whole 96 m patch while demotion is per-column | was: three different answers to one question — GPU heightfield read raw voxels (demoted column ⇒ grains fall through), the cap read raw `terrain_height` (demoted crater renders as untouched ground), probe read `bulk_height`. Measured: 98% of grains return (3,605 → 78) | docs/47 §5, docs/39 item #4 | **none** for the open half: `World::demote_column_to_field`/`demote_patch_to_field` have zero production callers (the only call sites are `mesher.rs` tests, e.g. `mesher.rs:1003`); nothing in `Simulation` or the Ground scene ever triggers voxel→field |
| 7 | **Promotion is gated visually in one doc, physically in another.** docs/30: the trigger must be "a physical error bound … never a visual one". docs/39: gate on "camera-visible ∧ interacting" | the two docs, directly | **docs/44** | the gate that actually runs is docs/39's visual one: `Simulation::step` materialises on `(c - camera).length() < view_r` (`simulation.rs:156`) feeding `ResolutionField::update`, consumed by the Ground scene and `bin/run-definition`; docs/30's physical error bound has no consumer |
| 8 | **The honest footprint is computed, then discarded.** `crater_radius` from `V = E/σ` is derived, then clipped by `MATERIALIZE_CAP = 14.0`, and `resolve_patch` resolves the whole 96 m patch regardless | `lib.rs:858`, `lib.rs:992` (self-flagged) | **docs/44** | the evidence sites are gone: `MATERIALIZE_CAP` and `resolve_patch` were deleted with the terrain scene (docs/50), zero grep hits today. The question stands: `damage::crater_radius` is consumed by `interaction.rs:102` swept-collision effects (`OrbitDemo`), and the ground path clips its materialised crater at `MatterSim::impact`'s flagged `MAX_R = 24` LOD guard (`matter.rs:190`), consumed by `Simulation` (Ground) |
| 9 | **Matter leaks at the seam — and it is a DOMAIN property, now measurable per definition (docs/54).** Grains that leave the patch are culled by `matter::step`; the loss is invisible from a particle count, since de-resolution looks identical. `run-definition` reports created/returned/in-flight/lost: a 96 m patch with gentle events loses **0.0%** (260 created, 260 returned), while a 48 m patch with an energetic impact loses **28.8%** (6,328 created, 1,822 lost). The earlier ~2% figure was the big patch only. Sizing the domain to the event is the open question | measured 2026-07-21 from `definitions/ejecta-ground.json` and `definitions/small-island.json` | **docs/54** — was: ~2% of debris never returns to the field (deposition refused inside a dynamic body; the water branch is a self-flagged static-sea placeholder) | measured: 78 of 3,605 grains stranded per event, monotonic | this doc | `simulation::Simulation` (the Ground scene; `bin/run-definition` is the consumer that measures the loss) |
| 10 | **Vehicle/probe never contacts debris.** The probe is a CPU `Aggregate`; grains live in a GPU buffer. Coupling is a bounding-sphere exclusion, not contact | `lib.rs` settle path; `matter::couple_body` exists but is called only from tests | docs/38 4c′ | **none**: `matter::couple_body`'s only callers are its own tests (`matter.rs:2210`), and `Simulation::step` passes `&[]` for bodies (`simulation.rs:164`) |
| 11 | **An asteroid-era constant still runs in an Earth-g scene.** `MAX_EJECT = 0.045` m/s, capped "below the world's ~7 cm/s escape velocity" — a 0.1 mm ballistic hop at 9.81 m/s² | `matter.rs:41` | this doc | **none**: `MAX_EJECT` (now `matter.rs:63`) lives in `MatterSim::dig`, which no scene calls; its only callers are the `isotropy.rs` regression suite. The constant is unconsumed AND still wrong, in that order |
| 12 | **The render asserts a physical state the simulation does not have.** Every Earth scene draws an honest Rayleigh sky over a **vacuum**: `atmosphere::AirField` — pressure-layered, with verified hydrostatic balance, momentum-conserving drag and hypersonic entry heating — is instantiated in **no scene** | `grep AirField crates/engine/src/lib.rs` → nothing; 11 atmosphere tests pass regardless | **docs/48** | **none**: `AirField::new` appears only in `atmosphere.rs`'s own tests; every reference outside that file is a doc comment |

| 13 | **Incandescence has two curves.** "What colour does matter at temperature T glow?" is answered by `emission::incandescence` (docs/20, natively tested, returns premultiplied `[r,g,b]`) AND by a second copy inside `mod app` for the space band (returns `[r,g,b,intensity]`). They agree only on the 800 K glow threshold | both read directly: `emission.rs:13` ramps intensity `(T−800)/2200` capped at 4 with blue from 2600 K; the space-band copy ramps `x=(T−800)/2400` saturating at 3200 K with blue from `x>0.55`. At 2000 K one gives `[0.545, 0.297, 0]`, the other `[1.0, 0.5, 0.0]×0.6` | this doc — found 2026-07-20 during the docs/33 render-scaffolding lift; NOT unified in that PR because collapsing them changes what the space band looks like, which needs its own rig verification | `emission::incandescence`: the Ground scene only (`ground_scene.rs:764`, `:782`). The `mod app` copy (`lib.rs:3836`): `OrbitDemo` only (`lib.rs:3246`, `:3323`, `:3421`). `Terra` consumes neither |

| 14 | **Scene KINDS are code; scene INSTANCES are already data — except one.** Robin's requirement: a scene should be object/assembly definitions, coordinates and materials and "should NOT require special mods of the engine itself". **NARROWED 2026-07-21 by measurement, after the row was first written too broadly.** `orbit.html` and `twomoons.html` are the SAME script and the SAME `OrbitDemo` differing only by `data-world=…/world.json`; `terra.html` likewise. Those are data. What remains code: (a) the scene KIND — adding a genuinely new kind means a new `#[wasm_bindgen]` struct with its own pipelines and render loop, and deleting terrain cost **1,516 lines of `lib.rs`** plus a public-API symbol and a build entry; (b) **`birth.html` alone still has NO world file** — `data-scene="birth"` selects a hardcoded path whose initial conditions (body radii, materials, proto-Earth spin, impact geometry 1.15·v_esc / b≈R_e / d₀=1.6·contact) are Rust constants | pages measured directly; birth's ICs at `gpu_sph::build_impact_bodies` + `assemble_from_relaxed` | **docs/51** — (b) first, since it is the last scene on a code path | the scene kinds: `OrbitDemo` and `Terra` (`lib.rs`) plus `Ground` (`ground_scene.rs`), each a `#[wasm_bindgen]` struct. The birth ICs: `OrbitDemo` via `gpu_sph::build_impact_bodies_from`/`assemble_from_relaxed_with`; note `web/birth.html:77` now carries `data-world="/worlds/birth/world.json"` and the file exists, so (b) is narrower than first written |

| 15 | ~~**Deleting terrain orphaned three verified systems.**~~ **CLOSED 2026-07-21 (docs/53)** — `crate::simulation::Simulation` builds and steps them from a `"ground"` world DEFINITION (production code: 8 `MatterSim` refs, 4 `ResolutionField`, 1 `world::generate`), so the consumer is a file and no scene's deletion can orphan them again. Verified end to end from `definitions/ejecta-ground.json`: an effect propagates analytically off-camera, materialises 257 grains on entering view, and every one de-resolves back to the world (644,190 → 644,450 voxels, **+260** = 257 grains + 3 impact particles — matter conserved). Was: It was the ONLY production consumer of `matter::MatterSim` (the shared matter path), `resolution::ResolutionField` (docs/49 camera-driven resolution, wired the day before), and the voxel `world::World`+`mesher`; the granular GPU pipeline is now reachable only from `GpuProbe`, a compute-only diagnostic with no canvas | measured 2026-07-21 after the deletion: **0** references to `MatterSim`/`ResolutionField` anywhere in `lib.rs`; all 6 `world::generate` calls are inside `#[cfg(test)]`. Every test still passes, which is why it is easy to miss | **docs/51** — a requirement on the NEXT scene: re-consume them or delete them. This is docs/48's wiring pattern at its sharpest: physics wired into one place, and that place deleted | closed |

| 16 | **Scenes build their own worlds instead of using ONE Earth.** Robin: *"Terra should occur naturally from definitions of material, biomes, etc… a fully materialized object, reusable between scenes. Then this scene would simply be using that planet/solar system."* Instead every scene constructs its own: the ground scene calls `world::generate_from` for a private 96 m voxel cube and references `planet::earth()` only for `g = GM/R²` and air pressure | measured 2026-07-21: the ground patch is 96 m across = **0.00024% of Earth's circumference** — walk ~48 m and you reach the edge of the world; the planet supplies numbers, not a place. Detail is fine because the patch is 1 m voxels, NOT because the camera is close (which is what `ResolutionController::camera_grain_radius` exists to do) | **docs/23** (the north star: one Earth, the Moon hits it, zoom to the ball) + **docs/13** (scale-relative) + **docs/43** (worlds as data) — the principle was ALREADY recorded; this row is only the measured violation. Composition, not construction: `LayeredBody`, Terra's rasters, `ResolutionController::camera_grain_radius`, `hydrostatic`/`eos` and the GPU granular container all exist and have never been composed into one Earth every scene shares | **none** shares one Earth; each scene constructs its own: `Simulation` builds a private patch via `world::generate_from` (`simulation.rs:80`) with `planet::earth()` supplying only g and air pressure (`ground_scene.rs:597`, `:622`); `Terra` and `OrbitDemo` build their worlds and bodies from their own world files |

**A distinct violation class (item 12).** Rows 1–11 are *one question, two answers*. Row 12 is different:
the optics are honest and the world beneath them is empty. It inverts *physics drives the render* — not
by faking the picture, but by leaving the picture's subject unbuilt. It is invisible to every existing
check, because the atmosphere tests pass, the sky tests pass, and nothing asks whether anything
**instantiates** what they describe.

### The wiring pattern — look for this before building anything

Three independent cases now share one shape: **the law is built and proven, then wired into one place or
none.**

| verified physics | wired into |
|---|---|
| docs/39 JIT particalization (conserving to <1e-12) | planetary scale only; terrain **never** |
| `granular::terrain_contact_resolve` (energy-monotone, hardware-verified) | GPU grains only; bodies **never**, until PR #15 |
| `atmosphere::AirField` (hydrostatic, drag, entry heating) | **nothing** |

The instinct on finding a gap has been "we need to build X". The evidence says the likelier truth is "X
exists, verified, and nothing calls it." **Grep for the primitive before writing one** — the
second-cheapest move after reading the docs.

Items 5, 6, 9 and 11 are the ones a "same in every scene" reading makes urgent: they are places where the
*same matter* behaves differently depending on which scene or which rung of the resolution ladder it is
on.

## 4. The scale test

A world is a world is a world. So for any primitive, ask: **does it run at both ends?**

The de-resolution cycle is the worked example. docs/39 proves
`field → particalize → simulate → quiesce → bake_back → field` at planetary scale, conserving mass,
momentum and COM to **<1e-12**. It is listed there as item #4 that terrain is "the separate low-energy
instance of the same primitive, after planetary scale." Measurement (§3 item 6) confirms terrain has only
the first half. The primitive is proven; it has simply never been instantiated at the second scale.

That is the shape of most remaining work: **not new physics — the same law, run at the other end.**

## 5. What this forbids

- Tuning one side to agree with a known-broken other side. docs/45 records the live case: terrain repose
  implemented correctly would disagree with grain repose, because the grain side under-predicts for its
  own flagged reason (spherical grains roll). Making them agree by adjusting the correct half would look
  consistent and be wrong.
- Constants that read as physics. `steep_drop = 3` (a 72° slope — RETIRED, docs/45), `MATERIALIZE_CAP = 14.0`,
  `MAX_EJECT = 0.045`, a hardcoded damping ζ. Each was defensible when written and each silently became a
  lie when the world around it changed. A number is either derived from a material datum / conservation
  law, or explicitly flagged as a budget with an IOU (docs/24).
- A second implementation of a shared law "just for this scene". If the scene genuinely needs different
  physics, that is a docs/NN, not a copy.

## 6. How to use this doc

Before adding physics: check §2 for an existing primitive to extend. Before accepting a fork: apply §1.
After finding a new violation: **add a row to §3 with evidence**, so the next session inherits it instead
of rediscovering it.

## 7. The merge rule: wired, or IOU'd. Built and verified is not done

The ledger's consumers column exists because the repo's most corrosive pattern (rows 12 and 15, the
wiring table in §3) is invisible to every existing check: a law's tests pass whether or not anything
instantiates it. So the bar a physics change must clear to merge is stated here, once:

> **New physics lands either wired into at least one scene, or carrying a flagged IOU that names the
> milestone that wires it.** "Built and verified" does not count as done. It counts as inventory.

Corollaries:

- A ledger row whose consumers cell reads **none** stays OPEN no matter how green its tests are.
  Closing it means a production call site, not a passing suite.
- Consumer claims are grep results, never intentions. "Will be wired by X" without a flagged IOU
  naming X is the same claim as "none", written optimistically.
- Deleting a law's only consumer reopens its row (row 15 is the worked example: the wiring pattern at
  its sharpest, physics wired into one place and that place deleted).

---

**Related:** CLAUDE.md (the charter in brief) · docs/23, docs/24, docs/28 (the one-law charter docs) ·
docs/30 (temporal deltas; the physical-bound rule) · docs/32 §4 (the structural fork map) · docs/33
(realignment) · docs/38 (composite contact law; legitimate integrator specialization) · docs/39 (the JIT
particalization primitive, proven at planetary scale) · docs/44 (resolution by necessity) · docs/45
(terrain slope stability).
