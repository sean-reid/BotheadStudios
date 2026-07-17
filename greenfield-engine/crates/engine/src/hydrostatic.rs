//! Self-gravitating condensed-matter body in hydrostatic equilibrium (docs/33 stage 2). This is the proof
//! that a planet can be REAL MATTER — a cloud of particles that holds itself up under its own gravity via
//! its equation-of-state pressure — instead of the rigid analytic boundary the impact scene uses today
//! (docs/28 root cause #1). It is the "merge" the architecture map identified (docs/32 §3): it COMPOSES the
//! shared kernels rather than forking them —
//!   • pressure: [`crate::eos::Tillotson`] `P(ρ, u)` (docs/33 stage 1),
//!   • SPH: the ONE cubic-spline kernel [`crate::atmosphere::sph_w`]/[`sph_dw`],
//!   • self-gravity: [`crate::bhtree::BarnesHut`] (O(N log N)),
//! exactly the pieces `atmosphere::AirField` (verified 3D hydrostatic balance) and `aggregate::Aggregate`
//! already use. The only new physics is the condensed EOS closure; everything else is reused. At the
//! unification stage (docs/33 stage 5) this capability folds INTO `Aggregate` so a planet and its debris are
//! one particle system — here it is a focused, independently-verifiable module to get the physics right
//! first (the correctness-first discipline).
//!
//! **Method.** Fill a sphere with equal-mass particles at uniform number density (so the initial mass
//! density is ≈ the material's ρ₀). Each particle carries a specific internal energy `u = c·T` (isothermal
//! in this stage — the settled state is an *isothermal* hydrostatic equilibrium, the same class
//! `atmosphere.rs` verifies for an air column). Relax under self-gravity + SPH-EOS pressure with light
//! velocity damping until it settles. Then VERIFY the settled field satisfies hydrostatic equilibrium
//! pointwise: `dP/dr = −ρ(r)·g(r)`, with `g(r) = G·M(<r)/r²` from the enclosed particle mass — the same
//! balance test, now for a self-gravitating body.

use crate::eos::Tillotson;
use glam::DVec3;

/// A deterministic near-uniform direction on the unit sphere (Fibonacci sphere) — no RNG (which the sim
/// forbids), reproducible across runs.
fn fib_dir(i: usize, n: usize) -> DVec3 {
    let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
    let y = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
    let r = (1.0 - y * y).max(0.0).sqrt();
    let theta = golden * i as f64;
    DVec3::new(theta.cos() * r, y, theta.sin() * r)
}

/// A self-gravitating condensed-matter body: particles + the EOS + SPH smoothing length.
pub struct HydroBody {
    pub pos: Vec<DVec3>,
    pub vel: Vec<DVec3>,
    pub mass: Vec<f64>,
    /// Specific internal energy per particle (J/kg). Fixed in stage 2 (isothermal relaxation).
    pub u: Vec<f64>,
    pub eos: Tillotson,
    /// SPH smoothing length (m).
    pub h: f64,
    /// Gravitational softening (m) — set at half the particle spacing so gravity is honest to touching.
    pub softening: f64,
    /// Cached SPH density from the last [`Self::compute_density`] (kg/m³).
    pub rho: Vec<f64>,
}

impl HydroBody {
    /// Build a single-material sphere of `n` equal-mass particles totalling `total_mass`, uniformly
    /// distributed in volume in a sphere of the material's reference radius R₀=(3M/4πρ₀)^⅓ (so the initial
    /// density is ≈ ρ₀), each at temperature `temp_k` → `u = c·temp_k`.
    pub fn new_sphere(
        eos: Tillotson,
        total_mass: f64,
        temp_k: f64,
        specific_heat: f64,
        n: usize,
    ) -> Self {
        let r0 = (3.0 * total_mass / (4.0 * std::f64::consts::PI * eos.rho0)).cbrt();
        let m_i = total_mass / n as f64;
        let u_i = specific_heat * temp_k;
        let mut pos = Vec::with_capacity(n);
        for i in 0..n {
            // r = R₀·((i+0.5)/n)^⅓ fills the volume uniformly (equal volume per particle ⇒ uniform number
            // density ⇒ uniform mass density at ρ₀). Direction from the Fibonacci sphere.
            let rr = r0 * ((i as f64 + 0.5) / n as f64).cbrt();
            pos.push(fib_dir(i, n) * rr);
        }
        // Mean particle spacing ≈ (V/N)^⅓; SPH smoothing length a few spacings so each particle has enough
        // neighbours for a smooth density (≈ 30–50). Softening half a spacing.
        let spacing = (4.0 / 3.0 * std::f64::consts::PI * r0 * r0 * r0 / n as f64).cbrt();
        let h = 2.6 * spacing;
        HydroBody {
            vel: vec![DVec3::ZERO; n],
            mass: vec![m_i; n],
            u: vec![u_i; n],
            eos,
            h,
            softening: 0.5 * spacing,
            rho: vec![eos.rho0; n],
            pos,
        }
    }

