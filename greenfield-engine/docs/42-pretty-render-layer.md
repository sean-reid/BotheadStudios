# docs/42 — the "pretty render" layer over the GPU physics (design + build plan)

**Robin's vision:** the GPU SPH giant impact is the REAL physics; over the top, a "pretty" render as faithful to
how it would look to human eyes as possible — a **sphere** for Earth/Theia at rest, a **crater** carved on impact,
**atmosphere mist**, **ejecta particles** thrown from the field, clumps **resolving into spheres** as the disk
accretes into the Moon. A **slider** cross-fades physics-particle-view ⇄ pretty-render. This is the render-side of
the JIT primitive: `sphere (field) → particalize on event → simulate → bake back to sphere`, mirroring the physics.

**Why this is the right architecture:** it DECOUPLES physics-fidelity from visual-fidelity. The in-browser SPH is
N-limited and fixed-dt (WebGPU forbids the adaptive read-back — docs/41); making the raw particle disk look photoreal
fights those limits. With a pretty layer, the particles only need to be physically RIGHT (Earth sheds, a clump
accretes — the offline `tools/impact-run` is the converged truth); the pretty layer carries the look.

## What already exists (don't rebuild)

The CPU birth scene's Earth render (`OrbitDemo::render`, `lib.rs` ~3841–4176) is ALREADY most of the pretty layer:
- **Earth as a shell of 512 grains** (`shell_unis`, `self.sphere_gpu` instanced) with **continents/oceans** from a
  co-rotating landmask (`planet::earth_surface_material`), a **Rayleigh atmosphere veil** (`atmosphere::rayleigh_veil`
  + two-way transmittance), and **spin oblateness** (`tides::flattening_from_spin`, +f/3 equator / −2f/3 poles).
- **A crater**: `crater_site` + `hole_radius()` hide grains inside the bowl and draw `wall_unis`; it CO-ROTATES with
  the crust (`spin_rot`). The `interior_uni` glowing mantle shows through.
- **The Moon** as its own grain shell (`moon_unis`); **debris** as grains (`debris_unis`).

The gap: it is **CPU-driven** (from `self.bodies`, `self.spin_l`, `self.hole_radius`), and it is **mutually exclusive**
with the SPH particle render — `if !self.sph_active { …grains… }` vs `if self.sph_active { …billboards… }`. During the
GPU impact, `advance()` skips the CPU orbital physics, so the CPU state is frozen (pristine, non-spinning Earth).

## The core design problem: derive the pretty state from the GPU field

The pretty layer must read its state from the GPU SPH snapshot (`self.sph_snapshot`), not the frozen CPU bodies:

| pretty element | derive from the GPU field |
|----------------|---------------------------|
| Earth sphere center + radius | remnant COM + the 85%-mass radius (already computed in `gpu_sph::disk_stats`) |
| spin (oblateness, crust rotation) | angular momentum of the remnant particles (Σ m r×v) → ω → flattening + spin_angle |
| crater site + radius | the impact contact point (Theia's entry, prov=1 first-contact) + shock energy → bowl size |
| ejecta particles | the SPH particles that are unbound / high-altitude (disk+escaping subset) — render as glowing motes |
| moonlet spheres | the bound disk clumps (`gpu_sph::disk_moonlets`, already there) → growing spheres |

**Scale reconciliation (critical):** grains use `DISPLAY_SCALE = 1/6.371e6` (real Earth radius → 1.0 unit); SPH
billboards use `SPH_VIS_SCALE = 7.0e-7` (the sub-scale 5000 km body blown up ~4.46×). To overlay them for the slider,
render the pretty sphere at the SPH remnant's DISPLAY radius (`remnant_m · SPH_VIS_SCALE`), i.e. drive the grain shell
off the SPH remnant radius, not `EARTH_RADIUS_M`. One consistent frame or the two views won't register.

**The slider:** a `render_blend` uniform (0 = pretty, 1 = physics) on `OrbitDemo`, set from JS (`set_render_blend`) by
a new UI slider in `orbit.ts`. Cross-fade by SIZE (no alpha-blend/depth-sort headache): grain scale `×(1−blend)`,
billboard half-size `×blend`. Both pipelines already draw `self.sphere_gpu` / instanced billboards; only the scales
change per frame.

## Phased build (each rig-verifiable on its own)

1. **Slider + sphere-at-rest.** Add `render_blend` + `set_render_blend`; render the grain-shell Earth during
   `sph_active`, positioned/sized off the SPH remnant (COM + 85%-mass radius); slider cross-fades grains ⇄ billboards.
   Verify: `birth_gpu` rig — a pretty Earth that fades to its constituent physics particles and back.
2. **Crater on impact.** Capture the GPU impact contact point (first Theia–Earth contact in the snapshot) + shock
   energy; feed `crater_site`/`hole_radius` so the existing bowl machinery carves the sphere. Verify: crater appears
   at the real impact longitude, co-rotates.
3. **Ejecta + atmosphere mist.** Render the unbound/disk SPH subset as glowing ejecta motes over the sphere; add
   translucent mist shells (extend the Rayleigh veil into a volumetric-ish band). Verify: ejecta plume + limb haze.
4. **Accreting moonlet spheres.** `disk_moonlets` clumps → grain-shell spheres that grow as they accrete; the largest
   becomes the Moon (hand off to `enter_geologic_time`). Verify: disk → moonlets → a Moon in orbit.

## Open decisions (for Robin)

- **Default blend:** start pretty (0, slider reveals physics) or physics (1, slider reveals the render)?
- **Theia render:** a second textured sphere inbound, or only Earth + ejecta?
- **Crater persistence:** does the post-impact Earth keep a visible scar into geologic time (bake-back), or heal?

## Status (2026-07-18)

Physics + fixes landed & pushed (docs/41: spin IOU, browser shock-dt, eyes icons; the birth scene auto-starts the GPU
impact). This doc is the render-layer plan; Phase 1 is the next build.
