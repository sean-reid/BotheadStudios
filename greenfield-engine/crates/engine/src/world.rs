//! The voxel matter store and the Phase 1 layered-world generator.
//!
//! Each voxel holds a material index (0 = empty/air, else `material_index + 1`). This is the
//! authoritative "matter store" — later phases attach per-voxel density = material.density (so
//! summed mass drives gravity) and activate voxels into MPM particles under stress. The generator
//! lays a surface patch of the REAL layered Earth — grass skin, basalt crust, peridotite mantle, iron
//! core — as a declared vertical LOD (real materials/order, compressed depths; docs/25/28).

use crate::materials::{index_of, Material};
use glam::{IVec3, Vec3};

/// Width (X), height (Y, up), depth (Z) of the world in voxels. 1 voxel = 1 metre.
pub const W: usize = 96;
pub const H: usize = 56;
pub const D: usize = 96;

const GRASS_THICKNESS: usize = 1; // thin fragile biosphere skin over the crust

pub struct World {
    pub w: usize,
    pub h: usize,
    pub d: usize,
    /// `voxels[idx] == 0` is air; otherwise the material index is `voxels[idx] - 1`.
    pub voxels: Vec<u16>,
    /// Tallest column, for centering the camera on the terrain.
    pub max_top: usize,
}

