//! NVIDIA GPU backend through the CUDA driver API.
//!
//! Nothing is linked at build time: `nvcuda.dll` (Windows) or `libcuda.so.1`
//! (Linux) is opened at runtime, so the program starts on machines without an
//! NVIDIA driver and simply reports that the GPU is unavailable. The kernel is
//! shipped as PTX and compiled for the installed card by the driver.

use crate::argon::{self, BLOCK_WORDS};
use libloading::{Library, Symbol};
use nightfall_types::{Hash256, PowParams};
use std::ffi::{c_char, c_int, c_uint, c_void, CStr};

const PTX: &str = concat!(include_str!("argon2_fill.ptx"), "\0");

type CuResult = c_int;
type CuDevice = c_int;
type CuContext = *mut c_void;
type CuModule = *mut c_void;
type CuFunction = *mut c_void;
type CuDevPtr = u64;

struct Api {
    _lib: Library,
    init: unsafe extern "system" fn(c_uint) -> CuResult,
    device_get: unsafe extern "system" fn(*mut CuDevice, c_int) -> CuResult,
    device_get_name: unsafe extern "system" fn(*mut c_char, c_int, CuDevice) -> CuResult,
    ctx_create: unsafe extern "system" fn(*mut CuContext, c_uint, CuDevice) -> CuResult,
    ctx_set_current: unsafe extern "system" fn(CuContext) -> CuResult,
    ctx_destroy: unsafe extern "system" fn(CuContext) -> CuResult,
    ctx_synchronize: unsafe extern "system" fn() -> CuResult,
    mem_get_info: unsafe extern "system" fn(*mut usize, *mut usize) -> CuResult,
    module_load_data_ex: unsafe extern "system" fn(
        *mut CuModule,
        *const c_void,
        c_uint,
        *mut c_int,
        *mut *mut c_void,
    ) -> CuResult,
    module_get_function:
        unsafe extern "system" fn(*mut CuFunction, CuModule, *const c_char) -> CuResult,
    mem_alloc: unsafe extern "system" fn(*mut CuDevPtr, usize) -> CuResult,
    mem_free: unsafe extern "system" fn(CuDevPtr) -> CuResult,
    memcpy_htod: unsafe extern "system" fn(CuDevPtr, *const c_void, usize) -> CuResult,
    memcpy_dtoh: unsafe extern "system" fn(*mut c_void, CuDevPtr, usize) -> CuResult,
    launch_kernel: unsafe extern "system" fn(
        CuFunction,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        c_uint,
        *mut c_void,
        *mut *mut c_void,
        *mut *mut c_void,
    ) -> CuResult,
}

#[cfg(windows)]
const LIB_NAMES: &[&str] = &["nvcuda.dll"];
#[cfg(not(windows))]
const LIB_NAMES: &[&str] = &["libcuda.so.1", "libcuda.so"];

impl Api {
    fn load() -> Result<Self, String> {
        let lib = LIB_NAMES
            .iter()
            .find_map(|n| unsafe { Library::new(n) }.ok())
            .ok_or("no NVIDIA driver found (nvcuda.dll)")?;
        unsafe {
            macro_rules! sym {
                ($name:literal) => {{
                    let s: Symbol<_> = lib
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("driver lacks {}: {e}", $name))?;
                    *s
                }};
            }
            Ok(Self {
                init: sym!("cuInit"),
                device_get: sym!("cuDeviceGet"),
                device_get_name: sym!("cuDeviceGetName"),
                ctx_create: sym!("cuCtxCreate_v2"),
                ctx_set_current: sym!("cuCtxSetCurrent"),
                ctx_destroy: sym!("cuCtxDestroy_v2"),
                ctx_synchronize: sym!("cuCtxSynchronize"),
                mem_get_info: sym!("cuMemGetInfo_v2"),
                module_load_data_ex: sym!("cuModuleLoadDataEx"),
                module_get_function: sym!("cuModuleGetFunction"),
                mem_alloc: sym!("cuMemAlloc_v2"),
                mem_free: sym!("cuMemFree_v2"),
                memcpy_htod: sym!("cuMemcpyHtoD_v2"),
                memcpy_dtoh: sym!("cuMemcpyDtoH_v2"),
                launch_kernel: sym!("cuLaunchKernel"),
                _lib: lib,
            })
        }
    }
}

