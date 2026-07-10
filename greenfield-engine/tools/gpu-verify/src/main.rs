//! Headless GPU verification of the granular debris step (`shaders/particle_step.wgsl`) on a real
//! device (the box's RTX 2070, via Vulkan). Browser WebGPU can't be driven here, but native wgpu can
//! run the SAME shader, so we can actually confirm the spatial-hash + contact physics — not just
//! trust it by construction (docs/23).
//!
//! Scenes assert the behaviours that were missing:
//!   A. an overlapping pair pushes apart (contact repels),
//!   B. a vertical stack STAYS stacked (grains rest on each other — the fix for the "moiré" one-layer
//!      collapse and the stranded rim ring),
//!   C. nothing explodes / goes NaN.
//!
//! Exit code 0 = all scenes pass.

const SHADER: &str = include_str!("../../../shaders/particle_step.wgsl");

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Debug)]
struct Particle {
    offset: [f32; 3],
    temp: f32,
    vel: [f32; 3],
    resting: f32,
    color: [f32; 3],
    material: f32,
    emission: [f32; 3],
    _pad: f32,
}
impl Particle {
    fn at(x: f32, y: f32, z: f32) -> Self {
        Particle {
            offset: [x, y, z],
            temp: 300.0,
            vel: [0.0; 3],
            resting: 0.0,
            color: [0.5; 3],
            material: 0.0,
            emission: [0.0; 3],
            _pad: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    gravity: [f32; 3],
    dt: f32,
    center: [f32; 3],
    _c: f32,
    drag: f32,
    contact_damp: f32,
    settle_speed: f32,
    part_half: f32,
    cool_rate: f32,
    count: u32,
    world_w: u32,
    world_d: u32,
    cell_size: f32,
    table_mask: u32,
    bucket_k: u32,
    c_radius: f32,
    c_stiffness: f32,
    c_normal_damp: f32,
    c_friction: f32,
    c_tangent_damp: f32,
}

const PART_HALF: f32 = 0.21;
const TABLE_SIZE: u32 = 1 << 12; // 4096 cells — plenty for these tiny scenes
const BUCKET_K: u32 = 16;
const WORLD_W: u32 = 8;
const WORLD_D: u32 = 8;
const SUBSTEPS: u32 = 8;

fn base_params(count: u32) -> Params {
    Params {
        gravity: [0.0, -9.81, 0.0],
        dt: (1.0 / 60.0) / SUBSTEPS as f32,
        center: [4.0, 0.0, 4.0], // ground_y = heightfield_top(0) - center.y(0) = 0
        _c: 0.0,
        drag: 0.999,
        contact_damp: 0.4,
        settle_speed: 0.02,
        part_half: PART_HALF,
        cool_rate: 0.0,
        count,
        world_w: WORLD_W,
        world_d: WORLD_D,
        cell_size: 2.0 * PART_HALF, // ≥ contact diameter ⇒ contacts stay within ±1 cell
        table_mask: TABLE_SIZE - 1,
        bucket_k: BUCKET_K,
        c_radius: PART_HALF,
        c_stiffness: 1.0e5,
        c_normal_damp: 300.0,
        c_friction: 0.6,
        c_tangent_damp: 300.0,
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    clear: wgpu::ComputePipeline,
    insert: wgpu::ComputePipeline,
    forces: wgpu::ComputePipeline,
    integrate: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

fn init_gpu() -> Gpu {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no Vulkan adapter (RTX 2070 expected)");
    println!("adapter: {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gpu-verify"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
    }, None))
    .expect("request_device");

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("particle_step"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });

    let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage(1, false), // particles
            storage(2, true),  // heightfield
            storage(3, false), // grid_count
            storage(4, false), // grid_bucket
            storage(5, false), // forces
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pl"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let mk = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    Gpu {
        clear: mk("cs_grid_clear"),
        insert: mk("cs_grid_insert"),
        forces: mk("cs_forces"),
        integrate: mk("cs_integrate"),
        layout,
        device,
        queue,
    }
}

