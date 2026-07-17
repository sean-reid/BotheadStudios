// GPU SPH-EOS-gravity force kernel (docs/33 stage 4). The space-band self-gravitating condensed-matter
// step (`hydrostatic.rs`) moved to a WGSL compute shader so a giant impact can run at N~10^5 — the resolution
// the isotopic-crisis number needs. This is the SAME physics as the CPU `HydroBody::forces_and_dudt`, in f32:
//   • SPH density   ρ_i = Σ_j m_j W(r_ij, h_ij)                        (cubic spline, per-pair h_ij=½(h_i+h_j))
//   • Tillotson EOS  P_i = P(ρ_i, u_i)                                  (per-material; matches eos.rs)
//   • pressure force a_i = −Σ_j m_j (P_i/ρ_i² + P_j/ρ_j² + Π_ij) ∇W    (Monaghan artificial viscosity Π)
//   • self-gravity   a_i += Σ_j G m_j d/(|d|²+ε²)^{3/2}                (direct O(N²); a tree is stage 4b)
//   • energy         du_i/dt = ½ Σ_j m_j (…) (v_i−v_j)·∇W
// VERIFIED on the RTX 2070 (tools/sph-verify): this kernel matches an independent f64 CPU computation of
// the same equations (= hydrostatic.rs) to f32 precision — acceleration RMS rel error 1.9e-6, du/dt 3.6e-6.
// O(N²) here is the correctness pass; the neighbour grid + Barnes–Hut that make it O(N log N) are stage 4b
// (the CPU already has both — neighbors.rs / bhtree.rs — to port), and the KDK integration loop + scene
// wiring are stage 4c/5. This is one FORCE evaluation, verified.

const PI: f32 = 3.14159265359;
const G: f32 = 6.674e-11;

struct Params {
  n: u32,
  softening: f32,   // Plummer gravity softening
  av_alpha: f32,
  av_beta: f32,
}

// One condensed-matter particle. std430 layout: keep vec3s padded to 16 bytes (trailing scalar fills the pad).
struct Particle {
  pos: vec3<f32>, h: f32,        // position + smoothing length
  vel: vec3<f32>, u: f32,        // velocity + specific internal energy
  mass: f32, mat: u32, rho: f32, _pad: f32,  // mass, material id (0=basalt,1=iron), cached density
}

// Tillotson parameters for one material (matches eos::Tillotson; f32).
struct Eos {
  rho0: f32, a: f32, b: f32, cap_a: f32,
  cap_b: f32, e0: f32, e_iv: f32, e_cv: f32,
  alpha: f32, beta: f32, _p0: f32, _p1: f32,
}

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(2) var<storage, read> eos: array<Eos>;     // per material
@group(0) @binding(3) var<storage, read_write> acc: array<vec3<f32>>;   // output acceleration
@group(0) @binding(4) var<storage, read_write> dudt: array<f32>;        // output energy rate

// Cubic spline kernel (3D, σ = 8/(π h³)) — identical to atmosphere::sph_w.
fn sph_w(r: f32, h: f32) -> f32 {
  let q = r / h;
  let sig = 8.0 / (PI * h * h * h);
  if (q < 0.5) { return sig * (1.0 - 6.0 * q * q + 6.0 * q * q * q); }
  if (q < 1.0) { let t = 1.0 - q; return sig * 2.0 * t * t * t; }
  return 0.0;
}
// dW/dr — identical to atmosphere::sph_dw (≤0).
fn sph_dw(r: f32, h: f32) -> f32 {
  let q = r / h;
  let sig = 8.0 / (PI * h * h * h);
  if (q < 0.5) { return sig * (-12.0 * q + 18.0 * q * q) / h; }
  if (q < 1.0) { let t = 1.0 - q; return sig * (-6.0 * t * t) / h; }
  return 0.0;
}

