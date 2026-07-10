//! Granular contact — how loose grains push apart, stack, and flow (`docs/23`).
//!
//! Debris particles were resting on the terrain heightfield but ignoring *each other*: they could not
//! stack (everything piled into one layer — the "moiré") and could not slide down a crater wall to a
//! natural slope (a ring stranded on the rim). Both are the same missing physics: **particle-particle
//! contact**. This is the standard discrete-element (DEM) linear contact model — a soft repulsive
//! spring along the contact normal, damping that removes bounce, and **Coulomb friction** on the
//! tangential slip. Friction is the important part: it is what gives a pile its **angle of repose**,
//! and it is the same mechanism behind emergent static-vs-kinetic friction (`docs/23`) — grains at
//! rest settle and resist shear; grains sliding never settle. Nothing here is tuned per-object.
//!
//! The force is returned as an **acceleration** (the 1/m folded into the stiffness/damping constants):
//! the GPU step has no per-particle mass, so we model all debris as equal-mass grains — a documented
//! approximation, honest as long as it is flagged. Per-material mass is a later refinement.
//!
//! This module is the *physics of record*, verified natively; `shaders/particle_step.wgsl` mirrors
//! `contact_accel` exactly on the GPU (kept in sync by construction).

use glam::DVec3;

/// Contact parameters for equal-radius grains. All "stiffness/damping" are per-mass (accelerations),
/// so the model is mass-agnostic (see the module note).
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    /// Grain radius (m): two grains touch when their centres are closer than `2·radius`.
    pub radius: f64,
    /// Normal repulsion (1/s²): the penalty acceleration per metre of overlap. Stiffer ⇒ less
    /// interpenetration but a smaller stable timestep (ω = √stiffness).
    pub stiffness: f64,
    /// Normal damping (1/s): removes bounce by resisting the *approach* velocity. Contacts only ever
    /// push (never pull), so damping cannot turn into an attractive force.
    pub normal_damp: f64,
    /// Coulomb friction coefficient μ: the tangential force is capped at `μ · normal`. This cap is
    /// what produces the angle of repose (grains stop sliding once the slope shallows past ~atan μ).
    pub friction: f64,
    /// Tangential regularization (1/s): how sharply the friction force ramps with slip speed before it
    /// saturates at the Coulomb cap. Avoids a discontinuity at zero slip.
    pub tangent_damp: f64,
}

