//! Phase 3 matter solver: digging, material-driven fracture, and granular settling.
//!
//! This is the **CPU, natively-tested** foundation for destructible matter. It captures the target
//! behaviors — dig a hole, break chunks off, materials respond differently *by their own strength*,
//! debris falls under real gravity and settles back into the world (matter-conserving) — without the
//! full continuum machinery. It is deliberately structured to grow toward true **MLS-MPM** (add a
//! deformation gradient + constitutive stress, then move the hot loops to WGSL); see `docs/06`/`08`.
//!
//! Fracture is emergent from data: a voxel detaches only if the tool's stress exceeds its material's
//! `fracture_strength` (from `data/materials.json`). Granite (~1.2e7 Pa) shrugs off a tool that
//! shreds soil/grass (~5e3–1.5e4 Pa) — no per-material special-casing, just the numbers.
//!
//! Scale note (same as Phase 2): the test world is asteroid-sized, so its escape velocity is a few
//! cm/s. Ejection speeds are kept sub-escape so debris stays bound and re-settles; that is correct
//! micro-gravity physics, viewed via the time-scale.

use crate::gravity::MassField;
use crate::materials::Material;
use crate::world::World;
use glam::Vec3;

const PARTICLE_HALF: f32 = 0.45; // rendered/collision half-extent (voxel-ish)
const DRAG: f32 = 0.9995; // mild air drag per step
const CONTACT_DAMP: f32 = 0.35; // energy kept after touching ground
const SETTLE_SPEED: f32 = 0.02; // below this, a grounded particle deposits into the grid
const SETTLE_FRAMES: u32 = 10; // ...or after this many consecutive grounded steps
const MAX_EJECT: f32 = 0.045; // cap ejection speed below the world's ~7 cm/s escape velocity

/// A detached lump of matter in flight (one former voxel).
#[derive(Clone, Copy)]
pub struct Particle {
    pub pos: Vec3, // centered world coords
    pub vel: Vec3,
    pub material: usize,
    /// kg. Not read yet (gravity is mass-independent); kept for momentum/collision later.
    #[allow(dead_code)]
    pub mass: f32,
    resting_frames: u32,
}

pub struct MatterSim {
    pub particles: Vec<Particle>,
    max_particles: usize,
    dirty: bool, // a voxel changed → terrain needs re-meshing
}

impl MatterSim {
    pub fn new(max_particles: usize) -> Self {
        MatterSim {
            particles: Vec::new(),
            max_particles,
            dirty: false,
        }
    }

    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    /// Consume the "terrain changed" flag (caller re-meshes when true).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Dig a spherical region at `hit` (centered coords) with a tool of `strength` (Pa). Voxels whose
    /// material `fracture_strength` is at or below the tool strength detach into particles with a
    /// small outward+upward ejection; stronger materials are untouched. Returns the count detached.
    pub fn dig(
        &mut self,
        world: &mut World,
        materials: &[Material],
        hit: Vec3,
        radius: f32,
        strength: f32,
    ) -> usize {
        let center = world.center();
        let hv = hit + center; // voxel-space
        let ri = radius.ceil() as i32;
        let (cx, cy, cz) = (
            hv.x.floor() as i32,
            hv.y.floor() as i32,
            hv.z.floor() as i32,
        );
        let mut spawned = 0;
        for dz in -ri..=ri {
            for dy in -ri..=ri {
                for dx in -ri..=ri {
                    if self.particles.len() >= self.max_particles {
                        break;
                    }
                    let (x, y, z) = (cx + dx, cy + dy, cz + dz);
                    let vc = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    if (vc - hv).length() > radius {
                        continue;
                    }
                    let Some(mat) = world.material_at(x, y, z) else {
                        continue;
                    };
                    if strength < materials[mat].fracture_strength {
                        continue; // too strong to break with this tool
                    }
                    world.set_voxel(x, y, z, None);
                    let pos = vc - center;
                    let outward = (pos - hit).normalize_or_zero();
                    let dir = (outward * 0.7 + Vec3::Y * 0.6).normalize_or_zero();
                    let excess = (strength / materials[mat].fracture_strength).clamp(1.0, 6.0);
                    let speed = (0.02 * excess).min(MAX_EJECT);
                    self.particles.push(Particle {
                        pos,
                        vel: dir * speed,
                        material: mat,
                        mass: materials[mat].density,
                        resting_frames: 0,
                    });
                    spawned += 1;
                }
            }
        }
        if spawned > 0 {
            self.dirty = true;
        }
        spawned
    }

