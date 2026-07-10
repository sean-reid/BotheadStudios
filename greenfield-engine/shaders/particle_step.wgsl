// GPU particle step (docs/22, docs/23) — the debris sim, one compute invocation per particle.
//
// Now with PARTICLE-PARTICLE CONTACT (granular): grains push apart, stack, and flow to a natural
// slope, instead of resting only on the terrain heightfield (which left them stranded on ledges and
// unable to pile — the "moiré"). Contact needs neighbours, so each substep runs four passes:
//
//   1. cs_grid_clear   — zero the spatial-hash cell counts
//   2. cs_grid_insert  — bucket every particle into its grid cell (atomic)
//   3. cs_forces       — sum contact accelerations from the 27 neighbouring cells → forces[]
//   4. cs_integrate    — gravity + contact + drag + terrain collision + cooling, then move
//
// Splitting force-accumulation (pass 3, positions read-only) from integration (pass 4) avoids a
// read/write race on the particle buffer. The contact law is a WGSL mirror of `granular::contact_accel`
// (verified natively; kept in sync by construction).

struct Params {
    gravity      : vec3<f32>,
    dt           : f32,
    center       : vec3<f32>, // world.center() offset (centered→voxel coords)
    c_max_accel  : f32,       // cap on normal contact accel (prevents deep-overlap launches — docs/23)
    drag         : f32,
    contact_damp : f32,       // ground-collision velocity damping
    settle_speed : f32,
    part_half    : f32,
    cool_rate    : f32,
    count        : u32,
    world_w      : u32,
    world_d      : u32,
    // --- granular spatial hash + contact (docs/23) ---
    cell_size    : f32,       // grid cell edge (m); ≥ contact diameter so contacts stay within ±1 cell
    table_mask   : u32,       // hash table size − 1 (table size is a power of two)
    bucket_k     : u32,       // max particles recorded per hash bucket (overflow is dropped)
    c_radius     : f32,       // grain radius (m)
    c_stiffness  : f32,       // normal repulsion (1/s²) per metre of overlap
    c_normal_damp: f32,       // normal damping (1/s)
    c_friction   : f32,       // Coulomb μ
    c_tangent_damp: f32,      // tangential regularization (1/s)
};

struct Particle {
    offset   : vec3<f32>,
    temp     : f32,
    vel      : vec3<f32>,
    resting  : f32,
    color    : vec3<f32>,
    material : f32,
    emission : vec3<f32>,
    _pad     : f32,
};

@group(0) @binding(0) var<uniform> P : Params;
@group(0) @binding(1) var<storage, read_write> particles : array<Particle>;
@group(0) @binding(2) var<storage, read> heightfield : array<i32>;
@group(0) @binding(3) var<storage, read_write> grid_count : array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> grid_bucket : array<u32>;
@group(0) @binding(5) var<storage, read_write> forces : array<vec3<f32>>;

fn incandescence(t : f32) -> vec3<f32> {
    if (t <= 800.0) { return vec3<f32>(0.0); }
    let x = t - 800.0;
    let intensity = clamp(x / 2200.0, 0.0, 4.0);
    let gg = clamp(x / 2200.0, 0.0, 1.0);
    let bb = clamp((t - 2600.0) / 2400.0, 0.0, 1.0);
    return vec3<f32>(intensity, gg * intensity, bb * intensity);
}

// --- spatial hash ---------------------------------------------------------------------------------
fn cell_of(pos : vec3<f32>) -> vec3<i32> {
    return vec3<i32>(floor(pos / P.cell_size));
}
fn hash_cell(c : vec3<i32>) -> u32 {
    let h = (u32(c.x) * 73856093u) ^ (u32(c.y) * 19349663u) ^ (u32(c.z) * 83492791u);
    return h & P.table_mask;
}

// Acceleration on grain i from grain j — EXACT mirror of granular::contact_accel (docs/23).
fn contact_accel(pi : vec3<f32>, vi : vec3<f32>, pj : vec3<f32>, vj : vec3<f32>) -> vec3<f32> {
    let d = pi - pj;
    let dist = length(d);
    let touch = 2.0 * P.c_radius;
    if (dist >= touch || dist < 1.0e-9) { return vec3<f32>(0.0); }
    let n = d / dist;
    let overlap = touch - dist;
    let v_rel = vi - vj;
    let v_n = dot(v_rel, n);
    // Cap the spring term so a deep overlap (fast impact, dense jam) can't launch grains — the fix for
    // debris flung skyward and never settling (docs/23). Damping then subtracts to make it inelastic.
    let spring = min(P.c_stiffness * overlap, P.c_max_accel);
    let a_n_mag = max(spring - P.c_normal_damp * v_n, 0.0);
    let a_n = n * a_n_mag;
    let v_t = v_rel - n * v_n;
    let vt_mag = length(v_t);
    var a_t = vec3<f32>(0.0);
    if (vt_mag > 1.0e-9) {
        let mag = min(P.c_tangent_damp * vt_mag, P.c_friction * a_n_mag);
        a_t = -(v_t / vt_mag) * mag;
    }
    return a_n + a_t;
}