fn check(r: CuResult, what: &str) -> Result<(), String> {
    if r == 0 {
        Ok(())
    } else {
        Err(format!("CUDA {what} failed (error {r})"))
    }
}

/// A batch hasher: `batch` nonces per launch, each with its own m' KiB
/// workspace in device memory.
pub struct GpuHasher {
    api: Api,
    ctx: CuContext,
    func: CuFunction,
    params: PowParams,
    blocks: usize,
    batch: usize,
    mem: CuDevPtr,
    init: CuDevPtr,
    out: CuDevPtr,
    host_init: Vec<u64>,
    host_out: Vec<u64>,
    device_name: String,
    workspace_bytes: u64,
    last_error: Option<String>,
}

// The context is only used from the thread that owns the hasher; it is made
// current on every call, so moving the hasher between threads is sound.
unsafe impl Send for GpuHasher {}

impl GpuHasher {
    pub fn new(params: PowParams, batch: usize) -> Result<Self, String> {
        if !argon::supported(params) {
            return Err("unsupported PoW parameters for the GPU kernel".into());
        }
        let api = Api::load()?;
        unsafe {
            check((api.init)(0), "init")?;
            let mut dev: CuDevice = 0;
            check((api.device_get)(&mut dev, 0), "device")?;
            let mut name = [0 as c_char; 256];
            check(
                (api.device_get_name)(name.as_mut_ptr(), 255, dev),
                "device name",
            )?;
            let device_name = CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned();

            let mut ctx: CuContext = std::ptr::null_mut();
            check((api.ctx_create)(&mut ctx, 0, dev), "context")?;

            let mut module: CuModule = std::ptr::null_mut();
            let r = (api.module_load_data_ex)(
                &mut module,
                PTX.as_ptr() as *const c_void,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if r != 0 {
                (api.ctx_destroy)(ctx);
                return Err(format!(
                    "CUDA kernel load failed (error {r}) — update the NVIDIA driver"
                ));
            }
            let mut func: CuFunction = std::ptr::null_mut();
            check(
                (api.module_get_function)(&mut func, module, c"argon2_fill".as_ptr()),
                "kernel lookup",
            )?;

            let blocks = argon::lane_blocks(params);
            let lane_bytes = (blocks * BLOCK_WORDS * 8) as u64;
            let (mut free, mut total) = (0usize, 0usize);
            check((api.mem_get_info)(&mut free, &mut total), "memory info")?;
            // Leave a quarter of the free memory to the display and driver.
            let fit = ((free as u64 / 4 * 3) / lane_bytes).max(1) as usize;
            let batch = batch.max(1).min(fit);

            let mut mem: CuDevPtr = 0;
            let mut init: CuDevPtr = 0;
            let mut out: CuDevPtr = 0;
            check(
                (api.mem_alloc)(&mut mem, lane_bytes as usize * batch),
                "workspace alloc",
            )?;
            check(
                (api.mem_alloc)(&mut init, batch * 2 * BLOCK_WORDS * 8),
                "input alloc",
            )?;
            check(
                (api.mem_alloc)(&mut out, batch * BLOCK_WORDS * 8),
                "output alloc",
            )?;

            let mut me = Self {
                api,
                ctx,
                func,
                params,
                blocks,
                batch,
                mem,
                init,
                out,
                host_init: vec![0; batch * 2 * BLOCK_WORDS],
                host_out: vec![0; batch * BLOCK_WORDS],
                device_name,
                workspace_bytes: lane_bytes * batch as u64,
                last_error: None,
            };
            me.self_test()?;
            Ok(me)
        }
    }

    /// Refuse to mine with a card that does not reproduce the consensus hash.
    fn self_test(&mut self) -> Result<(), String> {
        let pre: Vec<u8> = (0..96u32)
            .map(|i| (i as u8).wrapping_mul(29).wrapping_add(7))
            .collect();
        let n = self.batch.min(4);
        let nonces: Vec<u64> = [0u64, 1, u64::MAX, 0x9E37_79B9_7F4A_7C15][..n].to_vec();
        let got = self.hash_batch(&pre, &nonces);
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        for (nonce, h) in nonces.iter().zip(got) {
            if h != nightfall_crypto::nighthash(&pre, *nonce, self.params) {
                return Err(format!(
                    "self-test failed on {}: GPU hash differs from the CPU — GPU disabled",
                    self.device_name
                ));
            }
        }
        Ok(())
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
        self.workspace_bytes
    }

    /// A device error from the last batch, if any. The caller switches the
    /// GPU off instead of spinning on a card that stopped working.
    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// Hash `nonces` (at most `batch`) for one header. Order is preserved.
    /// A device error yields hashes that cannot match (all 0xFF), which the
    /// caller treats as "nothing found"; real hits are always CPU-checked.
    pub fn hash_batch(&mut self, header_preimage: &[u8], nonces: &[u64]) -> Vec<Hash256> {
        assert!(
            nonces.len() <= self.batch,
            "more nonces than the batch holds"
        );
        let n = nonces.len();
        if n == 0 {
            return Vec::new();
        }
        match self.run(header_preimage, nonces) {
            Ok(()) => (0..n)
                .map(|i| argon::final_tag(&self.host_out[i * BLOCK_WORDS..(i + 1) * BLOCK_WORDS]))
                .collect(),
            Err(e) => {
                self.last_error = Some(e);
                vec![Hash256([0xFF; 32]); n]
            }
        }
    }

    fn run(&mut self, header_preimage: &[u8], nonces: &[u64]) -> Result<(), String> {
        let n = nonces.len();
        let salt = argon::salt_for(header_preimage);
        for (i, &nonce) in nonces.iter().enumerate() {
            let h0 = argon::h0(self.params, header_preimage, nonce, &salt);
            argon::first_blocks(
                &h0,
                &mut self.host_init[i * 2 * BLOCK_WORDS..(i + 1) * 2 * BLOCK_WORDS],
            );
        }
        unsafe {
            check((self.api.ctx_set_current)(self.ctx), "set context")?;
            check(
                (self.api.memcpy_htod)(
                    self.init,
                    self.host_init.as_ptr() as *const c_void,
                    n * 2 * BLOCK_WORDS * 8,
                ),
                "upload",
            )?;
            let mut mem = self.mem;
            let mut init = self.init;
            let mut out = self.out;
            let mut m = self.blocks as u32;
            let mut seg = (self.blocks / 4) as u32;
            let mut count = n as u32;
            let mut args: [*mut c_void; 6] = [
                &mut mem as *mut _ as *mut c_void,
                &mut init as *mut _ as *mut c_void,
                &mut out as *mut _ as *mut c_void,
                &mut m as *mut _ as *mut c_void,
                &mut seg as *mut _ as *mut c_void,
                &mut count as *mut _ as *mut c_void,
            ];
            check(
                (self.api.launch_kernel)(
                    self.func,
                    n as c_uint,
                    1,
                    1,
                    32,
                    1,
                    1,
                    0,
                    std::ptr::null_mut(),
                    args.as_mut_ptr(),
                    std::ptr::null_mut(),
                ),
                "launch",
            )?;
            check((self.api.ctx_synchronize)(), "kernel")?;
            check(
                (self.api.memcpy_dtoh)(
                    self.host_out.as_mut_ptr() as *mut c_void,
                    self.out,
                    n * BLOCK_WORDS * 8,
                ),
                "download",
            )?;
        }
        Ok(())
    }
}

impl Drop for GpuHasher {
    fn drop(&mut self) {
        unsafe {
            (self.api.ctx_set_current)(self.ctx);
            (self.api.mem_free)(self.mem);
            (self.api.mem_free)(self.init);
            (self.api.mem_free)(self.out);
            (self.api.ctx_destroy)(self.ctx);
        }
    }
}
