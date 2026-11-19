// Argon2id v1.3 block filling for Nighthash-v2: one lane, one pass.
//
// Same algorithm as argon2_fill.metal. One CUDA block of 32 threads (one warp)
// per nonce; each thread owns 4 of the 128 words. Blocks 0 and 1 come from the
// host in `init`; the last block is written to `out` for the host to finish.

typedef unsigned long long u64;
typedef unsigned int u32;

#ifndef NF_DEVICE
#define NF_DEVICE __device__ __forceinline__
#define NF_SYNC() __syncthreads()
#define NF_SHARED __shared__
#define NF_TID threadIdx.x
#define NF_GID blockIdx.x
#endif

NF_DEVICE u64 rotr64(u64 x, u32 n) { return (x >> n) | (x << (64 - n)); }

NF_DEVICE u64 blamka(u64 a, u64 b) {
    u64 lo = 0xFFFFFFFFull;
    return a + b + 2ull * (a & lo) * (b & lo);
}

NF_DEVICE void G(u64 *v, u32 a, u32 b, u32 c, u32 d) {
    u64 va = v[a], vb = v[b], vc = v[c], vd = v[d];
    va = blamka(va, vb); vd = rotr64(vd ^ va, 32);
    vc = blamka(vc, vd); vb = rotr64(vb ^ vc, 24);
    va = blamka(va, vb); vd = rotr64(vd ^ va, 16);
    vc = blamka(vc, vd); vb = rotr64(vb ^ vc, 63);
    v[a] = va; v[b] = vb; v[c] = vc; v[d] = vd;
}

NF_DEVICE u32 row_ix(u32 r, u32 j) { return 16u * r + j; }
NF_DEVICE u32 col_ix(u32 c, u32 j) { return 16u * (j >> 1) + 2u * c + (j & 1u); }

NF_DEVICE void permute(u64 *R, u32 t) {
    u32 line = t >> 2, g = t & 3u;
    G(R, row_ix(line, g), row_ix(line, 4 + g), row_ix(line, 8 + g), row_ix(line, 12 + g));
    NF_SYNC();
    G(R, row_ix(line, g), row_ix(line, 4 + ((g + 1) & 3u)),
         row_ix(line, 8 + ((g + 2) & 3u)), row_ix(line, 12 + ((g + 3) & 3u)));
    NF_SYNC();
    G(R, col_ix(line, g), col_ix(line, 4 + g), col_ix(line, 8 + g), col_ix(line, 12 + g));
    NF_SYNC();
    G(R, col_ix(line, g), col_ix(line, 4 + ((g + 1) & 3u)),
         col_ix(line, 8 + ((g + 2) & 3u)), col_ix(line, 12 + ((g + 3) & 3u)));
    NF_SYNC();
}

NF_DEVICE void fill_block(u64 *P, u64 *R, const u64 *ref, u64 *next, u32 t) {
    u64 keep[4];
    for (u32 k = 0; k < 4; k++) {
        u32 w = t * 4 + k;
        u64 r = ref[w] ^ P[w];
        R[w] = r;
        keep[k] = r;
    }
    NF_SYNC();
    permute(R, t);
    for (u32 k = 0; k < 4; k++) {
        u32 w = t * 4 + k;
        u64 v = keep[k] ^ R[w];
        next[w] = v;
        P[w] = v;
    }
    NF_SYNC();
}

NF_DEVICE void next_addresses(u64 *Z, u64 *A, u64 *R, u32 t) {
    if (t == 0) { Z[6] += 1; }
    NF_SYNC();
    u64 keep[4];
    for (u32 k = 0; k < 4; k++) { u32 w = t * 4 + k; R[w] = Z[w]; keep[k] = Z[w]; }
    NF_SYNC();
    permute(R, t);
    for (u32 k = 0; k < 4; k++) { u32 w = t * 4 + k; u64 v = keep[k] ^ R[w]; A[w] = v; R[w] = v; keep[k] = v; }
    NF_SYNC();
    permute(R, t);
    for (u32 k = 0; k < 4; k++) { u32 w = t * 4 + k; A[w] = keep[k] ^ R[w]; }
    NF_SYNC();
}

// mem:  nonces * m * 128 words (device workspace)
// init: nonces * 256 words (blocks 0 and 1 per nonce, from the host)
// out:  nonces * 128 words (block m-1 per nonce, for the host)
#ifndef NF_HOST_EMULATION
extern "C" __global__ void argon2_fill(u64 *mem, const u64 *init, u64 *out, u32 m, u32 seg, u32 nonces)
#else
void argon2_fill_one(u64 *mem, const u64 *init, u64 *out, u32 m, u32 seg, u32 nonces,
                     u64 *P, u64 *R, u64 *Z, u64 *A, u32 gid_in, u32 t_in)
#endif
{
#ifndef NF_HOST_EMULATION
    NF_SHARED u64 P[128];
    NF_SHARED u64 R[128];
    NF_SHARED u64 Z[128];
    NF_SHARED u64 A[128];
    u32 gid = NF_GID;
    u32 t = NF_TID;
#else
    u32 gid = gid_in;
    u32 t = t_in;
#endif
    if (gid >= nonces) { return; }

    u64 *lane = mem + (u64)gid * (u64)m * 128ull;
    const u64 *src = init + (u64)gid * 256ull;

    for (u32 k = 0; k < 8; k++) {
        u32 w = t * 8 + k;          // 256 words, 8 per thread
        lane[w] = src[w];
    }
    for (u32 k = 0; k < 4; k++) {
        u32 w = t * 4 + k;
        P[w] = src[128 + w];        // previous block = block 1
        Z[w] = 0;
    }
    NF_SYNC();

    for (u32 slice = 0; slice < 4; slice++) {
        bool independent = slice < 2;
        u32 start = (slice == 0) ? 2u : 0u;
        if (independent) {
            if (t == 0) {
                Z[0] = 0;
                Z[1] = 0;
                Z[2] = slice;
                Z[3] = m;
                Z[4] = 1;
                Z[5] = 2;
                Z[6] = 0;
            }
            NF_SYNC();
            if (slice == 0) { next_addresses(Z, A, R, t); }
        }
        for (u32 i = start; i < seg; i++) {
            if (independent && (i % 128u) == 0) { next_addresses(Z, A, R, t); }
            u64 pseudo = independent ? A[i % 128u] : P[0];
            u64 j1 = pseudo & 0xFFFFFFFFull;
            u64 area = (slice == 0) ? (u64)(i - 1) : (u64)(slice * seg + i - 1);
            u64 x = (j1 * j1) >> 32;
            x = (area * x) >> 32;
            u64 ref_index = area - 1 - x;
            u32 cur = slice * seg + i;
            fill_block(P, R, lane + ref_index * 128ull, lane + (u64)cur * 128ull, t);
        }
    }

    const u64 *last = lane + (u64)(m - 1) * 128ull;
    u64 *dst = out + (u64)gid * 128ull;
    for (u32 k = 0; k < 4; k++) {
        u32 w = t * 4 + k;
        dst[w] = last[w];
    }
}
