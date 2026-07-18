# GPU Barnes–Hut — built, verified, and measured against direct-sum (docs/36 result)

This is the RESULT of the `docs/36` build spec. The GPU Barnes–Hut (LBVH) self-gravity solver is built and
**verified correct**; the performance measurement **disconfirms the docs/36 premise** that it restores
in-browser fps. Read this before wiring anything (docs/36 stage "Wiring into the engine").

## What was built — `tools/gpu-bh-verify` + `shaders/bh_gravity.wgsl`

A standalone native-Vulkan crate (same pattern as `sph-verify`/`impact-run`) hosting the full LBVH pipeline
as WGSL compute kernels, each verified against an independent CPU reference before the next is trusted:

| Stage | Kernel | Verified against | Result |
|------:|--------|------------------|--------|
| 0 | `cs_gravity_direct` | CPU f64 direct sum | RMS 2.4e-6 (f32) |
| 1 | `cs_bbox_reset`/`cs_bbox` (float-radix atomicMin/Max) | CPU min/max | **exact** (lossless u32 encoding) |
| 2 | `cs_morton` (30-bit) | CPU Morton | **bit-exact**; coincident→equal |
| 4 | `cs_tree_reset`/`cs_tree` (Karras 2012) | structural (reachability + parent/child) | every leaf reached once, pointers consistent |
| 5 | `cs_com` (atomic children-ready climb) | root Σm / Σm·x / AABB | mass 1.0e-8, COM 8.2e-8 — **atomic climb is coherent** |
| 6 | `cs_gravity_bh` (θ-traversal) | GPU direct-sum + CPU f64 direct | see below |

**Accuracy (the correctness bar, θ-traversal vs CPU f64 direct at N=1500):** θ=0.5 → RMS **0.70 %**, max 6.2 %;
θ=0.25 → 0.12 %; θ=0.1 → 0.009 %; **θ→0 → 1.8e-6 (recovers the exact direct sum)** — the strong structural
proof that every particle is reached exactly once and the COM/tree are correct.

Design notes that mattered: the opening criterion is the **robust (Salmon–Warren / Barnes 1994) MAC** using the
AABB **diagonal** as the node size plus the centre↔COM offset δ — a plain `maxside/dist<θ` on a *tight* box
under-opens and left a 28 % worst-case particle; the diagonal+δ form keeps the tight adaptive box (mandatory
for resolution, docs/36) AND caps the error. Traversal runs in **Morton order over a permuted `sbodies[]`** so
adjacent threads walk coherent paths and read contiguous memory (both the self-read and leaf-bucket sums
coalesce). **Leaf bucketing** (K particles/leaf) is parameterized (`bucket_k`; K=1 = classic LBVH).

The interim sort is a **CPU sort** (read back → sort (code,index) → upload cluster codes + sorted order +
sorted bodies). The GPU radix sort (docs/36 stage 3) was **not built** — see the recommendation.

## The measured finding — direct-sum wins below N≈128k (RTX 2070)

Per-eval GPU wall time of the **traversal** kernel vs the direct-sum kernel, θ=0.5, uniform cloud:

| N | BH ms (K=1) | direct ms | BH speedup |
|------:|------:|------:|------:|
| 2 000 | 0.63 | 0.40 | 0.63× |
| 8 000 | 1.81 | 1.62 | 0.89× |
| 32 000 | 9.9 | 8.5 | 0.86× |
| 128 000 | 40.1 | 86.3 | **2.15×** |

Asymptotic scaling (32k→128k) is exactly as theory predicts — **direct → O(N²) (p≈1.84), BH → O(N log N)
(p≈1.0)** — but the *crossover* where BH overtakes direct-sum is **N≈128 000**. Leaf bucketing (K=8/16/32)
does **not** lower it: buckets improve accuracy (larger exact leaf sums, RMS→6e-4) but cost more traversal
time (more per-thread leaf work), so K=1 has the lowest crossover. A 32- vs 64-deep traversal stack made no
difference (not register-spill-bound). The tree BUILD cost (bbox→morton→sort→tree→COM, every force eval) is
**on top** of the traversal number, so full-pipeline BH is even less favourable at low N.

**Why:** GPU direct N-body is the near-ideal GPU workload — every thread reads the same `bodies[j]` in
lockstep (broadcast), pure coalesced FMA, compute-bound, ~2070-peak FLOPS. Barnes–Hut trades those cheap FLOPs
for **divergent, memory-bound tree traversal** (per-thread stack walks, scattered 64-byte node fetches). On
this hardware the trade only pays once N² dwarfs the tree work — past ~128k. (Our direct-sum baseline is the
*naive* loop, not even shared-memory-tiled; a tiled direct sum would push the crossover *higher* still, so
this is a conservative, BH-favourable measurement.)

## Recommendation (no-fudge: the measurement, not the hypothesis)

**Do NOT wire GPU Barnes–Hut into the browser SPH step (docs/36 "Wiring" / stage 8).** The browser impact
runs at N≤~20k and the offline `tools/impact-run` converges at N≈35k — **both are far below the 128k
crossover**, so BH would *reduce* fps there, not restore it. The docs/36 premise ("restores fps at 8k, unlocks
20k") is disconfirmed for a per-thread GPU traversal on the 2070. Keep the direct O(N²) gravity for N≤~100k;
it is the right tool in that range.

Additional context: at N=8k gravity is only ~25 % of the browser frame (1.6 ms × 40 evals = 64 ms of a ~250 ms
/ 4 fps frame), so even *free* gravity would lift 4→~5.3 fps — **gravity is not the browser fps lever**. The
levers are elsewhere (substep count, the SPH grid pass, render, WebGPU overhead).

**Where BH IS the right tool: very-high-N offline convergence (N≳128k).** The disk isotopic fraction is
scatter/relaxation-noise-limited (docs/28 ceiling, docs/35) and wants an ensemble at higher N. At N≳128k, BH
gives a real and growing speedup (2.15× at 128k; extrapolating O(N log N) vs O(N²): ~9× at 512k). So the open
decision for Robin:

- **(A) Pursue a converged disk number** → finish the GPU radix sort (docs/36 stage 3, the one hard kernel —
  positions change every step so an offline run also needs it on-GPU), then run `tools/impact-run` at N≳128k
  with BH gravity. This is the only path where the BH work pays off.
- **(B) Defer.** The verified BH crate + kernel are banked and re-verifiable; leave direct-sum in place
  everywhere. Revisit if/when a high-N convergence campaign is scheduled.

The GPU radix sort was deliberately **not** built yet because it is gated on this decision (it is only needed
for an on-GPU pipeline, i.e. option A) and is the most expensive remaining kernel — building it speculatively
would burn budget on a path we may not take.

## Reproduce

`cd tools/gpu-bh-verify && cargo run --release` (RTX 2070, native Vulkan). Prints PASS for every stage +
the scaling/bucketing table. Correctness bars: bbox exact, morton bit-exact, tree structural, COM <1e-6,
θ=0.5 RMS <1 %, θ→0 recovers direct, direct-sum → O(N²).
