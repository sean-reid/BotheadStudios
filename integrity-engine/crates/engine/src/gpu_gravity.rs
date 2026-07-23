//! **GPU direct-sum self-gravity — the engine's GPU backend for a per-particle N-body force.**
//!
//! Self-gravity on a debris cloud is embarrassingly parallel — every particle sums the pull of every
//! other — yet the orbital scene was walking a Barnes-Hut tree on a SINGLE CPU THREAD (34 ms/pass at
//! N=3000, measured; 64% of the frame). A GPU does the same sum in microseconds, in parallel, and
//! EXACTLY: there is no opening-angle multipole error, so it is higher fidelity as well as far faster.
//!
//! This wraps the already-verified `cs_gravity_direct` kernel in `shaders/bh_gravity.wgsl` (checked
//! against the CPU direct sum in `tools/gpu-bh-verify`). It is the piece a foreseen collision dispatches
//! to: the engine forecasts the impact (`interaction::detect`), materialises the particles, and, at or
//! above the measured [`DIRECT_SUM_KNEE`], steps their self-gravity here instead of on the CPU
//! ([`crate::aggregate::Aggregate`] carries the [`GravityField`] and makes the shunt per pass). Birth
//! already runs its SPH gravity on the GPU this way; this brings the orbital debris onto the same footing.
//!
//! **Precision.** The kernel is f32 (the GPU's native width); the CPU reference is f64. For self-gravity
//! that difference is far below the chaos/theta noise the disk already tolerates — the correctness test
//! pins the agreement.

use glam::DVec3;
use wgpu::util::DeviceExt;

/// **The measured CPU/GPU knee for the direct-sum dispatch** (particle count). Below it the CPU is
/// genuinely faster and the aggregate stays on its brute/tree path; at or above it the GPU direct sum is
/// dispatched. Read off `gpu_gravity_speedup`, not guessed: on an Apple M4 Max (Metal) the GPU per-call
/// round trip is ~1.3 ms and nearly flat, while the CPU cost per pass crosses it between N=400 (0.5x,
/// CPU wins) and N=750 (2.0x, GPU wins); equal-cost interpolation lands at N of roughly 550. The
/// discrete-card numbers already recorded in this file's bench history (RTX 5060 Ti: ~2.5 ms floor
/// against a 48 ms single-thread brute sum at N=3000) put their crossover in the same few-hundred
/// range, so 600 sits on the CPU-favoured side of every measured box. Above the knee the dispatch is
/// never slower than the tree on any measured hardware, and it is EXACT (no theta multipole error):
/// higher fidelity and higher speed together.
pub const DIRECT_SUM_KNEE: usize = 600;

/// Matches `Params` in `bh_gravity.wgsl`. Only `n` and `soft2` matter for the direct sum; the rest belong
/// to the tree kernels in the same file and are left zero here.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: u32,
    theta: f32,
    soft2: f32,
    n_leaves: u32,
    bucket_k: u32,
    _pad: [u32; 3],
}

/// A reusable GPU direct-sum gravity pipeline. Build once; call [`GpuGravity::accelerations`] per step.
pub struct GpuGravity {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuGravity {
    /// Build the pipeline for the `cs_gravity_direct` entry point of `bh_gravity.wgsl`.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu-gravity"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../shaders/bh_gravity.wgsl").into()),
        });
        // cs_gravity_direct touches only bindings 0 (Params), 1 (bodies), 2 (acc).
        let entry = |binding: u32, read_only: bool, uniform: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: if uniform {
                    wgpu::BufferBindingType::Uniform
                } else {
                    wgpu::BufferBindingType::Storage { read_only }
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu-gravity-layout"),
            entries: &[entry(0, true, true), entry(1, true, false), entry(2, false, false)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu-gravity-pipeline-layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu-gravity"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_gravity_direct"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }

    /// Softened self-gravity acceleration on every body, computed on the GPU. `softening` is the Plummer
    /// length (m). Synchronous: uploads the bodies, dispatches, and reads the result back with a blocking
    /// poll, which is correct for a native step and for measurement. Native only, because the blocking
    /// poll is exactly what a browser forbids; [`GravityField`] wraps the same dispatch in the engine's
    /// two-phase read-back there.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn accelerations(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pos: &[DVec3],
        mass: &[f64],
        softening: f64,
    ) -> Vec<DVec3> {
        if pos.is_empty() {
            return Vec::new();
        }
        let staging = self.dispatch_to_staging(device, queue, pos, mass, softening);
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::Maintain::Wait);
        let raw: Vec<[f32; 4]> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        raw.iter().map(|a| DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64)).collect()
    }

    /// Upload the bodies, dispatch `cs_gravity_direct`, and submit a copy of the acceleration field into
    /// a fresh MAP_READ staging buffer. The one dispatch both platforms share: the native blocking path
    /// polls the returned buffer immediately; the browser path maps it asynchronously and collects it a
    /// later pass ([`GravityField`]).
    fn dispatch_to_staging(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pos: &[DVec3],
        mass: &[f64],
        softening: f64,
    ) -> wgpu::Buffer {
        let n = pos.len();
        let bodies: Vec<[f32; 4]> = pos
            .iter()
            .zip(mass)
            .map(|(p, &m)| [p.x as f32, p.y as f32, p.z as f32, m as f32])
            .collect();
        let params = Params {
            n: n as u32,
            theta: 0.0,
            soft2: (softening * softening) as f32,
            n_leaves: n as u32,
            bucket_k: 1,
            _pad: [0; 3],
        };
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gravity-params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bodies_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gravity-bodies"),
            contents: bytemuck::cast_slice(&bodies),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let acc_size = (n * 16) as u64; // vec4<f32>
        let acc_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gravity-acc"),
            size: acc_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gravity-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: bodies_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: acc_buf.as_entire_binding() },
            ],
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gravity-dispatch"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((n as u32).div_ceil(64), 1, 1);
        }
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gravity-staging"),
            size: acc_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_buffer_to_buffer(&acc_buf, 0, &staging, 0, acc_size);
        queue.submit(Some(enc.finish()));
        staging
    }
}