/// Acceleration on grain *i* due to contact with grain *j* (equal radii). Zero unless they overlap.
/// Symmetric: grain *j* receives the negation from its own evaluation, so momentum is conserved.
#[inline]
pub fn contact_accel(pi: DVec3, vi: DVec3, pj: DVec3, vj: DVec3, c: &Contact) -> DVec3 {
    let d = pi - pj;
    let dist = d.length();
    let touch = 2.0 * c.radius;
    if dist >= touch || dist < 1.0e-9 {
        return DVec3::ZERO; // not in contact (or coincident — no defined normal)
    }
    let n = d / dist; // unit normal, from j toward i
    let overlap = touch - dist;
    let v_rel = vi - vj;
    let v_n = v_rel.dot(n); // >0 separating, <0 approaching

    // Normal: repulsion minus damping of the approach. Clamp to ≥0 so a contact can only push apart,
    // never suck together (an over-damped separating contact would otherwise pull).
    let a_n_mag = (c.stiffness * overlap - c.normal_damp * v_n).max(0.0);
    let a_n = n * a_n_mag;

    // Tangential (Coulomb friction): oppose the slip, but never exceed μ·normal. Regularized so it
    // ramps smoothly from zero slip instead of chattering.
    let v_t = v_rel - n * v_n;
    let vt_mag = v_t.length();
    let a_t = if vt_mag > 1.0e-9 {
        let mag = (c.tangent_damp * vt_mag).min(c.friction * a_n_mag);
        -(v_t / vt_mag) * mag
    } else {
        DVec3::ZERO
    };

    a_n + a_t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Contact {
        Contact {
            radius: 0.5,
            stiffness: 2.0e4,
            normal_damp: 140.0,
            friction: 0.6,
            tangent_damp: 200.0,
        }
    }

    #[test]
    fn separated_grains_do_not_interact() {
        let c = params();
        // 2.0 apart, touch distance is 1.0 → no contact.
        let a = contact_accel(
            DVec3::new(2.0, 0.0, 0.0),
            DVec3::ZERO,
            DVec3::ZERO,
            DVec3::ZERO,
            &c,
        );
        assert_eq!(a, DVec3::ZERO);
    }

    #[test]
    fn overlapping_grains_push_apart_along_the_normal() {
        let c = params();
        // Centres 0.8 apart (touch 1.0) → overlap 0.2, along +x. At rest ⇒ pure repulsion.
        let a = contact_accel(
            DVec3::new(0.8, 0.0, 0.0),
            DVec3::ZERO,
            DVec3::ZERO,
            DVec3::ZERO,
            &c,
        );
        assert!(
            a.x > 0.0 && a.y.abs() < 1e-9 && a.z.abs() < 1e-9,
            "along +x"
        );
        assert!(
            (a.x - c.stiffness * 0.2).abs() < 1e-6,
            "repulsion = stiffness·overlap (got {})",
            a.x
        );
    }

    #[test]
    fn contacts_only_push_never_pull() {
        let c = params();
        // Overlapping but separating fast: damping term is large, but the force must not go negative
        // (a contact cannot pull two grains together).
        let a = contact_accel(
            DVec3::new(0.99, 0.0, 0.0),
            DVec3::new(100.0, 0.0, 0.0), // flying apart
            DVec3::ZERO,
            DVec3::ZERO,
            &c,
        );
        assert!(a.x >= 0.0, "never attractive (got {})", a.x);
    }

    #[test]
    fn approaching_grains_feel_extra_damping() {
        let c = params();
        let at_rest = contact_accel(
            DVec3::new(0.8, 0.0, 0.0),
            DVec3::ZERO,
            DVec3::ZERO,
            DVec3::ZERO,
            &c,
        );
        let approaching = contact_accel(
            DVec3::new(0.8, 0.0, 0.0),
            DVec3::new(-1.0, 0.0, 0.0), // moving toward j
            DVec3::ZERO,
            DVec3::ZERO,
            &c,
        );
        assert!(
            approaching.x > at_rest.x,
            "approach adds damping resistance ({} vs {})",
            approaching.x,
            at_rest.x
        );
    }

    #[test]
    fn friction_opposes_slip_and_is_capped_by_the_normal_force() {
        let c = params();
        // Overlap along x; slip along z. Friction should be along −z and never exceed μ·normal.
        let a = contact_accel(
            DVec3::new(0.8, 0.0, 0.0),
            DVec3::new(0.0, 0.0, 5.0), // sliding in +z
            DVec3::ZERO,
            DVec3::ZERO,
            &c,
        );
        assert!(a.z < 0.0, "friction opposes the slip");
        let normal = c.stiffness * 0.2;
        assert!(
            a.z.abs() <= c.friction * normal + 1e-6,
            "friction capped at μ·normal ({} vs {})",
            a.z.abs(),
            c.friction * normal
        );
    }

    /// Integration check: a grain dropped onto a grain already resting on the floor comes to rest
    /// *stacked above it* — contact both stops interpenetration and holds the weight. This is the
    /// behaviour that was missing (debris could not stack).
    #[test]
    fn a_dropped_grain_settles_stacked_on_another() {
        let c = params();
        let g = -9.81;
        let floor = 0.0; // grains rest with centre at floor + radius
        let rest_y = floor + c.radius;
        // j sits on the floor; i starts a bit above, will fall and stack.
        let mut pi = DVec3::new(0.0, rest_y + 3.0, 0.0);
        let mut vi = DVec3::ZERO;
        let mut pj = DVec3::new(0.0, rest_y, 0.0);
        let mut vj = DVec3::ZERO;
        let dt = 1.0 / 3000.0; // fine step for the stiff contact
        for _ in 0..30_000 {
            let ai = DVec3::new(0.0, g, 0.0) + contact_accel(pi, vi, pj, vj, &c);
            let aj = DVec3::new(0.0, g, 0.0) + contact_accel(pj, vj, pi, vi, &c);
            vi += ai * dt;
            vj += aj * dt;
            pi += vi * dt;
            pj += vj * dt;
            // Floor: neither grain passes through it.
            for (p, v) in [(&mut pi, &mut vi), (&mut pj, &mut vj)] {
                if p.y < rest_y {
                    p.y = rest_y;
                    if v.y < 0.0 {
                        v.y = 0.0;
                    }
                }
            }
        }
        // i ends up resting ~one diameter above the floor, on top of j — not interpenetrating, not
        // passed through.
        let gap = pi.y - pj.y;
        assert!(
            gap > 1.5 * c.radius && gap < 2.1 * c.radius,
            "i is stacked ~one diameter above j (gap {gap:.3}, diameter {:.3})",
            2.0 * c.radius
        );
        assert!(pj.y < rest_y + 0.05, "j stays on the floor (y {:.3})", pj.y);
        assert!(
            vi.length() < 0.1 && vj.length() < 0.1,
            "both settle to rest"
        );
    }
}
