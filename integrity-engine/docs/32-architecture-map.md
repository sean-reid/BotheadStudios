# Architecture map — for future Claude sessions (written 2026-07-17, refreshed 2026-07-19)

A durable orientation to the Integrity engine, written so a Claude session starts *oriented*
instead of rediscovering machinery. Read this before exploring. All refs are `file.rs:line` — click them.

> **Read [`docs/46-one-physics-charter.md`](46-one-physics-charter.md) too.** This doc maps the forks
> *structurally*; docs/46 states the **rule** that makes a fork legitimate (specialization / declared with
> an IOU / fudge) and carries the **conformance ledger** of open violations. §4 below and docs/46 §3 are
> two views of one thing — when they disagree, docs/46 is newer and wins.
>
> **On staleness.** This map went two days without an edit while docs/34–48 and ~20 commits landed, and
> drifted badly enough to assert a module was absent that had in fact been built and verified (§5). Line
> anchors rot fastest — treat them as a starting point, and follow docs/46's rule: **grep for the primitive
> before writing one.** If you find an anchor wrong, fix it here in the same commit.

**One-crate mental model.** Everything performance-critical is ONE Rust crate (`crates/engine`) compiled to
WASM via `wasm-pack`, sharing a single `wgpu` device with the renderer (zero-copy). `web/` is a thin
TypeScript+Vite host. Public demo: **integrity.bothead.net** (see docs/29). The engine's charter is
*everything is matter; one contact law + one gravity law govern it at every scale* (docs/23/24/28) — a tire,
a meteor, and Theia are meant to be the same physics at different scale/energy/material.

**The single most important architectural fact:** the physics *laws* are already largely unified and
scale-invariant; the *solvers and containers* are forked. See §4. Do not add a new per-scene particle path —
extend the shared one.

---

## 1. The THREE scene structs (the top of the call tree)

Three independent `wasm-bindgen` structs in `mod app` (`lib.rs:69`), each its own scene family, wgpu device,
and render loop:

- **`Engine`** (`lib.rs:244`, impl `:319`) — the **terrain band** (`terrain.html`, host `web/src/main.ts`).
  A 96 m Earth surface patch. Debris runs on the **GPU compute** shader (`particle_step.wgsl`). Probe = a
  CPU cohesive `Aggregate`.
- **`OrbitDemo`** (`lib.rs:2730`, impl `:2879`) — the **space band** (`orbit.html`, `birth.html`,
  `twomoons.html`, host `web/src/orbit.ts`). N-body Sun/Earth/Moon(s); a giant impact shatters a body into a
  debris cloud. **This one runs GPU compute now** — it owns a `gpu_sph::GpuSph` (`:2828`) driving
  `shaders/sph_step.wgsl`. (This doc previously said "No GPU compute — all particle physics is CPU"; that
  was true when written and is not true now.)
- **`Terra`** (`lib.rs:5140`, impl `:5178`) — the **worlds-as-data planet scene** (docs/43). Cube-sphere
  globe + fine ground cap under the camera + continuous fly camera (orbit ⇄ ground) + baked equirectangular
  rasters. Backed by its own submodule `crates/engine/src/terra/` (5 files, 910 lines).

