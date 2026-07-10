# Emergent impact — ejection from compression + rebound, not a scripted velocity

> Robin: *"Ejection of matter should be caused by compression/rebound."* Right now `matter::impact`
> hands every ejecta grain a number — `v = √(2·(0.05·E)/m)`, aimed outward. That is a script, not
> physics. The honest crater is what really happens: the meteor **compresses** the target, the
> compressed material **rebounds**, and that rebound **throws matter out** — the ejecta curtain and the
> bowl both *emerge*. No assigned ejecta velocity anywhere. This is the impact half of the
> everything-is-matter north star (`docs/23`): the meteor is a real body, the ground is real matter, and
> the same contact law we already trust does the rest.

## The real mechanism (what we're reproducing)

1. **Contact + penetration.** The impactor drives into the surface.
2. **Shock/compression.** A pressure wave races ahead, compressing the target — grains driven together,
   storing elastic energy in their contacts (½·k·δ² per contact).
3. **Rebound + excavation flow.** Behind the shock, the compressed material unloads (rarefaction) and
   flows — down and out at the centre, up and out toward the rim — carving the bowl and launching the
   **ejecta curtain** (roughly a 45° cone, fast near the centre, slow at the rim).
4. **Fallback + heat.** Sub-escape ejecta rain back (breccia); the dissipated energy is **heat**
   (`temp_k`), radiated — see the energy-conservation note in `docs/23`.

Every step is contact mechanics on real matter. Our job is to stop scripting step 3.

## The hard problems (why this is a subsystem, not a patch)

1. **Velocity scale / tunnelling.** Real impacts are km/s; a grain crossing >1 m per substep tunnels.
   17 km/s would need ~16 000 substeps/frame — impossible real-time. So the *microsecond shock itself*
   cannot be explicitly resolved. The honest move is to conserve the meteor's real **momentum + energy**
   and let the (resolvable) **rebound + flow** be emergent — the sub-µs shock is the one thing we
   approximate, and we say so. Options to evaluate: momentum-impulse into the contact region; a
   sub-frame continuous-collision pass; or depositing the energy as real **pre-compression** (contact
   overlap = stored PE) that the physics then releases.
2. **Elastic rebound vs. inelastic settling.** Ejection needs the contact to *store and return* energy
   (restitution); settling needs it to *dissipate*. Real rock does both — elastic under the shock,
   plastic/frictional afterward. Our current contact is heavily damped (great for settling, kills
   rebound). Fix: derive normal damping from each material's real **coefficient of restitution**
   (already in `materials.json`), so rebound is a material property, not a dial.
3. **Terrain as matter at the impact.** Compression/rebound needs the *target* to be grains, not a
   heightfield, at least in the impact region. Materialize a patch of terrain into grains on impact
   (the `docs/19` LOD-materialization bridge, made real), run the contact physics there, re-freeze to
   voxels once it settles.
4. **Directional contact stabilization.** The current implicit stabilizer is isotropic and damps free
   flight (`docs/23`) — it would smother the very ejection we want. Needs the directional (per-normal)
   form first, or the ejecta never leave.

## Staged plan (each stage verifiable on the 2070 via `tools/gpu-verify`)

- **Stage 0 — directional implicit.** Replace the isotropic `1/(1+dt²K)` with a per-grain stiffness
  tensor (damp only along contact normals). Test: energy still can't rise AND a grain's free/ejection
  velocity survives (an unconstrained grain keeps its speed). Prerequisite for any ejection.
- **Stage 1 — real restitution.** Normal damping from material restitution; add a bounce test (a grain
  dropped rebounds to ~e²·h). Now compressed grains *can* rebound.
- **Stage 2 — compression, not ejecta.** `matter::impact` stops assigning outward velocity. Instead it
  deposits the meteor's momentum/energy as **compression** on a materialized grain patch (inward drive
  / contact pre-load). Test (2070): a compressed patch on a constrained bed throws a curtain — grains
  leave up-and-out with *no* assigned outward velocity; ejecta speed falls with radius (excavation
  signature); energy never rises.
- **Stage 3 — meteor as a real body.** The impactor is a Fe-Ni grain body with real mass; its momentum
  transfers by contact. Reconcile the km/s scale honestly (impulse vs. sub-frame CCD), flagged.
- **Stage 4 — the crater is the flow.** The bowl + rim emerge from the excavation flow re-freezing to
  voxels; no `dig`-style carve. This closes the loop with `docs/23`'s "the crater is what happens."

## The test that keeps us honest

Every stage runs under the **energy fudge-detector** (`gpu-verify` scene I): total mechanical energy may
only fall (the rest is heat). Ejection that *emerges* passes it; ejection that's *injected* (a teleport,
a scripted velocity that over-adds energy, an unstable contact) fails it. That is how we know the
crater came from physics and not from us.