impl World {
    #[inline]
    pub fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (y * self.d + z) * self.w + x
    }

    /// Material index at a voxel, or `None` for air / out of bounds.
    #[inline]
    pub fn material_at(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        if x < 0
            || y < 0
            || z < 0
            || x as usize >= self.w
            || y as usize >= self.h
            || z as usize >= self.d
        {
            return None;
        }
        let v = self.voxels[self.idx(x as usize, y as usize, z as usize)];
        if v == 0 {
            None
        } else {
            Some((v - 1) as usize)
        }
    }

    #[inline]
    pub fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        self.material_at(x, y, z).is_some()
    }

    /// The offset used to center the world on the origin (shared by the mesher, gravity, and
    /// physics so geometry and forces live in the same coordinate frame).
    pub fn center(&self) -> Vec3 {
        Vec3::new(
            self.w as f32 * 0.5,
            self.max_top as f32 * 0.5,
            self.d as f32 * 0.5,
        )
    }

    /// The Y (in voxel units) where air begins above column `(x, z)` — i.e. the surface top.
    /// `None` if the column is empty or out of bounds.
    pub fn surface_top_voxel(&self, x: i32, z: i32) -> Option<i32> {
        if x < 0 || z < 0 || x as usize >= self.w || z as usize >= self.d {
            return None;
        }
        for y in (0..self.h as i32).rev() {
            if self.is_solid(x, y, z) {
                return Some(y + 1);
            }
        }
        None
    }

    /// Set a voxel's material (`None` = air). Out-of-bounds writes are ignored.
    pub fn set_voxel(&mut self, x: i32, y: i32, z: i32, material: Option<usize>) {
        if x < 0
            || y < 0
            || z < 0
            || x as usize >= self.w
            || y as usize >= self.h
            || z as usize >= self.d
        {
            return;
        }
        let i = self.idx(x as usize, y as usize, z as usize);
        self.voxels[i] = material.map(|m| m as u16 + 1).unwrap_or(0);
    }

    /// Total number of solid voxels — used for matter-conservation checks (tests).
    #[allow(dead_code)]
    pub fn solid_count(&self) -> usize {
        self.voxels.iter().filter(|&&v| v != 0).count()
    }

    /// March a ray (given in centered coordinates) through the grid; return the first solid voxel it
    /// hits and the centered hit position. Amanatides–Woo DDA — used for click-to-dig picking.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<(i32, i32, i32, Vec3)> {
        let d = dir.normalize_or_zero();
        if d == Vec3::ZERO {
            return None;
        }
        let o = origin + self.center(); // ray origin in voxel space

        let mut v = IVec3::new(o.x.floor() as i32, o.y.floor() as i32, o.z.floor() as i32);
        let step = IVec3::new(sign(d.x), sign(d.y), sign(d.z));

        // Parametric distance to the first voxel boundary on each axis, and per-voxel increments.
        let t_max = |oc: f32, dc: f32, s: i32| -> f32 {
            if dc == 0.0 {
                f32::INFINITY
            } else if s > 0 {
                (oc.floor() + 1.0 - oc) / dc
            } else {
                (oc.floor() - oc) / dc
            }
        };
        let mut tmx = t_max(o.x, d.x, step.x);
        let mut tmy = t_max(o.y, d.y, step.y);
        let mut tmz = t_max(o.z, d.z, step.z);
        let tdx = if d.x != 0.0 {
            (1.0 / d.x).abs()
        } else {
            f32::INFINITY
        };
        let tdy = if d.y != 0.0 {
            (1.0 / d.y).abs()
        } else {
            f32::INFINITY
        };
        let tdz = if d.z != 0.0 {
            (1.0 / d.z).abs()
        } else {
            f32::INFINITY
        };

        let mut t = 0.0f32;
        for _ in 0..8192 {
            if self.is_solid(v.x, v.y, v.z) {
                return Some((v.x, v.y, v.z, origin + d * t));
            }
            if tmx <= tmy && tmx <= tmz {
                v.x += step.x;
                t = tmx;
                tmx += tdx;
            } else if tmy <= tmz {
                v.y += step.y;
                t = tmy;
                tmy += tdy;
            } else {
                v.z += step.z;
                t = tmz;
                tmz += tdz;
            }
            if t > max_dist {
                break;
            }
        }
        None
    }

    /// Solid voxels **not** connected (6-connectivity, through solid) to the anchored base (the
    /// `y = 0` layer). These are unsupported and should collapse. A flood-fill from the base marks
    /// everything supported; the rest is returned. O(number of voxels).
    pub fn find_unsupported(&self) -> Vec<(i32, i32, i32)> {
        const NEIGHBORS: [(i32, i32, i32); 6] = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ];
        let mut supported = vec![false; self.w * self.h * self.d];
        let mut stack: Vec<usize> = Vec::new();

        // Seed with every solid voxel in the base layer.
        for z in 0..self.d {
            for x in 0..self.w {
                if self.is_solid(x as i32, 0, z as i32) {
                    let i = self.idx(x, 0, z);
                    if !supported[i] {
                        supported[i] = true;
                        stack.push(i);
                    }
                }
            }
        }

        // Flood-fill through connected solid voxels.
        while let Some(i) = stack.pop() {
            let x = i % self.w;
            let rem = i / self.w;
            let z = rem % self.d;
            let y = rem / self.d;
            for (dx, dy, dz) in NEIGHBORS {
                let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
                if self.is_solid(nx, ny, nz) {
                    let j = self.idx(nx as usize, ny as usize, nz as usize);
                    if !supported[j] {
                        supported[j] = true;
                        stack.push(j);
                    }
                }
            }
        }

        // Collect solid voxels the fill never reached.
        let mut out = Vec::new();
        for y in 0..self.h {
            for z in 0..self.d {
                for x in 0..self.w {
                    if self.is_solid(x as i32, y as i32, z as i32) && !supported[self.idx(x, y, z)]
                    {
                        out.push((x as i32, y as i32, z as i32));
                    }
                }
            }
        }
        out
    }
}

