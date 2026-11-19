//! Nighthash-v2 on the GPU: Metal on macOS, CUDA (NVIDIA) on Windows and
//! Linux. Local-only, experimental.
//!
//! Every result is a candidate, not a proof: callers must re-check a winning
//! nonce with `nightfall_crypto::pow::nighthash` before submitting a block, so
//! a driver or kernel fault can never put an invalid block on the network.

pub mod argon;

#[cfg(target_os = "macos")]
mod metal_backend;
#[cfg(target_os = "macos")]
pub use metal_backend::GpuHasher;

#[cfg(any(windows, target_os = "linux"))]
mod cuda_backend;
#[cfg(any(windows, target_os = "linux"))]
pub use cuda_backend::GpuHasher;

/// Stub so callers compile on every other platform; it never initialises.
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub struct GpuHasher;

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
impl GpuHasher {
    pub fn new(_: nightfall_types::PowParams, _: usize) -> Result<Self, String> {
        Err("GPU mining is not available on this platform".into())
    }
}