/// Run `frames` frames (each SUBSTEPS substeps) on `particles`; return the settled particles.
fn simulate(gpu: &Gpu, mut particles: Vec<Particle>, frames: u32) -> Vec<Particle> {
    use wgpu::util::DeviceExt;
    let count = particles.len() as u32;
    let params = base_params(count);

    let pbuf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particles"),
            contents: bytemuck::cast_slice(&particles),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
    let ubuf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    let heightfield = vec![0i32; (WORLD_W * WORLD_D) as usize];
    let hbuf = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("heightfield"),
            contents: bytemuck::cast_slice(&heightfield),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let make_storage = |label: &str, size: u64| {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    };
    let gcount = make_storage("grid_count", (TABLE_SIZE as u64) * 4);
    let gbucket = make_storage("grid_bucket", (TABLE_SIZE as u64) * (BUCKET_K as u64) * 4);
    let fbuf = make_storage("forces", (count as u64) * 16);

    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bind"),
        layout: &gpu.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: pbuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: hbuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: gcount.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: gbucket.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: fbuf.as_entire_binding() },
        ],
    });

    let ceil = |n: u32| n.div_ceil(64);
    for _ in 0..frames {
        for _ in 0..SUBSTEPS {
            let mut enc = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, &bind, &[]);
                pass.set_pipeline(&gpu.clear);
                pass.dispatch_workgroups(ceil(TABLE_SIZE), 1, 1);
                pass.set_pipeline(&gpu.insert);
                pass.dispatch_workgroups(ceil(count), 1, 1);
                pass.set_pipeline(&gpu.forces);
                pass.dispatch_workgroups(ceil(count), 1, 1);
                pass.set_pipeline(&gpu.integrate);
                pass.dispatch_workgroups(ceil(count), 1, 1);
            }
            gpu.queue.submit(Some(enc.finish()));
        }
    }

    // Read back.
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: (count as u64) * std::mem::size_of::<Particle>() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_buffer_to_buffer(&pbuf, 0, &staging, 0, staging.size());
    gpu.queue.submit(Some(enc.finish()));
    let slice = staging.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device.poll(wgpu::Maintain::Wait);
    let data = slice.get_mapped_range();
    particles = bytemuck::cast_slice::<u8, Particle>(&data).to_vec();
    drop(data);
    staging.unmap();
    particles
}

fn finite(ps: &[Particle]) -> bool {
    ps.iter().all(|p| p.offset.iter().all(|c| c.is_finite()))
}

fn main() {
    let gpu = init_gpu();
    let mut failures = 0;

    // Scene A: an overlapping pair must push apart (contact repels).
    {
        let ps = vec![
            Particle::at(-0.15, 5.0, 0.0), // overlap: centres 0.30 < diameter 0.42
            Particle::at(0.15, 5.0, 0.0),
        ];
        let out = simulate(&gpu, ps, 30);
        let sep = (out[1].offset[0] - out[0].offset[0]).abs();
        let ok = finite(&out) && sep >= 0.40;
        println!(
            "A pair-repels: separation {:.3} (want ≥0.40 = diameter)  {}",
            sep,
            if ok { "PASS" } else { "FAIL" }
        );
        failures += !ok as i32;
    }

    // Scene B: a vertical stack of 6 grains must STAY stacked (rest on each other), not collapse to a
    // single layer on the ground. This is the core fix.
    {
        let n = 6;
        let ps: Vec<Particle> = (0..n)
            .map(|k| Particle::at(0.0, PART_HALF + k as f32 * (2.0 * PART_HALF), 0.0))
            .collect();
        let out = simulate(&gpu, ps, 400);
        let ys: Vec<f32> = out.iter().map(|p| p.offset[1]).collect();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &y in &ys {
            lo = lo.min(y);
            hi = hi.max(y);
        }
        let span = hi - lo;
        // A collapsed pile would span ~0; a preserved stack of 6 spans ~5 diameters (2.1). Allow some
        // settling compression — require it clearly did NOT collapse.
        let ok = finite(&out) && span > 3.0 * (2.0 * PART_HALF) && lo > PART_HALF - 0.05;
        let mut sorted = ys.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "B stack-preserved: span {:.3} (want >{:.3}), heights {:?}  {}",
            span,
            3.0 * (2.0 * PART_HALF),
            sorted.iter().map(|y| (y * 100.0).round() / 100.0).collect::<Vec<_>>(),
            if ok { "PASS" } else { "FAIL" }
        );
        failures += !ok as i32;
    }

    // Scene C: a compact 5×5×3 block dropped onto the floor must settle finitely (no explosion) and
    // spread wider than it started (granular flow toward a slope).
    {
        let mut ps = Vec::new();
        for ix in 0..5 {
            for iz in 0..5 {
                for iy in 0..3 {
                    ps.push(Particle::at(
                        (ix as f32 - 2.0) * 0.44,
                        PART_HALF + 3.0 + iy as f32 * 0.44,
                        (iz as f32 - 2.0) * 0.44,
                    ));
                }
            }
        }
        let x0: Vec<f32> = ps.iter().map(|p| p.offset[0]).collect();
        let init_spread = x0.iter().cloned().fold(f32::MIN, f32::max)
            - x0.iter().cloned().fold(f32::MAX, f32::min);
        let out = simulate(&gpu, ps, 500);
        let xf: Vec<f32> = out.iter().map(|p| p.offset[0]).collect();
        let spread = xf.iter().cloned().fold(f32::MIN, f32::max)
            - xf.iter().cloned().fold(f32::MAX, f32::min);
        let ok = finite(&out) && spread >= init_spread;
        println!(
            "C pile-flows: x-spread {:.2} → {:.2} (want ≥ initial), finite {}  {}",
            init_spread,
            spread,
            finite(&out),
            if ok { "PASS" } else { "FAIL" }
        );
        failures += !ok as i32;
    }

    if failures == 0 {
        println!("\nALL GPU SCENES PASS ✔ (granular contact verified on real hardware)");
    } else {
        println!("\n{failures} GPU SCENE(S) FAILED �’");
        std::process::exit(1);
    }
}
