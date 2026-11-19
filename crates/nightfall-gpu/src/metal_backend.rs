use crate::argon::{self, BLOCK_WORDS};
use metal::{
    Buffer, CommandQueue, CompileOptions, ComputePipelineState, Device, MTLResourceOptions, MTLSize,
};
use nightfall_types::{Hash256, PowParams};

const SOURCE: &str = include_str!("argon2_fill.metal");

#[repr(C)]
#[derive(Clone, Copy)]
struct FillParams {
    m: u32,
    seg: u32,
    nonces: u32,
    pad: u32,
}

/// A batch hasher: `batch` nonces per GPU dispatch, each with its own
/// m' KiB workspace in one shared (unified-memory) buffer.
pub struct GpuHasher {
    params: PowParams,
    blocks: usize,
    batch: usize,
    queue: CommandQueue,
    pipeline: ComputePipelineState,
    buffer: Buffer,
    device_name: String,
}

impl GpuHasher {
    pub fn new(params: PowParams, batch: usize) -> Result<Self, String> {
        if !argon::supported(params) {
            return Err("unsupported PoW parameters for the GPU kernel".into());
        }
        let device = Device::system_default().ok_or("no Metal device")?;
        let library = device
            .new_library_with_source(SOURCE, &CompileOptions::new())
            .map_err(|e| format!("Metal shader compile failed: {e}"))?;
        let function = library
            .get_function("argon2_fill", None)
            .map_err(|e| format!("kernel missing: {e}"))?;
        let pipeline = device
            .new_compute_pipeline_state_with_function(&function)
            .map_err(|e| format!("pipeline failed: {e}"))?;
        let blocks = argon::lane_blocks(params);
        let lane_bytes = (blocks * BLOCK_WORDS * 8) as u64;
        let max = device.max_buffer_length();
        let batch = batch.max(1).min((max / lane_bytes) as usize).max(1);
        let buffer = device.new_buffer(
            lane_bytes * batch as u64,
            MTLResourceOptions::StorageModeShared,
        );
        Ok(Self {
            params,
            blocks,
            batch,
            queue: device.new_command_queue(),
            pipeline,
            buffer,
            device_name: device.name().to_string(),
        })
    }

    pub fn params(&self) -> PowParams {
        self.params
    }

    pub fn batch(&self) -> usize {
        self.batch
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn memory_bytes(&self) -> u64 {
        self.buffer.length()
    }

    /// Metal reports faults through the command buffer; nothing to collect.
    pub fn take_error(&mut self) -> Option<String> {
        None
    }

    /// Hash `nonces` (at most `batch`) for one header. Order is preserved.
    pub fn hash_batch(&mut self, header_preimage: &[u8], nonces: &[u64]) -> Vec<Hash256> {
        assert!(
            nonces.len() <= self.batch,
            "more nonces than the batch holds"
        );
        let n = nonces.len();
        if n == 0 {
            return Vec::new();
        }
        let lane_words = self.blocks * BLOCK_WORDS;
        let words = unsafe {
            std::slice::from_raw_parts_mut(
                self.buffer.contents() as *mut u64,
                lane_words * self.batch,
            )
        };

        let salt = argon::salt_for(header_preimage);
        for (i, &nonce) in nonces.iter().enumerate() {
            let h0 = argon::h0(self.params, header_preimage, nonce, &salt);
            argon::first_blocks(
                &h0,
                &mut words[i * lane_words..i * lane_words + 2 * BLOCK_WORDS],
            );
        }

        let fill = FillParams {
            m: self.blocks as u32,
            seg: (self.blocks / 4) as u32,
            nonces: n as u32,
            pad: 0,
        };
        let cmd = self.queue.new_command_buffer();
        let enc = cmd.new_compute_command_encoder();
        enc.set_compute_pipeline_state(&self.pipeline);
        enc.set_buffer(0, Some(&self.buffer), 0);
        enc.set_bytes(
            1,
            std::mem::size_of::<FillParams>() as u64,
            &fill as *const FillParams as *const std::ffi::c_void,
        );
        enc.dispatch_thread_groups(MTLSize::new(n as u64, 1, 1), MTLSize::new(32, 1, 1));
        enc.end_encoding();
        cmd.commit();
        cmd.wait_until_completed();

        (0..n)
            .map(|i| {
                let last = i * lane_words + (self.blocks - 1) * BLOCK_WORDS;
                argon::final_tag(&words[last..last + BLOCK_WORDS])
            })
            .collect()
    }
}