    /// Advance all particles by `dt`: gravity from the field, terrain collision, and — when a
    /// particle comes to rest — deposit it back into the voxel grid (piling; matter-conserving).
    pub fn step(&mut self, world: &mut World, field: &MassField, dt: f32) {
        let center = world.center();
        let bound = world.w.max(world.h).max(world.d) as f32;

        let mut i = 0;
        while i < self.particles.len() {
            let mut p = self.particles[i];
            // Debris uses the cheap COM approximation — O(1) per particle, adequate for many lumps.
            let accel = field.acceleration_point_approx(p.pos, 4.0);
            p.vel += accel * dt;
            p.vel *= DRAG;
            p.pos += p.vel * dt;

            // Drifted off the world entirely → lost (rare; ejection is sub-escape).
            if p.pos.y < -center.y - 20.0
                || p.pos.y > center.y + bound
                || p.pos.x.abs() > bound
                || p.pos.z.abs() > bound
            {
                self.particles.swap_remove(i);
                continue;
            }

            let xi = (p.pos.x + center.x).floor() as i32;
            let zi = (p.pos.z + center.z).floor() as i32;
            let ground_y = world
                .surface_top_voxel(xi, zi)
                .map(|t| t as f32 - center.y)
                .unwrap_or(-center.y - 1.0);

            if p.pos.y - PARTICLE_HALF <= ground_y {
                p.pos.y = ground_y + PARTICLE_HALF;
                p.vel *= CONTACT_DAMP;
                p.resting_frames += 1;
                if p.vel.length() < SETTLE_SPEED || p.resting_frames > SETTLE_FRAMES {
                    // Deposit into the column's air-start voxel (stacks / refills the crater).
                    if let Some(ty) = world.surface_top_voxel(xi, zi) {
                        if (ty as usize) < world.h {
                            world.set_voxel(xi, ty, zi, Some(p.material));
                            self.dirty = true;
                        }
                    }
                    self.particles.swap_remove(i);
                    continue;
                }
            } else {
                p.resting_frames = 0;
            }

            self.particles[i] = p;
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{gravity, materials, world};

    fn center_surface(w: &World) -> f32 {
        let c = w.center();
        w.surface_top_voxel(c.x as i32, c.z as i32).unwrap() as f32 - c.y
    }

    #[test]
    fn dig_detaches_soft_but_not_hard() {
        let mats = materials::load();

        // Soft layers (soil/grass, ~5e3–1.5e4 Pa) detach under a 1e6 Pa tool.
        let mut w1 = world::generate(&mats);
        let surf = center_surface(&w1);
        let mut soft = MatterSim::new(50_000);
        let n_soft = soft.dig(&mut w1, &mats, Vec3::new(0.0, surf - 1.5, 0.0), 3.0, 1.0e6);
        assert!(n_soft > 0, "soil/grass should detach under a 1e6 Pa tool");

        // The same tool deep in granite (1.2e7 Pa) removes nothing.
        let mut w2 = world::generate(&mats);
        let mut hard = MatterSim::new(50_000);
        let n_rock = hard.dig(&mut w2, &mats, Vec3::new(0.0, surf - 30.0, 0.0), 3.0, 1.0e6);
        assert_eq!(
            n_rock, 0,
            "granite resists a tool weaker than its fracture strength"
        );

        // A stronger blast (2e7 Pa) *does* break the rock.
        let mut w3 = world::generate(&mats);
        let mut blast = MatterSim::new(50_000);
        let n_blast = blast.dig(&mut w3, &mats, Vec3::new(0.0, surf - 30.0, 0.0), 3.0, 2.0e7);
        assert!(n_blast > 0, "a strong enough blast breaks granite");
    }

    #[test]
    fn matter_conserved_through_dig_and_settle() {
        let mats = materials::load();
        let mut w = world::generate(&mats);
        let field = gravity::MassField::build(&w, &mats, 4);
        let before = w.solid_count();
        let surf = center_surface(&w);

        let mut sim = MatterSim::new(50_000);
        let n = sim.dig(&mut w, &mats, Vec3::new(0.0, surf - 1.5, 0.0), 3.0, 1.0e6);
        assert!(n > 0);
        // Right after digging: removed n voxels, spawned n particles.
        assert_eq!(w.solid_count() + sim.particle_count(), before);

        // Settle. The invariant (voxels + airborne particles == original) must hold every step.
        let mut settled = false;
        for _ in 0..40_000 {
            sim.step(&mut w, &field, 5.0);
            assert_eq!(
                w.solid_count() + sim.particle_count(),
                before,
                "matter conserved each step"
            );
            if sim.particle_count() == 0 {
                settled = true;
                break;
            }
        }
        assert!(settled, "all debris should eventually settle");
        assert_eq!(
            w.solid_count(),
            before,
            "matter fully conserved after settling"
        );
    }
}