/// **The live dispatch**: [`GpuGravity`] bound to the device and queue it runs on, attachable to a
/// particle system ([`crate::aggregate::Aggregate::gpu_gravity`]). The device is the SHARED one: in the
/// browser it is the same device the renderer and `gpu_sph` already run on (a scene hands it over when
/// the debris materialises), natively it comes from [`crate::gpu_host::GpuHost`]. wgpu devices and
/// queues are reference-counted handles, so cloning binds, it does not duplicate the GPU.
///
/// Two execution modes, one per platform, because WebGPU forbids blocking (`Maintain::Wait` is a no-op
/// in the browser, the `gpu_sph` lesson):
///  * **native**, synchronous: submit this pass's positions and read the field back within the pass.
///    Exact at the pass's own positions; this is what the correctness test and the bench measure.
///  * **wasm**, two-phase (the `gpu_store` read-back pattern): a pass harvests the field submitted by
///    the PREVIOUS pass if the map has completed, and submits the current positions for a later one.
///    The harvested field is one submission old, the same class of deferral the block-timestep
///    scheduler already accepts when a coasting particle keeps the acceleration from its last kick.
///    While nothing has landed yet, or the particle count changed mid-flight (an absorb/drain), the
///    caller falls back to the CPU tree for that pass, so no pass ever blocks and none goes without
///    gravity.
pub struct GravityField {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: GpuGravity,
    /// In-flight staging buffer and its completion flag. `Arc<AtomicBool>`, not `Rc<Cell<bool>>`: wgpu
    /// bounds the `map_async` callback by `WasmNotSend` (plain `Send` off-wasm), so the `Rc` form
    /// compiles only for the browser (the defect `gpu_store` had to fix twice).
    staging: Option<wgpu::Buffer>,
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl GravityField {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        GravityField {
            pipeline: GpuGravity::new(device),
            device: device.clone(),
            queue: queue.clone(),
            staging: None,
            ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// The self-gravity field for these positions, or `None` when the caller must take the CPU path for
    /// this pass (browser only: first pass, map still in flight, or a mid-flight count change).
    pub fn accelerations(&mut self, pos: &[DVec3], mass: &[f64], softening: f64) -> Option<Vec<DVec3>> {
        if pos.is_empty() {
            return Some(Vec::new());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Native blocks legally: submit, wait, read. The field is exact, never deferred.
            self.staging = None; // drop any stale in-flight buffer (defensive; native drains every pass)
            self.submit(pos, mass, softening);
            self.device.poll(wgpu::Maintain::Wait);
            self.take()
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Harvest the previous pass's field if its map completed; reject it if the cloud changed
            // size mid-flight (stale shape would misassign forces). Then keep exactly one job in flight.
            let harvested = self.take().filter(|g| g.len() == pos.len());
            if self.staging.is_none() {
                self.submit(pos, mass, softening);
            }
            harvested
        }
    }

    /// Phase 1: dispatch the direct sum for these positions and start the async map of the result.
    fn submit(&mut self, pos: &[DVec3], mass: &[f64], softening: f64) {
        let staging = self.pipeline.dispatch_to_staging(&self.device, &self.queue, pos, mass, softening);
        self.ready.store(false, std::sync::atomic::Ordering::Release);
        let flag = self.ready.clone();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |res| {
            if res.is_ok() {
                flag.store(true, std::sync::atomic::Ordering::Release);
            }
        });
        self.staging = Some(staging);
    }

    /// Phase 2: the mapped field if the map has completed, else `None` while pending or idle.
    fn take(&mut self) -> Option<Vec<DVec3>> {
        if !self.ready.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }
        let staging = self.staging.take()?;
        let out = {
            let data = staging.slice(..).get_mapped_range();
            bytemuck::cast_slice::<u8, [f32; 4]>(&data)
                .iter()
                .map(|a| DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64))
                .collect()
        };
        staging.unmap();
        self.ready.store(false, std::sync::atomic::Ordering::Release);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_direct(pos: &[DVec3], mass: &[f64], soft: f64) -> Vec<DVec3> {
        let s2 = soft * soft;
        (0..pos.len())
            .map(|i| {
                let mut a = DVec3::ZERO;
                for j in 0..pos.len() {
                    if i == j {
                        continue;
                    }
                    let d = pos[j] - pos[i];
                    let r2 = d.length_squared() + s2;
                    a += d * (crate::orbit::G * mass[j] / (r2 * r2.sqrt()));
                }
                a
            })
            .collect()
    }

