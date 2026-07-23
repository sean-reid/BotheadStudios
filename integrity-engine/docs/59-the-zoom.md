# The zoom: celestial to local, one event (docs/59)

> North-star step 3 (docs/23): the Moon-Earth collision is a celestial energy event; zooming into
> ground zero materializes the local matter (the ball and a terrain patch) and runs the same impact
> thermodynamics there, with the state carried down from the celestial field. The ball's destruction
> at the small scale is the same event as the flash at the large scale, conserved across resolution.

This is the first production trigger for resolution-by-necessity (docs/44), which the charter calls
the largest gap between the promise and the code (docs/46). It is governed by Law III (simulate what
necessity requires), Law IV (the camera changes representation, never existence), Law V (every
deferred computation is a named IOU), and Law VII (methods and constants are sourced, not invented).

## What exists and what is missing

Exists: the SPH machine resolves both declared and live impacts through one assembly; the settled
field is read back each frame; the ball is declared matter in a world definition; surface-nets
meshing; the de-res ladder's downward rungs (zero production consumers today); Terra's cube-sphere
globe and fly camera.

Missing, as five separable pieces: one body definition serving both the orbital presence and the
local surface; a descent camera that holds f32 precision to 2 m above the surface; the camera-driven
materialization trigger; conserved initialization of the fine patch from the coarse field; and
re-coherence of the settled site into meshed ground.

## The method, sourced

**One-shot refinement at zoom commit, not continuous adaptivity.** No production impact code refines
continuously mid-run: giant-impact SPH holds equal masses and buys convergence with particle count
(Kegerreis et al. 2019, MNRAS 487:5029, arXiv:1901.09934), and the DART impact studies grade
resolution statically in the initial conditions, finest at the impact point with ~1% growth per
shell (Stickle et al. 2022; Owen et al. 2022, Planet. Sci. J. 3). Compressible astrophysical SPH has
one modern existence proof of true adaptive refinement (Nealon and Price 2025, PASA 42,
arXiv:2409.11470). The honest browser-scale version is therefore event-driven: when the camera
commits to the descent, split the patch once, at the most quiescent moment available.

**Splitting that conserves by construction.** Parent particles split on the icosahedron stencil with
a mandatory child retained at the parent position (Vacondio et al. 2016, CMAME 300:442). Children
inherit the parent velocity and specific internal energy; child masses sum to the parent's. Mass,
momentum, angular momentum, kinetic and internal energy are then exact up to rounding (Feldman and
Bonet 2007, IJNME 72:295). The scheme's entire error is a density blip at the interface, bounded by
the placement optimization (separation ~0.4 h, child smoothing ~0.9 h in the source; re-derive the
constants for our kernel offline). With a stiff Tillotson material a small density error is a large
pressure error, so:

**Relax, then release.** Sample the coarse SPH-interpolated fields at the child sites, then relax
child positions against that target density with a frozen clock and damped shifting, the coarse
exterior held as a guard band, before the event proceeds (the accepted initialization discipline:
Diehl et al. 2015, arXiv:1211.0525; interface treatment per Chiron et al. 2018, JCP 354:552).

**Interface discipline.** Resolution ratio at most 2 per interface, no cross-level interactions,
buffer shells between levels, and contamination as a first-class failure: a coarse particle
penetrating the fine patch invalidates the run and says so on screen (the cosmological zoom-in
practice, Hahn and Abel 2011, arXiv:1103.6031). A hidden smoothing-over here would be a fudge.

**Validation gate: crater scaling, not eyeballing.** The refined patch's crater is checked against
Holsapple-Housen pi-group scaling in the gravity regime (Holsapple 1993, Annu. Rev. Earth Planet.
Sci. 21:333; coefficients from the Holsapple-Housen v2.2.1 table, hard rock K1 0.012, mu 0.55 and
regolith K1 0.14, mu 0.4, with the coefficient vintage named in the test). Factor-of-two agreement
in rim diameter passes; when the crater approaches the body's own radius the check degrades,
explicitly, to an order-of-magnitude sanity bound, because pi-scaling assumes a point source.

**An energy ledger per event.** Kinetic, internal, and participating potential energy are audited
across the bridge and the drift shown, not hidden (the decomposition of Carter, Lock and Stewart
2020, JGR Planets 125, arXiv:1912.04936, whose reported 3-16% conservation errors traced to EOS
evaluation, the same term that will dominate ours). Fixed energy-partition fractions never enter
the code; the ledger computes each event's own split. Literature ranges (heating 20-60%, escaping
ejecta a few to 10%) appear only in tests as sanity bounds, cited.

## Order of work

1. One Earth: a single body definition owns the orbital body and its local surface patch.
2. Descent camera: floating-origin rendering to 2 m altitude, and the materialization trigger,
   which deliberately mirrors the moon-drop's resolution-distance idiom so the engine has one
   materialization pattern. These two can proceed in parallel after (1).
3. Conserved hand-down: split, relax, release, with the pi-scaling gate and the energy ledger.
4. Re-coherence: the settled site returns to meshed ground through the surface-nets rung.

## Status, 2026-07-23

Item 1 landed (one Earth, see the ledger row 16 narrowing). Item 2's trigger half landed together
with item 3's entry point: `crate::site` derives the view-necessity threshold (one coarse SPH
particle's matter share against the docs/49 angular budget, measured from the live field when one
exists) and the space band materializes the Ground Zero world's declared ball and strata patch
through `crate::refine` on the descending crossing, folding back through the docs/61 gauge on the
ascending one, bidirectional for the out-and-back demo arc, ledger and refusals on the HUD. Open
within items 2 and 3, carried by docs/46 row 18: the descent camera below the orbit camera's
floor; releasing relief surfaces in the relax (the shipped site is the exact conserving split
with its density residual stated); the mid-event hand-down and any entry of the fine patch into
dynamics; the pi-scaling gate still awaits its end-to-end consumer.

## IOUs this design leaves open, named

- Melt, vapor, and comminution tracking in the fine patch (fractions per Gault and Heitowit 1963;
  O'Keefe and Ahrens 1977) are deferred; the ledger reports their absence.
- Impactor spin at the live hand-off remains a zero vector until per-body angular momentum exists
  in the N-body state (docs/58 item 3).
- Resolution chosen from impact energy and view scale beyond the fixed patch budget (docs/44,
  docs/47) stays deferred; the patch budget is a declared compute statement.
- Continuous adaptivity (split and merge during the event) is out of scope; merging cannot conserve
  kinetic energy exactly and is not needed for a one-shot zoom.

## Open questions (Robin)

- Where docs/58 item 7 lands collision routing determines who owns the trigger's entry point.
- Which representation owns the shared Earth: the world definition (docs/43) is the natural home,
  but Terra and the space band both carry private builds today.
- Whether the patch budget rides the existing 2400-particle statement or gets its own.