    /// SPH density estimate ρ_i = Σ_j m_j W(|r_i−r_j|, h), including the self term. Cached in `self.rho`.
    pub fn compute_density(&mut self) {
        let n = self.pos.len();
        let grid = crate::neighbors::NeighborGrid::build(&self.pos, self.h);
        let w0 = crate::atmosphere::sph_w(0.0, self.h);
        let mut rho = vec![0.0f64; n];
        for i in 0..n {
            rho[i] = self.mass[i] * w0;
        }
        grid.for_each_pair(&self.pos, |i, j| {
            let r = (self.pos[i] - self.pos[j]).length();
            if r < self.h {
                let w = crate::atmosphere::sph_w(r, self.h);
                rho[i] += self.mass[j] * w;
                rho[j] += self.mass[i] * w;
            }
        });
        self.rho = rho;
    }

    /// Per-particle acceleration: Barnes–Hut self-gravity + the symmetric SPH-EOS pressure force
    /// `a_i = −Σ_j m_j (P_i/ρ_i² + P_j/ρ_j²) ∇W`, with `P = Tillotson(ρ, u)`. Assumes `compute_density`
    /// ran this step.
    pub fn accelerations(&self) -> Vec<DVec3> {
        let n = self.pos.len();
        // Self-gravity (the long-range partner) — the same tree Aggregate uses.
        let bh = crate::bhtree::BarnesHut::build(&self.pos, &self.mass, 0.5, self.softening);
        let mut acc = bh.accelerations(&self.pos, &self.mass);
        // Pressure (the short-range partner) over the same neighbour grid.
        let p: Vec<f64> = (0..n).map(|i| self.eos.pressure(self.rho[i], self.u[i])).collect();
        let grid = crate::neighbors::NeighborGrid::build(&self.pos, self.h);
        grid.for_each_pair(&self.pos, |i, j| {
            let dv = self.pos[i] - self.pos[j];
            let r = dv.length();
            if r >= self.h || r < 1.0e-9 {
                return;
            }
            let term = p[i] / (self.rho[i] * self.rho[i]) + p[j] / (self.rho[j] * self.rho[j]);
            let grad = (dv / r) * crate::atmosphere::sph_dw(r, self.h); // ∇W points from j→i, dW<0
            acc[i] += grad * (-term * self.mass[j]);
            acc[j] += grad * (term * self.mass[i]);
        });
        acc
    }

    /// One damped relaxation step (settle toward equilibrium): recompute density, then
    /// `v = (v + a·dt)·damp; x += v·dt`. Damping is numerical — the equilibrium (dP/dr=−ρg) is the physics,
    /// exactly as in `atmosphere::AirField::relax_step`.
    pub fn relax_step(&mut self, dt: f64, damp: f64) {
        self.compute_density();
        let acc = self.accelerations();
        for i in 0..self.pos.len() {
            self.vel[i] = (self.vel[i] + acc[i] * dt) * damp;
            self.pos[i] += self.vel[i] * dt;
        }
    }

    /// A CFL-safe relaxation timestep from the EOS sound speed and the smoothing length: dt = cfl·h/c.
    pub fn relax_dt(&self, cfl: f64) -> f64 {
        let c = self.eos.sound_speed_sq(self.eos.rho0, self.u.first().copied().unwrap_or(1.0)).sqrt();
        cfl * self.h / c.max(1.0)
    }

    pub fn com(&self) -> DVec3 {
        let m: f64 = self.mass.iter().sum();
        self.pos.iter().zip(&self.mass).map(|(p, &mi)| *p * mi).sum::<DVec3>() / m.max(1e-30)
    }