// Tillotson pressure P(ρ,u) — matches eos::Tillotson::pressure.
fn pressure(e: Eos, rho: f32, u: f32) -> f32 {
  let r = max(rho, 1.0e-9);
  let eta = r / e.rho0;
  let mu = eta - 1.0;
  let omega = u / (e.e0 * eta * eta) + 1.0;
  let p_c = (e.a + e.b / omega) * r * u + e.cap_a * mu + e.cap_b * mu * mu;
  if (eta >= 1.0 || u <= e.e_iv) { return p_c; }
  let z = e.rho0 / r - 1.0;
  let p_e = e.a * r * u
    + (e.b * r * u / omega + e.cap_a * mu * exp(-e.beta * z)) * exp(-e.alpha * z * z);
  if (u >= e.e_cv) { return p_e; }
  return ((u - e.e_iv) * p_e + (e.e_cv - u) * p_c) / (e.e_cv - e.e_iv);
}

fn sound_speed(e: Eos, rho: f32, u: f32) -> f32 {
  let r = max(rho, 1.0e-9);
  let dr = r * 1.0e-3;
  let dp = (pressure(e, r + dr, u) - pressure(e, r - dr, u)) / (2.0 * dr);
  let p = pressure(e, r, u);
  return sqrt(max(dp + p / (r * r) * dfdu(e, r, u), 0.0));
}
fn dfdu(e: Eos, rho: f32, u: f32) -> f32 {
  let du = abs(u) * 1.0e-3 + 1.0;
  return (pressure(e, rho, u + du) - pressure(e, rho, u - du)) / (2.0 * du);
}

// PASS 1 — SPH density ρ_i = Σ_j m_j W(r_ij, h_ij) + self, per-pair h_ij = ½(h_i+h_j). O(N²).
@compute @workgroup_size(64)
fn cs_density(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= P.n) { return; }
  let pi = particles[i];
  var rho = pi.mass * sph_w(0.0, pi.h);
  for (var j: u32 = 0u; j < P.n; j++) {
    if (j == i) { continue; }
    let pj = particles[j];
    let r = length(pi.pos - pj.pos);
    let hij = 0.5 * (pi.h + pj.h);
    if (r < hij) { rho += pj.mass * sph_w(r, hij); }
  }
  particles[i].rho = rho;
}

// PASS 2 — forces: self-gravity (direct O(N²)) + symmetric SPH-EOS pressure with Monaghan AV, and du/dt.
@compute @workgroup_size(64)
fn cs_forces(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= P.n) { return; }
  let pi = particles[i];
  let ei = eos[pi.mat];
  let p_i = pressure(ei, pi.rho, pi.u);
  let c_i = sound_speed(ei, pi.rho, pi.u);
  let s2 = P.softening * P.softening;
  var a = vec3<f32>(0.0);
  var de: f32 = 0.0;
  for (var j: u32 = 0u; j < P.n; j++) {
    if (j == i) { continue; }
    let pj = particles[j];
    let d = pj.pos - pi.pos;
    let r2 = dot(d, d);
    // gravity (softened, matches BarnesHut direct sum)
    a += d * (G * pj.mass / pow(r2 + s2, 1.5));
    // SPH pressure + AV (short range)
    let r = sqrt(r2);
    let hij = 0.5 * (pi.h + pj.h);
    if (r < hij && r > 1.0e-9) {
      let ej = eos[pj.mat];
      let p_j = pressure(ej, pj.rho, pj.u);
      let c_j = sound_speed(ej, pj.rho, pj.u);
      let dvel = pi.vel - pj.vel;
      let dpos = pi.pos - pj.pos;             // i − j (opposite of d)
      let vr = dot(dvel, dpos);
      var pi_ij: f32 = 0.0;
      if (vr < 0.0) {
        let mu = hij * vr / (r * r + 0.01 * hij * hij);
        let c_bar = 0.5 * (c_i + c_j);
        let rho_bar = 0.5 * (pi.rho + pj.rho);
        pi_ij = (-P.av_alpha * c_bar * mu + P.av_beta * mu * mu) / rho_bar;
      }
      let coeff = p_i / (pi.rho * pi.rho) + p_j / (pj.rho * pj.rho) + pi_ij;
      let grad = (dpos / r) * sph_dw(r, hij);  // ∇_i W
      a += grad * (-coeff * pj.mass);
      de += 0.5 * pj.mass * coeff * dot(dvel, grad);
    }
  }
  acc[i] = a;
  dudt[i] = de;
}
