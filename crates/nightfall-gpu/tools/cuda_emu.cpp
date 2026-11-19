// Host emulation of argon2_fill.cu: the same kernel source, run with 32 CPU
// threads per nonce and a real barrier in place of __syncthreads(). Used to
// check the CUDA kernel against the consensus hash on machines without an
// NVIDIA GPU.  Usage: cuda_emu m seg nonces < init.bin > out.bin
#include <barrier>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <thread>
#include <vector>

static thread_local std::barrier<> *g_bar = nullptr;
#define NF_DEVICE static inline
#define NF_SYNC() g_bar->arrive_and_wait()
#define NF_SHARED
#define NF_HOST_EMULATION 1
#include "../src/argon2_fill.cu"

int main(int argc, char **argv) {
    if (argc != 4) { std::fprintf(stderr, "usage: cuda_emu m seg nonces\n"); return 2; }
    u32 m = std::atoi(argv[1]), seg = std::atoi(argv[2]), nonces = std::atoi(argv[3]);
    std::vector<u64> init((size_t)nonces * 256), out((size_t)nonces * 128);
    if (std::fread(init.data(), 8, init.size(), stdin) != init.size()) return 3;
    std::vector<u64> mem((size_t)nonces * m * 128);
    for (u32 gid = 0; gid < nonces; gid++) {
        u64 P[128], R[128], Z[128], A[128];
        std::barrier<> bar(32);
        std::vector<std::thread> th;
        for (u32 t = 0; t < 32; t++) {
            th.emplace_back([&, t] {
                g_bar = &bar;
                argon2_fill_one(mem.data(), init.data(), out.data(), m, seg, nonces, P, R, Z, A, gid, t);
            });
        }
        for (auto &x : th) x.join();
    }
    std::fwrite(out.data(), 8, out.size(), stdout);
    return 0;
}
