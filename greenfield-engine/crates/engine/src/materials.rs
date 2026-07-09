//! Loads the cited material database (`data/materials.json`) that ships with the engine.
//!
//! Phase 1 only needs each material's **density** (the physical source of truth) and **albedo**
//! (for rendering). We deserialize just those fields; serde ignores the rest. Later phases will
//! read the full mechanical/optical property set (see `docs/04-materials-model.md`).

use serde::Deserialize;

/// The material database, embedded at compile time so the WASM is self-contained.
const MATERIALS_JSON: &str = include_str!("../../../data/materials.json");

#[derive(Deserialize)]
struct RawFile {
    materials: Vec<RawMaterial>,
}

#[derive(Deserialize)]
struct RawMaterial {
    id: String,
    mechanical: RawMechanical,
    optical: RawOptical,
}

#[derive(Deserialize)]
struct RawMechanical {
    /// kg/m^3. Present for every material in the seed database.
    density: f32,
    /// Pa. Resistance to being pulled apart; null for liquids. Drives fracture (Phase 3).
    #[serde(default)]
    tensile_strength: Option<f32>,
    /// Pa. Fallback bonding strength where tensile isn't given.
    #[serde(default)]
    cohesion: Option<f32>,
}

#[derive(Deserialize)]
struct RawOptical {
    /// Linear RGB, each 0..1.
    albedo: [f32; 3],
}

/// A material as the engine consumes it.
#[derive(Clone, Debug)]
pub struct Material {
    pub id: String,
    /// kg/m^3. Authoritative per-material mass; drives self-gravity (voxel mass = density * volume).
    pub density: f32,
    pub albedo: [f32; 3],
    /// Pa. How hard it is to fracture/detach a chunk (Phase 3): rock is high (barely chips), soil and
    /// grass are ~1000× lower (detach easily). Falls back to cohesion, then to "effectively unbreakable".
    pub fracture_strength: f32,
}

/// Parse the embedded database. Panics with a clear message if the bundled JSON is malformed
/// (that would be a build-time data error, surfaced immediately in the console).
pub fn load() -> Vec<Material> {
    let file: RawFile =
        serde_json::from_str(MATERIALS_JSON).expect("bundled data/materials.json is invalid JSON");
    file.materials
        .into_iter()
        .map(|m| Material {
            id: m.id,
            density: m.mechanical.density,
            albedo: m.optical.albedo,
            fracture_strength: m
                .mechanical
                .tensile_strength
                .or(m.mechanical.cohesion)
                .unwrap_or(1.0e12),
        })
        .collect()
}

/// Find the index of a material by id. Panics if a required material is missing (Phase 1 relies
/// on `granite`, `dirt`, and `grass` existing in the seed set).
pub fn index_of(materials: &[Material], id: &str) -> usize {
    materials
        .iter()
        .position(|m| m.id == id)
        .unwrap_or_else(|| panic!("material '{id}' not found in materials.json"))
}