    /// **The GPU gravity must equal the CPU brute-force sum.** Same softened Newtonian force, so the only
    /// difference is f32-vs-f64 rounding — far below the noise the disk already lives with. If this
    /// drifts, the backend is computing different physics, and no speedup is worth that. Skips cleanly
    /// when no GPU is present.
    #[test]
    fn gpu_gravity_matches_the_cpu_direct_sum() {
        let Ok(host) = crate::gpu_host::GpuHost::headless(None) else {
            eprintln!("no GPU device - skipping gpu_gravity correctness test");
            return;
        };
        let mut s = 0x51ED_2707u64;
        let mut rng = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 40) as f64 / (1u64 << 24) as f64 - 0.5
        };
        let n = 1500;
        let pos: Vec<DVec3> = (0..n)
            .map(|_| DVec3::new(rng(), rng(), rng()).normalize_or_zero() * (1.7e6 * (0.2 + rng().abs())))
            .collect();
        let mass: Vec<f64> = (0..n).map(|_| 5.0e19).collect();
        let soft = 2.5e4;

        let g = GpuGravity::new(&host.device);
        let gpu = g.accelerations(&host.device, &host.queue, &pos, &mass, soft);
        let cpu = cpu_direct(&pos, &mass, soft);

        let mut worst = 0.0f64;
        for (a, b) in gpu.iter().zip(&cpu) {
            let denom = b.length().max(1e-30);
            worst = worst.max((*a - *b).length() / denom);
        }
        assert!(
            worst < 1e-3,
            "GPU direct-sum gravity must match the CPU sum to f32 precision; worst relative error {worst:.2e}"
        );
    }

    /// A debris-like self-gravitating cloud (the moon-drop configuration, gravity isolated) at count `n`.
    fn debris_cloud(n: usize, seed: u64) -> Vec<crate::orbit::Body> {
        let mut s = seed;
        let mut rng = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 40) as f64 / (1u64 << 24) as f64 - 0.5
        };
        (0..n)
            .map(|_| crate::orbit::Body {
                pos: DVec3::new(rng(), rng(), rng()).normalize_or_zero()
                    * (1.7e6 * (0.2 + rng().abs())),
                vel: DVec3::ZERO,
                mass: 5.0e19,
            })
            .collect()
    }

    /// **The live dispatch, exercised through the aggregate it is wired into.** Above the knee, an
    /// aggregate carrying a [`GravityField`] must produce the same self-gravity the CPU tree would,
    /// within the tree's own θ bound (RMS relative < 1e-2, the bound `tools/gpu-bh-verify` enforces
    /// between the tree and the direct sum). The GPU sum is EXACT, so the difference measured here IS
    /// the tree's multipole error; past the bound, the dispatch is computing different physics. Skips
    /// cleanly with no GPU.
    #[test]
    fn the_aggregate_dispatch_matches_the_cpu_tree_within_the_theta_bound() {
        let Ok(host) = crate::gpu_host::GpuHost::headless(None) else {
            eprintln!("no GPU device - skipping aggregate dispatch test");
            return;
        };
        let n = 1500; // above DIRECT_SUM_KNEE, the moon-drop cloud's scale
        assert!(n >= DIRECT_SUM_KNEE);
        let soft = 2.5e4;
        let bodies = debris_cloud(n, 0xA66E_D15Bu64);
        let mut cpu_agg = crate::aggregate::Aggregate::new(bodies.clone(), soft);
        let mut gpu_agg = crate::aggregate::Aggregate::new(bodies, soft);
        gpu_agg.gpu_gravity = Some(GravityField::new(&host.device, &host.queue));

        let a_cpu = cpu_agg.accelerations();
        let a_gpu = gpu_agg.accelerations();
        assert!(
            a_cpu.iter().zip(&a_gpu).any(|(a, b)| a != b),
            "identical fields mean the GPU path was never taken; the knee gate is not dispatching"
        );
        let (mut err_sq, mut ref_sq) = (0.0f64, 0.0f64);
        for (g, c) in a_gpu.iter().zip(&a_cpu) {
            err_sq += (*g - *c).length_squared();
            ref_sq += c.length_squared();
        }
        let rms = (err_sq / ref_sq.max(1e-300)).sqrt();
        assert!(
            rms < 1e-2,
            "the dispatched GPU field must agree with the CPU tree within its θ bound; RMS rel {rms:.2e}"
        );
    }

    /// **Below the knee the CPU path stands**, field attached or not: the measured crossover says the
    /// CPU is faster there, so the gate must not dispatch. Pinned by bitwise identity (the same CPU
    /// code runs in both aggregates). Skips cleanly with no GPU.
    #[test]
    fn below_the_knee_the_cpu_path_stands_even_with_a_field_attached() {
        let Ok(host) = crate::gpu_host::GpuHost::headless(None) else {
            eprintln!("no GPU device - skipping knee gate test");
            return;
        };
        let n = 300; // below DIRECT_SUM_KNEE
        assert!(n < DIRECT_SUM_KNEE);
        let bodies = debris_cloud(n, 0x0BE1_0B0Bu64);
        let mut cpu_agg = crate::aggregate::Aggregate::new(bodies.clone(), 2.5e4);
        let mut gpu_agg = crate::aggregate::Aggregate::new(bodies, 2.5e4);
        gpu_agg.gpu_gravity = Some(GravityField::new(&host.device, &host.queue));
        assert_eq!(
            cpu_agg.accelerations(),
            gpu_agg.accelerations(),
            "below the knee both aggregates must take the identical CPU path"
        );
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    /// **GPU vs CPU Barnes-Hut at production N**, so the dispatch win is measured, not asserted. Reports
    /// the GPU per-call time INCLUDING the upload+dispatch+readback round trip (the honest cost of the
    /// synchronous backend) against the CPU tree at the SAME per-pass cost the live path pays
    /// (`Aggregate::accelerations_masked` rebuilds the tree every pass, so build + eval, not eval alone).
    /// The N sweep brackets the crossover so [`DIRECT_SUM_KNEE`] is read off this table, not guessed.
    ///
    /// Run: `cargo test --lib gpu_gravity_speedup -- --ignored --nocapture`
    /// (multi-GPU boxes: set `INTEGRITY_ADAPTER`; the harness refuses to guess).
    #[test]
    #[ignore = "perf bench — needs a GPU; run with --ignored --nocapture"]
    fn gpu_gravity_speedup() {
        let Ok(host) = crate::gpu_host::GpuHost::headless(None) else {
            eprintln!("no GPU — skipping");
            return;
        };
        println!("  adapter: {} ({:?})", host.info.name, host.info.backend);
        let mut s = 0xBEEF_1234u64;
        let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; (s >> 40) as f64 / (1u64 << 24) as f64 - 0.5 };
        let g = GpuGravity::new(&host.device);
        for &n in &[200usize, 400, 750, 1000, 1500, 2000, 3000, 6000] {
            let pos: Vec<DVec3> = (0..n).map(|_| DVec3::new(rng(), rng(), rng()).normalize_or_zero() * 1.7e6).collect();
            let mass: Vec<f64> = (0..n).map(|_| 5.0e19).collect();
            // Warm up (shader/pipeline, first alloc).
            let _ = g.accelerations(&host.device, &host.queue, &pos, &mass, 2.5e4);
            let t = Instant::now();
            for _ in 0..10 { let _ = g.accelerations(&host.device, &host.queue, &pos, &mass, 2.5e4); }
            let gpu_ms = t.elapsed().as_secs_f64() * 1e3 / 10.0;
            // CPU Barnes-Hut at the live per-pass cost: build the tree AND evaluate it, like the
            // aggregate's acceleration pass does (positions move every pass, so the tree cannot be reused).
            let t = Instant::now();
            for _ in 0..3 {
                let bh = crate::bhtree::BarnesHut::build(&pos, &mass, 0.5, 2.5e4);
                let _ = bh.accelerations(&pos, &mass);
            }
            let cpu_ms = t.elapsed().as_secs_f64() * 1e3 / 3.0;
            println!("  N={n:<5} GPU {gpu_ms:6.2} ms (incl round trip)   CPU-BH build+eval {cpu_ms:6.2} ms   -> {:.1}x", cpu_ms / gpu_ms.max(1e-6));
        }
    }
}