    /// Mass-weighted RMS radius about the COM — the stability yardstick (a settled body holds it steady).
    pub fn rms_radius(&self) -> f64 {
        let c = self.com();
        let m: f64 = self.mass.iter().sum();
        let s: f64 = self.pos.iter().zip(&self.mass).map(|(p, &mi)| mi * (*p - c).length_squared()).sum();
        (s / m.max(1e-30)).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orbit::G;

    /// Enclosed particle mass within radius `r` of the COM (for the analytic g(r)=G·M(<r)/r²).
    fn enclosed_mass(b: &HydroBody, c: DVec3, r: f64) -> f64 {
        b.pos.iter().zip(&b.mass).filter(|(p, _)| (**p - c).length() <= r).map(|(_, &m)| m).sum()
    }

    #[test]
    #[ignore = "self-gravitating relaxation (~thousands of steps) — run with --ignored"]
    fn a_self_gravitating_eos_body_settles_into_hydrostatic_balance() {
        // docs/33 stage 2: the keystone. A single-material rocky body, built as equal-mass particles at ρ₀,
        // must relax under self-gravity + Tillotson-EOS pressure into a STABLE hydrostatic equilibrium whose
        // settled field satisfies dP/dr = −ρ(r)·g(r) pointwise. This proves a planet can be real matter that
        // holds itself up — the prerequisite for dissolving the rigid boundary (docs/28 #1, docs/31).
        let eos = Tillotson::basalt();
        // A ~1500 km rocky body: central pressure ~ few GPa (mild, well-behaved compression, μ~0.1), N in
        // the tree regime. Total mass = (4/3)π R₀³ ρ₀ at R₀=1500 km.
        let r0 = 1.5e6;
        let total_mass = 4.0 / 3.0 * std::f64::consts::PI * r0 * r0 * r0 * eos.rho0;
        let mut b = HydroBody::new_sphere(eos, total_mass, 300.0, 840.0, 3000);

        let dt = b.relax_dt(0.2);
        // Relax to equilibrium, tracking RMS radius to confirm it settles (not collapsing/exploding).
        let mut rms_hist = Vec::new();
        for step in 0..4000 {
            b.relax_step(dt, 0.96);
            if step % 200 == 0 {
                rms_hist.push(b.rms_radius());
            }
        }
        b.compute_density();

        // (1) STABILITY: the RMS radius over the last quarter of the run is steady (settled), and the body
        // neither collapsed to a point nor unbound itself.
        let last = &rms_hist[rms_hist.len().saturating_sub(4)..];
        let mean: f64 = last.iter().sum::<f64>() / last.len() as f64;
        let spread = last.iter().map(|r| (r - mean).abs()).fold(0.0, f64::max) / mean;
        println!("settled RMS radius {:.0} km (spread {:.1}% over last steps)", mean / 1e3, spread * 100.0);
        assert!(spread < 0.05, "body must settle to a steady RMS radius (spread {:.2})", spread);
        assert!(mean > 0.3 * r0 && mean < 1.2 * r0, "RMS radius {mean:.3e} sane vs R₀={r0:.3e}");

        // (2) HYDROSTATIC BALANCE, pointwise, in the settled field. Bin by radius; at interior quantiles
        // (off the free surface, where the SPH density deficit dominates), the pressure gradient must carry
        // the weight: dP/dr ≈ −ρ·g. Measure at two interior radii via finite differences of shell-mean P.
        let c = b.com();
        let mut idx: Vec<usize> = (0..b.pos.len()).collect();
        idx.sort_by(|&a, &bb| {
            (b.pos[a] - c).length().partial_cmp(&(b.pos[bb] - c).length()).unwrap()
        });
        // Shell-mean P and ρ at a target radius (mean over particles in [r-Δ, r+Δ]).
        let shell = |r: f64, dr: f64| -> (f64, f64, usize) {
            let (mut sp, mut sr, mut cnt) = (0.0, 0.0, 0usize);
            for &i in &idx {
                let ri = (b.pos[i] - c).length();
                if (ri - r).abs() <= dr {
                    sp += b.eos.pressure(b.rho[i], b.u[i]);
                    sr += b.rho[i];
                    cnt += 1;
                }
            }
            if cnt == 0 { (0.0, 0.0, 0) } else { (sp / cnt as f64, sr / cnt as f64, cnt) }
        };
        let settled_r = mean; // ~ the surface radius
        let dr = 0.12 * settled_r;
        let mut checked = 0;
        for frac in [0.35_f64, 0.55] {
            let r = frac * settled_r;
            let (p_lo, _, n_lo) = shell(r - dr, dr);
            let (p_hi, _, n_hi) = shell(r + dr, dr);
            let (_, rho_mid, n_mid) = shell(r, dr);
            if n_lo < 20 || n_hi < 20 || n_mid < 20 {
                continue;
            }
            let dpdr = (p_hi - p_lo) / (2.0 * dr);
            let g = G * enclosed_mass(&b, c, r) / (r * r);
            let expect = -rho_mid * g; // hydrostatic equilibrium
            let rel = (dpdr - expect).abs() / expect.abs().max(1.0);
            println!(
                "hydrostatic @ r={:.0} km: dP/dr {:.3e} vs −ρg {:.3e} (rel {:.2}), P_shell {:.3e} Pa",
                r / 1e3, dpdr, expect, rel, shell(r, dr).0
            );
            // Operator/finite-N tolerance (cf. atmosphere.rs's 3D balance at ~35%): the gradient must
            // carry the weight to within SPH truncation + shell-binning error, and have the right SIGN.
            assert!(dpdr < 0.0, "pressure must DECREASE outward at r={r:.3e}");
            assert!(rel < 0.5, "hydrostatic balance within operator error at r={r:.3e} (rel {rel:.2})");
            checked += 1;
        }
        assert!(checked >= 1, "at least one interior shell must have enough particles to test balance");

        // (3) CENTRAL PRESSURE of the right ORDER: compare the core shell pressure to the uniform-density
        // analytic estimate P_c ≈ (3/8π)·G·M²/R⁴ — same order (factor ~few), confirming it's a real planet
        // pressure, not a numerical artefact.
        let (p_core, _, _) = shell(0.1 * settled_r, 0.1 * settled_r);
        let p_analytic = 3.0 / (8.0 * std::f64::consts::PI) * G * total_mass * total_mass / settled_r.powi(4);
        println!("central P {:.3e} Pa vs uniform-density estimate {:.3e} Pa", p_core, p_analytic);
        assert!(
            p_core > 0.2 * p_analytic && p_core < 5.0 * p_analytic,
            "central pressure {p_core:.2e} must be the order of the self-gravity estimate {p_analytic:.2e}"
        );
    }
}