// --- pass 1: clear cell counts --------------------------------------------------------------------
@compute @workgroup_size(64)
fn cs_grid_clear(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i > P.table_mask) { return; } // table size = table_mask + 1
    atomicStore(&grid_count[i], 0u);
}

// --- pass 2: bucket each particle into its cell ---------------------------------------------------
@compute @workgroup_size(64)
fn cs_grid_insert(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= P.count) { return; }
    let h = hash_cell(cell_of(particles[i].offset));
    let slot = atomicAdd(&grid_count[h], 1u);
    if (slot < P.bucket_k) {
        grid_bucket[h * P.bucket_k + slot] = i;
    }
}

// --- pass 3: accumulate contact forces from the 27 neighbouring cells -----------------------------
@compute @workgroup_size(64)
fn cs_forces(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= P.count) { return; }
    let pi = particles[i].offset;
    let vi = particles[i].vel;
    let base = cell_of(pi);
    var acc = vec3<f32>(0.0);
    for (var dz = -1; dz <= 1; dz = dz + 1) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            for (var dx = -1; dx <= 1; dx = dx + 1) {
                let h = hash_cell(base + vec3<i32>(dx, dy, dz));
                let n = min(atomicLoad(&grid_count[h]), P.bucket_k);
                for (var s = 0u; s < n; s = s + 1u) {
                    let j = grid_bucket[h * P.bucket_k + s];
                    if (j == i) { continue; }
                    acc = acc + contact_accel(pi, vi, particles[j].offset, particles[j].vel);
                }
            }
        }
    }
    forces[i] = acc;
}

// --- pass 4: integrate (gravity + contact + drag + terrain + cooling) -----------------------------
@compute @workgroup_size(64)
fn cs_integrate(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= P.count) { return; }
    var pt = particles[i];

    // Cool toward ambient (Newton's law of cooling). EVERY particle steps every frame — a settled
    // grain is not skipped, so it keeps cooling AND re-checks its support / neighbours.
    pt.temp = 300.0 + (pt.temp - 300.0) * exp(-P.cool_rate * P.dt);

    // Uniform planetary surface gravity + the contact acceleration from neighbouring grains.
    let a = P.gravity + forces[i];
    var vel = (pt.vel + a * P.dt) * P.drag;
    var pos = pt.offset + vel * P.dt;

    // Terrain heightfield collision (the column's solid top). Grains stack on each OTHER via contact;
    // this just stops them sinking into bedrock.
    let cx = i32(floor(pos.x + P.center.x));
    let cz = i32(floor(pos.z + P.center.z));
    var resting = 0.0;
    if (cx >= 0 && cz >= 0 && cx < i32(P.world_w) && cz < i32(P.world_d)) {
        let top = heightfield[u32(cz) * P.world_w + u32(cx)];
        let ground_y = f32(top) - P.center.y;
        if (pos.y - P.part_half <= ground_y) {
            pos.y = ground_y + P.part_half;
            vel = vel * P.contact_damp;
            if (length(vel) < P.settle_speed) { resting = 1.0; }
        }
    }

    // Static friction (approximation): a grain that is RESTING ON something (net contact force points
    // up — it is being held against gravity) and moving below the settle speed sticks. Real static
    // friction holds until shear exceeds μ·N, so a resting grain does not creep — and a pile holds its
    // angle of repose instead of flattening under the velocity-only (kinetic) friction. Requiring
    // UPWARD support (not merely "in contact") avoids freezing a grain mid-arc that just grazes
    // another. Proper static friction needs stateful per-contact tangential springs; this threshold
    // form is the honest GPU approximation, flagged (docs/23).
    if (forces[i].y > 1.0 && length(vel) < P.settle_speed) {
        vel = vec3<f32>(0.0);
        pos = pt.offset; // don't creep this substep either
    }

    pt.offset = pos;
    pt.vel = vel;
    pt.resting = resting;
    pt.emission = incandescence(pt.temp);
    particles[i] = pt;
}
