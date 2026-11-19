// Argon2id v1.3 block filling for Nighthash-v2: one lane, one pass.
//
// One threadgroup of 32 threads per nonce. Each block is 128 u64 words; each
// thread owns 4 of them. The permutation P runs on threadgroup memory: 8 rows,
// then 8 columns, each as two half-rounds of 4 independent G functions, so
// the 32 threads cover one half-round of all 8 rows (or columns) at once.
//
// Blocks 0 and 1 are written by the CPU before dispatch; the CPU also derives
// the final tag from block m'-1. This kernel fills blocks 2..m'-1.

#include <metal_stdlib>
using namespace metal;

struct FillParams {
    uint m;        // blocks in the lane (m')
    uint seg;      // m / 4
    uint nonces;   // threadgroups in this dispatch
    uint pad;
};

static inline ulong rotr64(ulong x, uint n) { return (x >> n) | (x << (64 - n)); }

static inline ulong blamka(ulong a, ulong b) {
    ulong lo = 0xFFFFFFFFul;
    return a + b + 2ul * (a & lo) * (b & lo);
}

static inline void G(threadgroup ulong *v, uint a, uint b, uint c, uint d) {
    ulong va = v[a], vb = v[b], vc = v[c], vd = v[d];
    va = blamka(va, vb); vd = rotr64(vd ^ va, 32);
    vc = blamka(vc, vd); vb = rotr64(vb ^ vc, 24);
    va = blamka(va, vb); vd = rotr64(vd ^ va, 16);
    vc = blamka(vc, vd); vb = rotr64(vb ^ vc, 63);
    v[a] = va; v[b] = vb; v[c] = vc; v[d] = vd;
}

// Word index of element j (0..15) of row r: 16r + j.
static inline uint row_ix(uint r, uint j) { return 16u * r + j; }
// Word index of element j (0..15) of column c: pairs (2c, 2c+1) of each row.
static inline uint col_ix(uint c, uint j) { return 16u * (j >> 1) + 2u * c + (j & 1u); }

// P on all rows then all columns of R. Thread t: line = t/4, g = t%4.
static inline void permute(threadgroup ulong *R, uint t) {
    uint line = t >> 2, g = t & 3u;
    // rows: column step, then diagonal step
    G(R, row_ix(line, g), row_ix(line, 4 + g), row_ix(line, 8 + g), row_ix(line, 12 + g));
    threadgroup_barrier(mem_flags::mem_threadgroup);
    G(R, row_ix(line, g), row_ix(line, 4 + ((g + 1) & 3u)),
         row_ix(line, 8 + ((g + 2) & 3u)), row_ix(line, 12 + ((g + 3) & 3u)));
    threadgroup_barrier(mem_flags::mem_threadgroup);
    // columns
    G(R, col_ix(line, g), col_ix(line, 4 + g), col_ix(line, 8 + g), col_ix(line, 12 + g));
    threadgroup_barrier(mem_flags::mem_threadgroup);
    G(R, col_ix(line, g), col_ix(line, 4 + ((g + 1) & 3u)),
         col_ix(line, 8 + ((g + 2) & 3u)), col_ix(line, 12 + ((g + 3) & 3u)));
    threadgroup_barrier(mem_flags::mem_threadgroup);
}

// next = P(prev ^ ref) ^ (prev ^ ref); prev := next. Pass 0, so no XOR-in.
static inline void fill_block(threadgroup ulong *P, threadgroup ulong *R,
                              device const ulong *ref, device ulong *next, uint t) {
    ulong keep[4];
    for (uint k = 0; k < 4; k++) {
        uint w = t * 4 + k;
        ulong r = ref[w] ^ P[w];
        R[w] = r;
        keep[k] = r;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    permute(R, t);
    for (uint k = 0; k < 4; k++) {
        uint w = t * 4 + k;
        ulong v = keep[k] ^ R[w];
        next[w] = v;
        P[w] = v;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
}

// Argon2i address block: A = G(0, G(0, Z)) after incrementing the counter.
static inline void next_addresses(threadgroup ulong *Z, threadgroup ulong *A,
                                  threadgroup ulong *R, uint t) {
    if (t == 0) { Z[6] += 1; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    ulong keep[4];
    for (uint k = 0; k < 4; k++) { uint w = t * 4 + k; R[w] = Z[w]; keep[k] = Z[w]; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    permute(R, t);
    for (uint k = 0; k < 4; k++) { uint w = t * 4 + k; ulong v = keep[k] ^ R[w]; A[w] = v; R[w] = v; keep[k] = v; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    permute(R, t);
    for (uint k = 0; k < 4; k++) { uint w = t * 4 + k; A[w] = keep[k] ^ R[w]; }
    threadgroup_barrier(mem_flags::mem_threadgroup);
}

kernel void argon2_fill(device ulong *mem [[buffer(0)]],
                        constant FillParams &prm [[buffer(1)]],
                        uint gid [[threadgroup_position_in_grid]],
                        uint t [[thread_index_in_threadgroup]]) {
    threadgroup ulong P[128];
    threadgroup ulong R[128];
    threadgroup ulong Z[128];
    threadgroup ulong A[128];

    device ulong *lane = mem + (ulong)gid * (ulong)prm.m * 128ul;

    for (uint k = 0; k < 4; k++) {
        uint w = t * 4 + k;
        P[w] = lane[128 + w];   // previous block = block 1
        Z[w] = 0;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);

    for (uint slice = 0; slice < 4; slice++) {
        bool independent = slice < 2;   // Argon2id: pass 0, first half
        uint start = (slice == 0) ? 2u : 0u;
        if (independent) {
            if (t == 0) {
                Z[0] = 0;               // pass
                Z[1] = 0;               // lane
                Z[2] = slice;
                Z[3] = prm.m;
                Z[4] = 1;               // total passes
                Z[5] = 2;               // Argon2id
                Z[6] = 0;               // counter
            }
            threadgroup_barrier(mem_flags::mem_threadgroup);
            if (slice == 0) { next_addresses(Z, A, R, t); }
        }
        for (uint i = start; i < prm.seg; i++) {
            if (independent && (i % 128u) == 0) { next_addresses(Z, A, R, t); }
            ulong pseudo = independent ? A[i % 128u] : P[0];
            ulong j1 = pseudo & 0xFFFFFFFFul;
            ulong area = (slice == 0) ? (ulong)(i - 1) : (ulong)(slice * prm.seg + i - 1);
            ulong x = (j1 * j1) >> 32;
            x = (area * x) >> 32;
            ulong ref_index = area - 1 - x;
            uint cur = slice * prm.seg + i;
            fill_block(P, R, lane + ref_index * 128ul, lane + (ulong)cur * 128ul, t);
        }
    }
}
