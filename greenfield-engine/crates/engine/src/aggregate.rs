//! Self-gravitating particle aggregates — a body as a **cloud of particles held together by its own
//! gravity** (a "rubble pile"), rather than a point mass or a rigid sphere (`docs/21`).
//!
//! This is what makes celestial destruction a *simulation, not a mock*: the aggregate's cohesion and
//! (roughly spherical) shape **emerge** from mutual gravity (the representation invariant, `docs/15` —
//! roundness is emergent), and it **disrupts when given more energy than its gravitational binding
//! energy** — the particles simply exceed escape velocity and disperse. Nothing is scripted; a
//! shattered moon is the same N-body gravity that made it round, run past its binding energy.
//!
//! The particles reuse `orbit::Body` (pos, vel, mass). Gravity is **softened** (unlike the clean
//! two-body `orbit.rs`) because a dense cloud has close pairs whose bare 1/r² would explode. Material
//! and temperature per particle (for melt/vaporize glow via `damage`/`emission`) arrive when this is
//! wired into an impact — this module is the gravitational skeleton.

#![allow(dead_code)] // consumed by the space-band integration (staged) and native tests

use crate::orbit::{Body, G};
use glam::DVec3;

pub struct Aggregate {
    pub particles: Vec<Body>,
    /// Softening length (m): removes the 1/r² singularity between close particles. ~half the mean
    /// spacing keeps the cloud stable without erasing its self-gravity.
    pub softening: f64,
}

impl Aggregate {
    pub fn new(particles: Vec<Body>, softening: f64) -> Self {
        Aggregate {
            particles,
            softening,
        }
    }

    /// Softened mutual-gravity acceleration on every particle (N-body).
    pub fn accelerations(&self) -> Vec<DVec3> {
        let s2 = self.softening * self.softening;
        let p = &self.particles;
        let mut acc = vec![DVec3::ZERO; p.len()];
        for i in 0..p.len() {
            for j in 0..p.len() {
                if i == j {
                    continue;
                }
                let d = p[j].pos - p[i].pos;
                let r2 = d.length_squared() + s2;
                acc[i] += d * (G * p[j].mass * r2.powf(-1.5));
            }
        }
        acc
    }

    /// One velocity-Verlet step (symplectic; conserves energy over many dynamical times). Pass the
    /// same `acc` buffer each step, seeded with `accelerations()`.
    pub fn step(&mut self, acc: &mut Vec<DVec3>, dt: f64) {
        for (b, a) in self.particles.iter_mut().zip(acc.iter()) {
            b.vel += *a * (0.5 * dt);
            b.pos += b.vel * dt;
        }
        let new_acc = self.accelerations();
        for (b, a) in self.particles.iter_mut().zip(new_acc.iter()) {
            b.vel += *a * (0.5 * dt);
        }
        *acc = new_acc;
    }

    pub fn total_mass(&self) -> f64 {
        self.particles.iter().map(|b| b.mass).sum()
    }

    /// Center of mass.
    pub fn com(&self) -> DVec3 {
        let m = self.total_mass();
        if m <= 0.0 {
            return DVec3::ZERO;
        }
        self.particles
            .iter()
            .fold(DVec3::ZERO, |s, b| s + b.pos * b.mass)
            / m
    }

    /// Mass-weighted RMS radius about the COM — the cloud's *spread*. Bounded while the aggregate
    /// holds together; grows without limit once it disperses.
    pub fn rms_radius(&self) -> f64 {
        let m = self.total_mass();
        if m <= 0.0 {
            return 0.0;
        }
        let c = self.com();
        let s: f64 = self
            .particles
            .iter()
            .map(|b| b.mass * (b.pos - c).length_squared())
            .sum();
        (s / m).sqrt()
    }

    /// Gravitational **binding energy** (J, positive): `Σ_{i<j} G·m_i·m_j / r_ij` (softened) — the
    /// energy needed to disperse the aggregate to infinity. Give it more than this and it comes apart.
    pub fn binding_energy(&self) -> f64 {
        let s2 = self.softening * self.softening;
        let p = &self.particles;
        let mut e = 0.0;
        for i in 0..p.len() {
            for j in (i + 1)..p.len() {
                let r = ((p[j].pos - p[i].pos).length_squared() + s2).sqrt();
                e += G * p[i].mass * p[j].mass / r;
            }
        }
        e
    }

    /// Kinetic energy in the centre-of-mass frame (J) — the "disordered" energy that competes with
    /// binding. `kinetic_com > binding` ⇒ the aggregate flies apart.
    pub fn kinetic_energy_com(&self) -> f64 {
        let vcom = {
            let m = self.total_mass();
            if m <= 0.0 {
                DVec3::ZERO
            } else {
                self.particles
                    .iter()
                    .fold(DVec3::ZERO, |s, b| s + b.vel * b.mass)
                    / m
            }
        };
        self.particles
            .iter()
            .map(|b| 0.5 * b.mass * (b.vel - vcom).length_squared())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small cubic cloud of equal-mass particles, at rest.
    fn cloud(side: i32, spacing: f64, mass: f64) -> Vec<Body> {
        let mut v = Vec::new();
        for x in 0..side {
            for y in 0..side {
                for z in 0..side {
                    v.push(Body {
                        pos: DVec3::new(x as f64, y as f64, z as f64) * spacing,
                        vel: DVec3::ZERO,
                        mass,
                    });
                }
            }
        }
        v
    }

    #[test]
    fn a_self_gravitating_cloud_holds_together() {
        // A cold cloud bound by its own gravity does not fly apart — its spread stays bounded (it
        // collapses/virialises inward, it does not disperse). Cohesion is emergent, not glued.
        let mut agg = Aggregate::new(cloud(3, 100.0, 1.0e13), 50.0);
        let r0 = agg.rms_radius();
        let mut acc = agg.accelerations();
        for _ in 0..400 {
            agg.step(&mut acc, 2.0);
        }
        assert!(
            agg.rms_radius() < 3.0 * r0,
            "self-gravity keeps it bound (rms {:.1} vs r0 {:.1})",
            agg.rms_radius(),
            r0
        );
    }

    #[test]
    fn energy_above_binding_disrupts_it() {
        // Give the same cloud outward kinetic energy exceeding its binding energy and it comes
        // apart — emergent disruption, the identity behind a shattered moon (no scripted explosion).
        let mut agg = Aggregate::new(cloud(3, 100.0, 1.0e13), 50.0);
        let r0 = agg.rms_radius();
        let bind = agg.binding_energy();
        let com = agg.com();
        for b in &mut agg.particles {
            b.vel = (b.pos - com).normalize_or_zero() * 40.0; // outward kick
        }
        assert!(
            agg.kinetic_energy_com() > bind,
            "the kick exceeds binding (KE {:.2e} > bind {:.2e})",
            agg.kinetic_energy_com(),
            bind
        );

        let mut acc = agg.accelerations();
        for _ in 0..400 {
            agg.step(&mut acc, 2.0);
        }
        assert!(
            agg.rms_radius() > 10.0 * r0,
            "it disperses (rms {:.1} vs r0 {:.1})",
            agg.rms_radius(),
            r0
        );
    }
}
