// Space-band shader (scale-relative "orbit-to-ground", Step A): simple lit spheres for the planet and
// moon. No textures — just a per-body tint and one directional "sun", so we get a lit/dark hemisphere
// (phases). Positions/scales come from the N-body physics (orbit.rs), mapped to display units.

struct U {
    view_proj : mat4x4<f32>,
    model     : mat4x4<f32>,
    light_dir : vec4<f32>, // xyz = direction TO the sun, normalized
    tint      : vec4<f32>, // body color
};

@group(0) @binding(0) var<uniform> u : U;

struct VOut {
    @builtin(position) clip : vec4<f32>,
    @location(0) normal     : vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos : vec3<f32>, @location(1) nrm : vec3<f32>) -> VOut {
    var o : VOut;
    o.clip = u.view_proj * u.model * vec4<f32>(pos, 1.0);
    o.normal = (u.model * vec4<f32>(nrm, 0.0)).xyz;
    return o;
}

@fragment
fn fs_main(i : VOut) -> @location(0) vec4<f32> {
    let n = normalize(i.normal);
    let l = normalize(u.light_dir.xyz);
    let diffuse = max(dot(n, l), 0.0);
    let ambient = 0.05; // faint fill so the dark side isn't pure black
    return vec4<f32>(u.tint.rgb * (ambient + diffuse), 1.0);
}