fn sign(x: f32) -> i32 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// Generate the world as a surface patch of the REAL layered Earth (planet::earth()): a grass skin over
/// basalt crust, peridotite mantle, iron core — Earth's true radial column as a declared VERTICAL LOD
/// (material order real; layer thicknesses compressed into the patch so the strata are visible when a
/// dig or impact excavates). A gentle value-noise heightfield undulates the grassy surface.
pub fn generate(materials: &[Material]) -> World {
    // Real Earth column (planet::earth(), docs/25/28): a biosphere skin over basalt CRUST, peridotite
    // MANTLE, iron CORE. This is a DECLARED VERTICAL LOD: the material order is Earth's real radial
    // structure, but the layer THICKNESSES are rebalanced into the ~48-voxel patch (real crust is 0.4%
    // of the radius — invisible at true scale), so a dig or a giant impact exposes honest strata from
    // this surface frame (Robin: "see Theia impact from this perspective"). Depths are compressed —
    // flagged; 1 voxel = 1 m holds only for the near-surface probe/dig physics.
    let grass = index_of(materials, "grass") as u16 + 1;
    let crust = index_of(materials, "basalt") as u16 + 1;
    let mantle = index_of(materials, "peridotite") as u16 + 1;
    let core = index_of(materials, "iron") as u16 + 1;

    let mut voxels = vec![0u16; W * H * D];
    let base_top = H as i32 - 8; // leave headroom above the terrain
    let amplitude = 6.0f32;

    // Flat strata boundaries (real geology is horizontal), measured down from the nominal surface. The
    // grass skin follows the undulating terrain top; the crust/mantle/core boundaries are level, so a
    // dig anywhere hits the same layer at the same depth.
    const CRUST_VOX: i32 = 12; // basalt crust band (LOD-inflated from ~25 km)
    const MANTLE_VOX: i32 = 22; // peridotite mantle band
    let crust_bottom = base_top - CRUST_VOX;
    let mantle_bottom = crust_bottom - MANTLE_VOX;

    let mut max_top = 0usize;
    for z in 0..D {
        for x in 0..W {
            let n = fbm(x as f32, z as f32); // 0..1
            let top = (base_top as f32 - amplitude * (1.0 - n)).round() as i32;
            let top = top.clamp(GRASS_THICKNESS as i32 + 1, H as i32 - 1);
            let grass_start = top - GRASS_THICKNESS as i32;
            for y in 0..top {
                let v = if y >= grass_start {
                    grass
                } else if y >= crust_bottom {
                    crust
                } else if y >= mantle_bottom {
                    mantle
                } else {
                    core
                };
                let i = (y as usize * D + z) * W + x;
                voxels[i] = v;
            }
            max_top = max_top.max(top as usize);
        }
    }

    World {
        w: W,
        h: H,
        d: D,
        voxels,
        max_top,
    }
}

// --- deterministic value noise (no RNG; stable across runs/clients) ---

fn hash2(x: i32, z: i32) -> f32 {
    let mut h = (x.wrapping_mul(374_761_393)).wrapping_add(z.wrapping_mul(668_265_263)) as u32;
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t) // smoothstep
}

/// Bilinearly-interpolated value noise at lattice frequency `freq`.
fn value_noise(x: f32, z: f32, freq: f32) -> f32 {
    let fx = x * freq;
    let fz = z * freq;
    let x0 = fx.floor() as i32;
    let z0 = fz.floor() as i32;
    let tx = smooth(fx - x0 as f32);
    let tz = smooth(fz - z0 as f32);
    let a = hash2(x0, z0);
    let b = hash2(x0 + 1, z0);
    let c = hash2(x0, z0 + 1);
    let d = hash2(x0 + 1, z0 + 1);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * tz
}

/// Two-octave fractal noise in 0..1.
fn fbm(x: f32, z: f32) -> f32 {
    let n = 0.65 * value_noise(x, z, 0.045) + 0.35 * value_noise(x, z, 0.11);
    n.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials;

    #[test]
    fn column_is_earths_real_layers_top_to_bottom() {
        // docs/28 A: the terrain is a surface patch of the REAL layered Earth (planet::earth()) as a
        // declared vertical LOD — grass skin, basalt CRUST, peridotite MANTLE, iron CORE, in that order
        // down a column. Asserts the strata (not game grass/dirt/granite) so a dig/impact exposes honest
        // composition. Depths are LOD-compressed; the ORDER and MATERIALS are Earth's.
        let mats = materials::load();
        let w = generate(&mats);
        let id = |name| materials::index_of(&mats, name);
        let (cx, cz) = (W as i32 / 2, D as i32 / 2);
        let top = w.surface_top_voxel(cx, cz).expect("solid column at centre");

        // Surface skin is grass; the first solid below it is basalt crust.
        assert_eq!(w.material_at(cx, top - 1, cz), Some(id("grass")), "surface skin");
        // Walk down and record the sequence of DISTINCT materials encountered.
        let mut seq: Vec<usize> = Vec::new();
        for y in (0..top).rev() {
            if let Some(m) = w.material_at(cx, y, cz) {
                if seq.last() != Some(&m) {
                    seq.push(m);
                }
            }
        }
        assert_eq!(
            seq,
            vec![id("grass"), id("basalt"), id("peridotite"), id("iron")],
            "column must be Earth's real radial order: grass → crust → mantle → core"
        );
    }
}
