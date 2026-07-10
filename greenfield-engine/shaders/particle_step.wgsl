// GPU particle step (docs/22) — one compute invocation per debris particle. A direct port of the CPU
// `matter::step` hot loop: gravity (centre-of-mass approximation) + semi-implicit Euler + drag +
// collision against the terrain heightfield, plus the incandescent glow from temperature. Particles
// live in a storage buffer that is ALSO the render instance buffer (zero-copy sim↔render).

struct Params {
    gravity      : vec3<f32>, // uniform planetary surface gravity (m/s²), e.g. (0,-9.81,0) — docs/22
    dt           : f32,
    center       : vec3<f32>, // world.center() offset (centered→voxel coords)
    _c           : f32,
    drag         : f32,
    contact_damp : f32,
    settle_speed : f32,
    part_half    : f32,
    cool_rate    : f32, // 1/s — hot debris cools toward ambient (radiative/conductive), docs/20
    count        : u32,
    world_w      : u32,
    world_d      : u32,
};

// One particle. Laid out so the renderer reads `offset` (loc4), `color` (loc5), `emission` (loc6)
// straight out of it — 64 bytes, four 16-byte rows.
struct Particle {
    offset   : vec3<f32>, // = position in centered world coords (also the render instance offset)
    temp     : f32,       // K
    vel      : vec3<f32>,
    resting  : f32,       // 0 = in flight, 1 = settled
    color    : vec3<f32>, // material albedo (set on spawn)
    material : f32,       // material index (informational)
    emission : vec3<f32>, // incandescent glow (written here from temp)
    _pad     : f32,
};

@group(0) @binding(0) var<uniform> P : Params;
@group(0) @binding(1) var<storage, read_write> particles : array<Particle>;
@group(0) @binding(2) var<storage, read> heightfield : array<i32>; // per-column top voxel Y (−1 = empty)

// Black-body incandescence — a WGSL port of `emission::incandescence` (kept in sync, docs/20).
fn incandescence(t : f32) -> vec3<f32> {
    if (t <= 800.0) {
        return vec3<f32>(0.0);
    }
    let x = t - 800.0;
    let intensity = clamp(x / 2200.0, 0.0, 4.0);
    let gg = clamp(x / 2200.0, 0.0, 1.0);
    let bb = clamp((t - 2600.0) / 2400.0, 0.0, 1.0);
    return vec3<f32>(intensity, gg * intensity, bb * intensity);
}

@compute @workgroup_size(64)
fn cs_step(@builtin(global_invocation_id) gid : vec3<u32>) {
    let i = gid.x;
    if (i >= P.count) {
        return;
    }
    var pt = particles[i];

    // Cool toward ambient (Newton's law of cooling): hot debris radiates/conducts and its glow fades
    // (docs/20). EVERY particle steps every frame — a settled particle is NOT skipped, so it keeps
    // cooling AND re-checks its support (if the ground under it is dug away, it falls again). Freezing
    // "resting" particles was a bug (they never cooled, never fell).
    pt.temp = 300.0 + (pt.temp - 300.0) * exp(-P.cool_rate * P.dt);

    // Uniform planetary surface gravity — the slab is a patch of a planet, so it feels the planet's
    // ~uniform field (down), not its own micro-g self-gravity (which pulled all debris toward the world
    // centre — docs/22). Semi-implicit Euler + mild drag.
    var vel = (pt.vel + P.gravity * P.dt) * P.drag;
    var pos = pt.offset + vel * P.dt;

    // Collision against the terrain heightfield (the column's air-start Y).
    let cx = i32(floor(pos.x + P.center.x));
    let cz = i32(floor(pos.z + P.center.z));
    var resting = 0.0;
    if (cx >= 0 && cz >= 0 && cx < i32(P.world_w) && cz < i32(P.world_d)) {
        let top = heightfield[u32(cz) * P.world_w + u32(cx)];
        let ground_y = f32(top) - P.center.y;
        if (pos.y - P.part_half <= ground_y) {
            pos.y = ground_y + P.part_half;
            vel = vel * P.contact_damp;
            if (length(vel) < P.settle_speed) {
                resting = 1.0;
            }
        }
    }

    pt.offset = pos;
    pt.vel = vel;
    pt.resting = resting;
    pt.emission = incandescence(pt.temp);
    particles[i] = pt;
}