The fork between the particle paths (GPU terrain grains vs the space band's aggregate/SPH) is the central
thing a refactor toward "one particle module" must dissolve.

---

## 2. Physics core (the celestial / particle modules)

- **orbit.rs** (661) — N-body point-mass mechanics. `Body{pos,vel,mass}` (`:20`, the UNIVERSAL particle,
  f64, reused everywhere), `G` (`:18`), `accelerations` (O(N²) 1/r², `:28`), `verlet_step` (KDK symplectic,
  `:45`), `perigee` (`:83`), `resolve_contact` (inelastic COM merge, `:106`), `swept_first_contact` (CCD,
  `:141`), `contact_velocity` (dt-independent true impact velocity from vis-viva + angular-momentum, `:173`),
  `binding_energy` (`:203`). IOU: `resolve_contact` drops dissipation as heat (flagged placeholder).
- **aggregate.rs** (1788) — **THE particle-cloud simulator.** `Aggregate` (`:41`): `particles:Vec<Body>`,
  per-particle `temps`/`mat_ids`, `softening`, cohesive `bonds`, self-gravity (`gravity_source` extended
  1/r²+Gauss interior, `gravity_bodies` Sun+planets, `self_gravity` via Barnes–Hut), contact
  (`contact`/`per_grain_contact`/`contact_ref_mass`), the rigid **`boundary`** penalty sphere +
  `boundary_hole` crater + `boundary_force/torque_sum` (Newton-3rd-law reaction → planet spin), vapor/SPH
  (`vapor`, `contact_gas`, `boil_k`, `vapor_rs/h/latent_k/rho`), `source` provenance. Force kernel
  `accelerations()`/`accelerations_masked()` (`:498/:507`): uniform g → extended source (Gauss interior) →
  Barnes–Hut self-gravity (θ=0.5) → gravity_bodies → NeighborGrid contact (phase-dispatched) → boundary →
  bonds → SPH vapor `P=ρ·R_s·(T−L_v/c)`. `step()` (KDK+fracture+thermo, `:918`), `apply_thermo` (PdV cool,
  Stefan–Boltzmann, phase flip, dissipation heat, `:937`), block timestep `particle_timesteps`/`step_block`
  (`:774/:804`), `deposit_impact` (`:424`), `drain_settled` (demote settled mass+L back to planet, `:882`).
- **granular.rs** (753) — **the contact force law of record** (soft-sphere DEM). `Contact`
  (radius/stiffness/normal_damp/friction/tangent_damp/cohesion/coh_range/shock, `:63`),
  `contact_from_material` (stiffness=E·r/m, damping from restitution, cohesion σ·A/m, `:38`), `Contact::mix`
  (per-pair cross-material, idempotent, `:153`), `contact_accel` (the pair force, `:172`),
  `contact_dissipation` (exact heat partner, `:229`), `plough_loft` (momentum-conserving downrange
  excavation drag, `:110`), `terrain_contact_resolve` (NON-injecting heightfield constraint — the
  settling-storm fix, `:299`). The Rust `contact_accel` is hand-mirrored in `particle_step.wgsl:132`.
- **bhtree.rs** (250) — Barnes–Hut octree, O(N log N) softened self-gravity (docs/30). `build`/
  `accelerations`/`accelerations_active` (block-timestep subset), θ=0.5, `BRUTE_BELOW=1024`. RMS<1% vs
  brute force (verified).
- **neighbors.rs** (146) — `NeighborGrid` spatial hash, O(N) EXACT short-range pairs (`build`/
  `for_each_pair`, `BRUTE_BELOW=512`). The exactness is the invariant the perf work rests on.
- **impact.rs** (2024) — mutual-impact builder (the giant-impact physics-of-record). `ExcavSurface`
  (Curved|Flat — the ONE surface for planet AND terrain patch, `:41`), `Furrow` (downrange excavation
  ellipsoid + declared H-H ejection, `:117`; `ejection` `:222`), `ejecta_energy_scale` (EXACT KE cap,
  `:284`), `furrow_target_grains` (fills furrow with ρ·V layered grains, `:321`),
  `build_impact_debris_scaled` (materializes impactor + cap, applies `earth_omega` spin + `plough_loft`,
  wires the full `Aggregate`, `:421`). **Most-flagged module:** `Furrow::ejection` is an explicit
  RESOLUTION IOU (sub-grain shock declared from cited scaling, to be DELETED as N rises); the isotopic-crisis
  test (`:732`) documents that spin alone can't Earth-enrich the disk under a rigid boundary (docs/31).
- **planet.rs** (482) — `LayeredBody`: a planet DECLARED as concentric real-material layers;
  gravity/pressure/phase COMPUTED. `layer_at`/`temperature_at`/`enclosed_mass`/`gravity_at` (Gauss),
  **`pressure_at`** (analytic hydrostatic dP/dr=−ρg, midpoint-integrated inward — the VALIDATION TARGET for a
  particle planet, `:130`), `phase_at` (Simon–Glatzel melt + Clausius–Clapeyron boil). Profiles `earth()`/
  `moon()`/`sun()`/`theia()`. IOU: layer densities are declared PREM constants (compression is data, not
  computed — see the EOS gap §5).
- **materials.rs** (338) — `Material` (density, fracture_strength, youngs_modulus, friction, restitution,
  cohesion, optical, `thermal:Option<Thermal>`) loaded from embedded cited `data/materials.json`. `Thermal`
  with `melt_point_at`/`boil_point_at`. IOU: friction/restitution/albedo are flagged constitutive
  placeholders; liquid `fracture_strength` forced to 0 (removed the "fluid stronger than granite" fudge).
- **damage.rs** (193) — cross-scale classification: `classify(energy_density,m)` → Intact/Fractured/Melted/
  Vaporized by ρ(cΔT+L) thresholds; `crater_volume=E/σ`; `ground_effect` (disruption at binding energy).
- **emission.rs** (57) — `incandescence(temp_k)` black-body-ish glow (hot matter emits, isn't lit).
- **tides.rs** (407) — spin as bookkept angular momentum; secular tidal migration (validated vs 3.8 cm/yr
  lunar recession), Radau–Darwin flattening, J₂, `Moonlet` geologic-time mergers (Hill/Gladman).
- **gravity.rs** (181) — voxel-world self-gravity (`MassField`, one-level Barnes–Hut over block³ mass
  points, f32) — the terrain-band analogue of bhtree; a DISTINCT particle system from `Aggregate`.
- **isotropy.rs** (168) — test-only grid-isotropy guardrail (docs/15): gravity + dig stay direction-
  independent (lattice is a sampling grid, not matter).

### 2b. The docs/33 realignment stack (built after this map was first written — omitted from it entirely)

These four are the physics core of the realignment. A session reading only §2 above would conclude they do
not exist and rebuild them; they are real, tested, and partly wired.

- **eos.rs** (373, 7 tests) — **the Tillotson condensed-matter EOS.** See §5 item 6 for the anchors and the
  wiring status. This is the module §5 used to call "CONFIRMED ABSENT".
- **hydrostatic.rs** (1357, 10 tests — 9 `#[ignore]`d as measurement runs) — self-gravitating
  condensed-matter body in hydrostatic equilibrium (docs/33 stage 2): gravity + Tillotson pressure +
  Monaghan artificial viscosity, KDK leapfrog. `HydroBody` (`:42`), `new_sphere` (`:66`),
  `new_differentiated` (`:97`, iron core + basalt mantle), `new_lod` (`:146`), `compute_density` (`:191`),
  `accelerations` (`:210`), `relax_step` (`:235`), `forces_and_dudt` (`:250`), `step` (`:295`, KDK),
  `courant_dt` (`:316`), `rms_radius` (`:344`). **Used only by `gpu_sph.rs`** (CPU-side build + relax before
  GPU upload) and the native tools; not referenced from `lib.rs` except through `gpu_sph`.
- **gpu_sph.rs** (710, **0 tests** — verified out-of-process by `tools/sph-verify`) — the in-browser GPU SPH
  stepper driving `shaders/sph_step.wgsl`; the most lib.rs-wired of the four. `GpuSph` (`:492`): `new`
  (`:520`), `upload` (`:598`), `encode_relax` (`:651`), `encode_kdk` (`:659`), `begin_readback` (`:671`),
  `take_readback` (`:698`). Setup `build_impact_bodies` (`:106`), `assemble_from_relaxed` (`:161`). Analysis
  `disk_stats_json` (`:211`), `moonlet_bodies` (`:287`), `largest_moonlet_orbit` (`:329`), `total_energy`
  (`:436`). Driven by `OrbitDemo` throughout (`lib.rs:2828/2846/2983/3594/3754/3781/3863/4368/4608/5082`).
- **accretion.rs** (298, 3 tests) — the growth operator (docs/33 stage 4c.3). Friends-of-friends clumping +
  boundedness + Roche gate: `Clump` (`:30`), `Clump::accretes()` (`:45`, internal KE + self-PE < 0 AND
  outside Roche), `find_clumps` (`:73`), `accrete` (`:166`, promotes a bound clump to one body conserving
  mass/momentum/COM). **Half-wired:** `find_clumps` is called from `gpu_sph.rs` (`:266/:315/:357/:412`) for
  moonlet detection, but **`accrete()` has no non-test caller** — the disk can be *measured* for clumps and
  cannot yet *grow* one. `tools/impact-run/src/main.rs:797` reimplements it instead of calling it.

## 3. Terrain / voxel / atmosphere modules

- **matter.rs** (2005) — CPU voxel-matter solver + the **bulk↔grain bridge**. `Particle`/`MatterSim`;
  promotion `dig`/`impact`/`materialize_region`/`materialize_furrow`/`materialize_steep_terrain`/`collapse`;
  de-resolution `deposit_resting_grain` (**single source of truth**, shared by CPU step AND GPU readback,
  `:795`); `step` (`:844`) (COM gravity + terrain snap-contact + settle — **no grain-grain contact on CPU**,
  that's GPU-only). All representation changes conserve matter + inject no energy (grain born at rest at its
  voxel centre). **Carries the third terrain-contact implementation** (`:872-887`, see §4.7): integer
  `surface_top_voxel` column top (no bilinear surface, no gradient, no normal), a hard position snap
  `p.pos.y = ground_y + PARTICLE_HALF`, and a scalar isotropic `p.vel *= CONTACT_DAMP` (`:33`, 0.15) with no
  μ and no normal load — the exact velocity-multiply fudge that `lib.rs:1437-1444` records as removed from
  the probe path. It survives here for CPU debris. Note `matter.rs:27-29` claims this class of
  non-conservative heightfield contact was resolved; that refers to the GPU/probe path, not to this snap.
- **world.rs** (1637) — the voxel matter store (`voxels:Vec<u16>`) + layered-Earth generator + terrain
  queries. `surface_height_bilinear` (the collision surface that MIRRORS `particle_step.wgsl::terrain_h`)
  now delegates to `surface_bilinear_grad`, which returns `(h, ∂h/∂x, ∂h/∂z)` — the surface NORMAL, without
  which a body on a slope has no normal impulse to bound friction with (PR #15). `find_structurally_
  unsupported` (cantilever support L_max≈√(σt/ρg) — basalt holds ~22 m, emergent from strength),
  `ocean_pressure` (continuous with the atmosphere column). Layered strata grass→basalt→peridotite→iron.
  **The T0 persistent field (commit `3d840ac`, docs/33 stage 6 sub-stage):** `World.displacement:Vec<f32>`
  (`:82`) is a w×d field of cell-CENTRE samples, so `bulk_height` (`:223`) is now
  `terrain_height + displacement_at − c.y`; `displacement_at` (`:233`) bilinear-samples it with a half-cell
  shift (`:237` — that half-shift was a real off-by-half, caught at 21.000 → 20.834 m);
  `demote_column_to_field` (`:271`) bakes a column's voxel surface into the field as the residual vs the
  procedural relief; `column_is_bakeable` (`:302`) is the admission test (false if a void hides under the
  top, since baking would silently delete it); `World::from_voxels` (`:89`) is the single construction seam.
  **Substrate only — nothing calls demotion in production**; both new fns appear solely in `world.rs` tests.
  The quiescence trigger, per-region tracking, and the bump/normal map are open. Its commit message states
  the rule worth carrying: *promotion asks whether the cheap model provably DIFFERS; demotion asks whether
  it can REPRESENT the state.*
- **mesher.rs** (1004) — surface meshing (Surface-Nets smooth terrain, curved `build_earth_cap`, sea,
  instanced debris/probe). Purely representational — "physics unchanged, this is the visual." All meshes emit
  one `Vertex{pos,nrm,col,mat}` → one triplanar pipeline.
- **body.rs** (270) — rigid `Sphere` under the field (semi-implicit Euler + voxel collision). NOTE: the live
  probe is actually an `Aggregate` (`lib.rs:2251`); `Sphere` is the debris-collision proxy + native tests —
  a SECOND rigid-body representation.
- **atmosphere.rs** (809) — **the SPH air field + the hydrostatic-balance template.** `sph_w`/`sph_dw` (the
  ONE cubic-spline kernel, shared with aggregate's vapor, `:39/:52`), `gas_contact_from_material` (stiffness
  from isentropic bulk modulus K=γ·P_ref, `:19`), `AirField` (`:96`): `compute_density` (ρ=Σm·W + mirror
  ghosts), `accelerations` (symmetric momentum-conserving `a=−Σm(P_i/ρ_i²+P_j/ρ_j²)∇W`, EOS `P=ρ·R_s·T`,
  `:220`), `relax_step` (damped settling). VERIFIED 3D hydrostatic balance (`:643`). **This is the machinery
  a self-gravitating condensed-matter planet reuses** — swap the EOS call, replace uniform gravity with
  Barnes–Hut self-gravity (exists in aggregate.rs), drop the box ghosts, add the energy equation (exists in
  apply_thermo). Points b+d already exist — only the EOS is new (§5).
- **texture.rs** (247) — procedural per-material textures from optical properties, zero image assets.
- **terra/** (5 files, 910) — the docs/43 worlds-as-data planet scene, backing the `Terra` struct (§1).
  `terra/mod.rs` (9), `terra/world_def.rs` (221 — the world JSON schema: a scene defined as DATA the engine
  loads, rather than as a code path per scene), `terra/fly_camera.rs` (240 — ONE altitude-blended camera,
  orbit ⇄ ground, docs/43 Phase 4), `terra/raster.rs` (175 — equirectangular raster sampling),
  `terra/globe_mesh.rs` (126 — displaced cube-sphere globe mesh, Phase 3). `world_def.rs` is where a world's
  declared properties live, so it is the natural home for per-world physical data (atmosphere composition
  and mass, gravity, materials) as scenes stop being branches.

## 4. The unification map — laws shared, solvers/containers forked

**Already shared (scale-invariant), do NOT duplicate:**
- `granular::Contact`+`contact_accel` — used by CPU `Aggregate`, the GPU shader (hand-mirror
  `particle_step.wgsl:132`), and (gas sibling) `atmosphere`. `Contact::mix` unifies cross-material.
- `sph_w`/`sph_dw` — one kernel for `AirField` AND aggregate vapor.
- `Furrow`+`ExcavSurface`+`ejection`+`ejecta_energy_scale` — ONE excavation primitive for terrain meteor
  (`Engine::meteor` `lib.rs:867`) AND Theia (`OrbitDemo::step_substep` `lib.rs:3357`).
- `plough_loft` — shared terrain↔giant-impact downrange drag.
- `deposit_resting_grain` — one de-resolution primitive (CPU + GPU).
- `Body` (universal particle), `Material`/`LayeredBody` (universal matter description).

**Forked (what a unified module must absorb):**
1. **Two container universes** — `Aggregate` (`Vec<Body>`, f64, celestial) vs voxel `World`+`MassField`+
   `MatterSim` (f32, terrain). Same laws, different data structures/integrators. A tire lives in one, Theia
   in the other.
2. **SIX integrators over one law** (was four when this was written) — GPU trapezoidal-implicit (terrain
   grains, `particle_step.wgsl:394`), CPU Euler settle-only (`MatterSim::step:844` — no grain-grain
   contact), CPU Verlet/KDK block-timestep (`Aggregate`), SPH relaxation (`AirField`, a standalone struct
   duplicating aggregate's SPH), CPU KDK + adaptive Courant dt (`hydrostatic::step:295`), and GPU KDK
   (`gpu_sph::encode_kdk:659` driving `sph_step.wgsl`). Some of this is legitimate specialization under
   docs/46 §1 — stiff contacts and orbital SPH genuinely want different schemes — but *six* is not a
   defended number; it is an accumulated one.
3. **The rigid-boundary fork** — in an impact, Earth is simultaneously a few materialized grains AND a rigid
   `boundary` penalty sphere + monopole/Gauss `gravity_source` + `boundary_hole`. This is docs/28 root-cause
   #1, in code (`aggregate.rs`): Earth's bulk is NOT the same particles as its debris. impact.rs:814 marks it
   as the lower bound that blocks the isotopic-crisis fix.
4. **Two rigid-body reps** — `body::Sphere` vs the cohesive-`Aggregate` probe.
5. **Manual WGSL mirrors — now three of them, all silent-drift seams.** (a) The GPU contact law is
   hand-kept in sync with the Rust one (guarded by `tools/gpu-verify`), not generated from it. (b)
   `gpu_sph::SphEos:42` hand-transcribes the WGSL `Eos` struct, with `basalt()`/`iron()` coefficients as
   hardcoded literals rather than reads of `eos.rs` (§5). (c) `tools/gpu-verify`'s own `repr(C)` `Params`
   mirror — which has already failed silently in practice, a padding field left where the shader expected a
   real one, so the coefficient arrived as 0.0 while everything compiled and the test reported success.
   **A `repr(C)` mirror that drifts from its shader fails silently by default.** Treat every one of these as
   requiring an explicit cross-check test, not review.
6. **GPU vs CPU physical depth — and now TWO GPU steppers with different physics.** `particle_step.wgsl`
   (terrain) has grain-grain contact + heightfield + cooling but NO self-gravity, NO SPH, NO EOS, one global
   material's contact params, f32; it carries per-grain `u` and `rho` (docs/38) but `rho` is a placeholder
   ρ₀ nothing computes yet. `sph_step.wgsl` (space) has SPH density + Tillotson + Monaghan AV + self-gravity
   + du/dt, and NO granular contact or heightfield. The CPU `Aggregate` has most of both but is N≈512–1536
   bound. Unification is no longer "the GPU path + four missing physics" — it is **reconciling two GPU
   kernels that each hold half the law**.
7. **Terrain contact — three implementations of one law, plus a fourth for voxels.** (a)
   `granular::terrain_contact_resolve` (`granular.rs:310`, `TerrainContact` `:285`) is the declared physics
   of record — non-injecting constraint: normal-velocity clamp → Coulomb friction bounded by μ·jn → bounded
   velocity-decoupled position projection. Its **only** production caller is
   `Engine::collide_probe_with_terrain` (`lib.rs:1430`, call at `:1482`); everything else calling it is a
   test. (b) `terrain_resolve` in `particle_step.wgsl:345` (called from `cs_integrate:457`) — the hand-kept
   GPU mirror. (c) `MatterSim::step`'s snap+`CONTACT_DAMP` for CPU debris (`matter.rs:872-887`) — cruder,
   normal-free, still live. Plus (d) `body::Sphere::collide` (`body.rs:55`), a distinct voxel-MTV resolver.
   Unifying these onto (a) is a known open task and the count above is the honest starting point.
8. **`AirField` is a container fork with no consumers.** `atmosphere.rs`'s SPH is a standalone struct
   duplicating aggregate's, and docs/48 found it instantiated in **zero scenes** — while docs/33 §4.5 has
   already scheduled it for absorption into `Aggregate`. Anything built against its standalone API buys a
   known rewrite. Note also that the *verified drag interaction* (docs/26 emergence test 4) does not live in
   `AirField` at all: `atmosphere.rs:502` builds an `Aggregate` of body + air parcels with a
   `gas_contact_from_material` `Contact` and lets drag fall out of `granular::contact_accel`.

## 5. The EOS / pressure inventory (and the gap)

Every pressure law that exists:
1. **SPH vapor** `P=ρ·R_s·(T−L_v/c)` (ideal gas) — `aggregate.rs:731`(force)/`:965`(energy).
2. **Atmosphere ideal-gas EOS** `P=ρ·R_s·T`, stiffness K=γ·P_ref — `atmosphere.rs:19/:96/:220`.
3. **Contact penalty** `f=k·overlap`, k=E·r/m (Young's modulus) — `granular::contact_accel`. Linear-elastic
   penalty, NOT a thermodynamic EOS.
4. **Analytic hydrostatic P(r)** dP/dr=−ρg — `planet::pressure_at:130`. Uses declared PREM densities;
   density does NOT respond to the pressure it computes.
5. **Boundary/bond springs** `f=k·penetration` / `k·(dist−rest)`.
6. **Condensed-matter EOS — Tillotson** `P(ρ,u)` — `eos.rs:52` (`sound_speed_sq` `:88`). Full three-branch
   form: compressed/cold `P=(a+b/ω)ρu+Aμ+Bμ²`, expanded-and-hot with `exp(−αz²)`/`exp(−βz)` decay to the
   ideal-gas limit `aρu`, energy-linear blend across partial vaporization (E_iv<u<E_cv). Cited per-material
   constructors: `granite()` `:113`, `basalt()` `:129`, `peridotite()` `:148`, `iron()` `:167`,
   `for_material(name)` `:185`. Wrapped by `enum Eos {Tillotson, IdealGas}` `:202` so the closure is
   swappable per material.

### The gap, restated correctly (this section used to be wrong)

**This doc previously said "Condensed-matter EOS: CONFIRMED ABSENT." That is false and has been since
docs/33 stage 1 landed.** `eos.rs` (373 lines, 7 tests) implements Tillotson, verified against Benz &
Asphaug 1999 (basalt) and Wissing & Hobbs 2020 (iron compressed branch). The claim survived here for two
days, and CLAUDE.md still repeats it — a caution about what a stale map costs: it does not merely omit, it
actively tells a session to build something that exists.

**The real gap is WIRING, and it is uneven.** Non-test consumers of `crate::eos::` are exactly two:

- `hydrostatic.rs:25` — heavy use; every `HydroBody` stores `Eos::Tillotson` per particle (`:83/:118/:126/:164/:172`).
- `gpu_sph.rs:110` — `build_impact_bodies` uses `Tillotson::iron()`/`basalt()` for the CPU-side bodies.

So Tillotson is live in the **space band only**. In the terrain `Engine` / voxel / granular path there is no
EOS at all: solids still resist compression via the linear-elastic contact penalty (#3), and `GpuParticle`'s
`rho` (`lib.rs:1907`) is a **placeholder ρ₀ until stage 4b.2 computes it** — the field exists to feed an EOS
that does not yet read it. Planet layer densities remain declared PREM constants (`planet.rs`).

**A silent-drift seam worth knowing about (new fork, not in §4's original list).** The GPU SPH path does
*not* flow through `eos.rs`. `gpu_sph::SphEos` (`gpu_sph.rs:42`) is a hand-transcribed 48-byte mirror of the
WGSL `Eos` struct, and `SphEos::basalt()` `:76` / `SphEos::iron()` `:79` are **hardcoded float literals**
with no compile-time link to `eos.rs:129/167`. `lib.rs:3754` calls `SphEos::basalt()/iron()` directly. Two
copies of one material's EOS coefficients, free to diverge, with nothing to catch it — the same failure
class as the WGSL contact mirror (§4.5), and the same class that bit gpu-verify's `repr(C)` Params mirror.

## 6. Scene wiring — the birth-of-the-Moon path (the canonical trace)

*(Anchors re-verified 2026-07-19 — every one had shifted by ~500–800 lines as `OrbitDemo` moved.)*

`orbit.ts` `OrbitDemo.create` → `start_birth` (`lib.rs:3447`, swaps body-2→Theia, inbound geometry
b=1.46·contact → emergent ~46° obliquity, zeroes proto-Earth spin) → per-frame `demo.advance(dtS)`
(`:3819`, wall-clock fixed-dt substeps) → `step_substep` (`:3968`): `verlet_step` → `swept_first_contact`
(`:3995`) → `contact_velocity` → **`build_impact_debris_scaled`** (`:4082`, Theia+Earth profiles,
512+1024 grains, converts `spin_l`→ω) → `moon_debris:Aggregate` → **`step_block`** (`:4155`, Barnes–Hut
self-gravity + grid contact + SPH vapor + boundary) → momentum-exact two-way coupling back to Earth,
boundary torque → `spin_l` (day length), tidal/J2 kicks, `drain_settled` demotes rested matter → Earth →
`push_snapshot` (`:4238`) → `render` (`:4348`, samples `RENDER_LAG_S` behind live; draws Earth as a
512-grain oblate shell, debris provenance-tinted blue=Earth/orange=Theia) → HUD `disk_stats_json` (`:3513`).

**There is now a SECOND path through this scene.** The trace above is the CPU `Aggregate` one. `OrbitDemo`
also holds `sph_snapshot: Vec<gpu_sph::SphParticle>` (`:2846`) fed by the GPU SPH stepper, with its own HUD
readout `gpu_disk_stats_json` (`:3777` → `gpu_sph::disk_stats_json`). So the birth scene has a CPU aggregate
path and a GPU SPH path coexisting — docs/33 said "the deployed birth scene is still the pre-realignment
`OrbitDemo`", which was true when written and is now only half true. **Establish which path a given run
actually exercises before attributing a number to it**; two paths, one scene, is precisely the shape docs/46
§1 asks you to justify or dissolve.

## 7. Render + GPU compute

**Nine WGSL shaders** in `shaders/` (1,462 lines) — this section used to list five.

- **Render** — terrain `Engine`: `sky.wgsl` (70, Rayleigh tri) → `world.wgsl` (107, triplanar material,
  water Fresnel) → `particles.wgsl` (46, instanced cube per grain). Space `OrbitDemo`: `space.wgsl` (53, lit
  sphere, per-instance model matrix; every element — Sun, Earth shell, crater walls, moon, debris — is the
  same unit `sphere_gpu` drawn per `UniformSlot`, zero-scale = hidden) **plus `sph_render.wgsl`** (64,
  instanced billboards drawn DIRECTLY from the `sph_step` particle buffer — zero-copy, the physics buffer IS
  the instance vertex buffer; Earth-relative f32 positions transformed per-instance to avoid planetary-
  coordinate cancellation). `Terra`: `globe.wgsl` (53, displaced cube-sphere with per-vertex biome albedo +
  a view-dependent atmospheric limb).
- **GPU compute 1 — `particle_step.wgsl`** (483, terrain `Engine` ONLY): `cs_grid_clear`/`insert` (spatial
  hash) → `cs_forces` (grain-grain contact from 27 neighbour cells, builds implicit tensor) → `cs_integrate`
  (directional trapezoidal θ=0.70 implicit contact solve + non-injecting `terrain_resolve:345` + cooling) →
  `cs_expand` (1 grain → 8 render sub-cubes). Has the hard GPU parts (spatial hash, stable implicit solve,
  4-pass barriers, non-blocking readback). Does NOT do self-gravity / SPH / EOS. Carries per-grain `u` and
  `rho` (docs/38) but one global material's contact parameters.
- **GPU compute 2 — `sph_step.wgsl`** (269, space band, docs/33 stage 4): SPH density ρ_i=Σm_j W(r_ij,h_ij)
  → Tillotson `P(ρ_i,u_i)` → pressure force a_i=−Σm_j(P_i/ρ_i²+P_j/ρ_j²+Π_ij)∇W with Monaghan artificial
  viscosity → direct self-gravity → du/dt. Same physics as `hydrostatic::forces_and_dudt` in f32, verified
  against an independent f64 CPU computation on the RTX 2070 (RMS rel error 1.9e-6).
- **GPU compute 3 — `bh_gravity.wgsl`** (317, docs/36/37): GPU Barnes–Hut over an LBVH — the O(N log N)
  replacement for `sph_step`'s direct O(N²) gravity loop, same softened Newtonian law, distant subtrees
  approximated below opening angle θ. **Read its own header before assuming it is live:** *"Built + verified
  standalone in tools/gpu-bh-verify BEFORE it is wired into the SPH step."* Another instance of docs/48's
  built-then-wired-nowhere pattern — check the call site, don't infer it.

**Verification tools** (`tools/`, native binaries — the pattern is *verify the GPU against an independent
CPU implementation out-of-process*): `gpu-verify` (terrain contact law vs the Rust one), `sph-verify`
(`sph_step.wgsl` vs an independent f64 `HydroBody` reimplementation), `gpu-bh-verify` (`bh_gravity.wgsl` vs
`bhtree.rs`), plus `bake-earth`, `impact-run`, `shot-server.mjs`. Note `gpu_sph.rs` has **0 in-crate tests**
by design — its correctness lives entirely in `tools/sph-verify`, so a change there is unguarded unless you
run that tool.

## 8. Workflow

**Moved to [`CLAUDE.md`](../CLAUDE.md) → "Hard rules (do not violate)".** This section used to restate
those rules, and had already drifted from them — it still said "next is docs/34" (it is now docs/49) and
quoted a stale test count. That is the doc-level form of what docs/46 forbids in physics: one question,
two answers, free to diverge. CLAUDE.md is the single source; nothing here was map-specific enough to
justify a second copy.

## 9. Docs index (the design record — `docs/NN-slug.md`)

01 prior-art · 02 oss-building-blocks · 03 **versioning** (SemVer; games pin exact) · 04 materials-model ·
05 data-pipeline · 06 material-modules · 07 material-taxonomy-and-objects · 08 **adaptive-resolution-and-
clumping** (represent matter at the coarsest resolution that still behaves right; refine/coarsen LOD) · 09
agentic-object-authoring · 10 robustness-and-common-pitfalls · 11 networking · 12 textures · 13
**scale-relative-simulation** (cost scales with what's observable; simulation LOD, not just render LOD) · 14
validation-demonstrations · 15 **representation-invariant** (the cube is a lattice, not matter) · 16
**unified-dynamics-and-awake-set** (every dynamic solid is the same matter in one awake-set loop) · 17
honest-appearance-and-observer-frame · 18 unified-deformation-and-damage (a bullet, a splash, the Moon =
same code, different params/LOD) · 19 lod-adaptive-damage · 20 impact-thermodynamics · 21
bodies-as-particle-aggregates · 22 **gpu-compute-particles** (the particle step belongs in WGSL compute;
zero-copy sim↔render; terrain done, space aggregate is the undone step 4) · 23 **everything-is-matter-north-
star** (the no-fudge charter) · 24 **emergent-impact** (ejecta from real compression→rebound, not scripted
v) · 25 layered-planets-and-atmosphere · 26 **atmosphere-as-matter** (air is particles too; the five
emergence tests — note test 4 defines drag as MOMENTUM-CONSERVING) · 27 birth-of-the-moon · 28 **missing-
impact-physics** (audit; root cause #1: Earth is a rigid boundary) · 29 deployment · 30 **accelerated-
compute-module** (grid + Barnes–Hut + block timesteps) · 31 **isotopic-crisis** (spin isn't the lever;
needs Earth-as-matter) · 32 this map · 33 **architecture-realignment** (the staged plan) · 34
stage-4c-pickup · 35 gpu-path-migration (stage 5) · 36 gpu-barnes-hut-spec · 37 **gpu-barnes-hut-findings**
(built, verified, measured vs direct-sum) · 38 **terrain-gpu-unification** (terrain onto the one GPU path;
the grain carries `u`/`rho`, not `temp`) · 39 **planetary-scale-jit-particalization** (T0↔T3 as ONE
primitive) · 40 converge-earth-fraction-pickup · 41 **earth-fraction-converged** (the ensemble result) · 42
pretty-render-layer · 43 worlds-as-data · 44 **resolution-by-necessity** (the interaction's energy sizes its
own footprint; *necessity* decides what we SIMULATE, *interest* only what we DRAW) · 45
**terrain-slope-stability** (Mohr–Coulomb, not a magic step height) · 46 **one-physics-charter** (THE rule —
specialization vs declared vs fudge — plus the conformance ledger; read before adding physics, add a row
when you find a violation) · 47 **go-kart-and-particle-granularity** (granularity is per-interaction, not
per-object) · 48 **the-atmosphere-is-built-and-unwired** (a verified atmosphere instantiated in zero
scenes — and the "built, then wired nowhere" pattern it names).
