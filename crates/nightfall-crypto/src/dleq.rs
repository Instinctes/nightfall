//! Cross-curve discrete-log equality: Ristretto ↔ secp256k1.
//!
//! Spec v0.2 §5.1. We do **not** use `sigma_fun`'s ed25519 leaf — that proves
//! a statement about an `EdwardsPoint` we cannot bind to our `RistrettoPoint`
//! `S_x` through dalek's public API. The leaf is ours; the combinators
//! (`Eq`, `And`, `Or`, `All`, Fiat–Shamir) stay the reviewed crate's.
//!
//! Construction is the same 252-bit Pedersen-bit proof as
//! `sigma_fun::ext::dl_secp256k1_ed25519_eq`. H is the basepoint on each
//! curve, matching xmr-btc-swap.

use crate::commit::generator_g;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar as ScalarQ;
use curve25519_dalek::traits::Identity;
use digest::Update;
use generic_array::{
    typenum::{self, type_operators::IsLessOrEqual, U252, U31},
    ArrayLength, GenericArray,
};
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sigma_fun::secp256k1::fun::marker::*;
use sigma_fun::secp256k1::fun::rand_core::{CryptoRng, RngCore};
use sigma_fun::secp256k1::fun::{g, s, Point as PointP, Scalar as ScalarP, G as GP};
use sigma_fun::{secp256k1, All, And, Either, Eq, FiatShamir, HashTranscript, Or, Sigma, Writable};
use std::marker::PhantomData;

const COMMITMENT_BITS: usize = 252;

type Transcript = HashTranscript<Sha256, ChaCha20Rng>;

/// `And` of 252 bit-ORs, then a pair of same-curve DLEQs.
pub type CoreProof = And<
    All<Or<And<secp256k1::DLG<U31>, DLG<U31>>, And<secp256k1::DLG<U31>, DLG<U31>>>, U252>,
    And<Eq<secp256k1::DLG<U31>, secp256k1::DL<U31>>, Eq<DLG<U31>, DL<U31>>>,
>;

/// Proves `A = x · B` for Ristretto points `(B, A)` in the statement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DL<L> {
    _len: PhantomData<L>,
}

impl<L: ArrayLength<u8>> Sigma for DL<L>
where
    L: IsLessOrEqual<U31>,
    <L as IsLessOrEqual<U31>>::Output: typenum::marker_traits::NonZero,
{
    type Witness = ScalarQ;
    type Statement = (RistrettoPoint, RistrettoPoint);
    type AnnounceSecret = ScalarQ;
    type Announcement = RistrettoPoint;
    type Response = ScalarQ;
    type ChallengeLength = L;

    fn respond(
        &self,
        witness: &Self::Witness,
        _statement: &Self::Statement,
        announce_secret: Self::AnnounceSecret,
        _announce: &Self::Announcement,
        challenge: &GenericArray<u8, Self::ChallengeLength>,
    ) -> Self::Response {
        let e = challenge_scalar(challenge);
        announce_secret + e * witness
    }

    fn announce(
        &self,
        statement: &Self::Statement,
        announce_secret: &Self::AnnounceSecret,
    ) -> Self::Announcement {
        announce_secret * statement.0
    }

    fn gen_announce_secret<Rng: CryptoRng + RngCore>(
        &self,
        _witness: &Self::Witness,
        rng: &mut Rng,
    ) -> Self::AnnounceSecret {
        ScalarQ::random(rng)
    }

    fn sample_response<Rng: CryptoRng + RngCore>(&self, rng: &mut Rng) -> Self::Response {
        ScalarQ::random(rng)
    }

    fn implied_announcement(
        &self,
        statement: &Self::Statement,
        challenge: &GenericArray<u8, Self::ChallengeLength>,
        response: &Self::Response,
    ) -> Option<Self::Announcement> {
        let (g, x) = statement;
        let e = challenge_scalar(challenge);
        Some(response * g - e * x)
    }

    fn hash_statement<H: Update>(&self, hash: &mut H, statement: &Self::Statement) {
        hash.update(statement.0.compress().as_bytes());
        hash.update(statement.1.compress().as_bytes());
    }

    fn hash_announcement<H: Update>(&self, hash: &mut H, announcement: &Self::Announcement) {
        hash.update(announcement.compress().as_bytes());
    }

    fn hash_witness<H: Update>(&self, hash: &mut H, witness: &Self::Witness) {
        hash.update(witness.as_bytes());
    }
}

/// Proves `A = x · G` for the Ristretto basepoint. `G` is omitted from the statement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DLG<L> {
    _len: PhantomData<L>,
}

impl<L: ArrayLength<u8>> Sigma for DLG<L>
where
    L: IsLessOrEqual<U31>,
    <L as IsLessOrEqual<U31>>::Output: typenum::marker_traits::NonZero,
{
    type Witness = ScalarQ;
    type Statement = RistrettoPoint;
    type AnnounceSecret = ScalarQ;
    type Announcement = RistrettoPoint;
    type Response = ScalarQ;
    type ChallengeLength = L;

    fn respond(
        &self,
        witness: &Self::Witness,
        _statement: &Self::Statement,
        announce_secret: Self::AnnounceSecret,
        _announce: &Self::Announcement,
        challenge: &GenericArray<u8, Self::ChallengeLength>,
    ) -> Self::Response {
        let e = challenge_scalar(challenge);
        announce_secret + e * witness
    }

    fn announce(
        &self,
        _statement: &Self::Statement,
        announce_secret: &Self::AnnounceSecret,
    ) -> Self::Announcement {
        announce_secret * RISTRETTO_BASEPOINT_POINT
    }

    fn gen_announce_secret<Rng: CryptoRng + RngCore>(
        &self,
        _witness: &Self::Witness,
        rng: &mut Rng,
    ) -> Self::AnnounceSecret {
        ScalarQ::random(rng)
    }

    fn sample_response<Rng: CryptoRng + RngCore>(&self, rng: &mut Rng) -> Self::Response {
        ScalarQ::random(rng)
    }

    fn implied_announcement(
        &self,
        statement: &Self::Statement,
        challenge: &GenericArray<u8, Self::ChallengeLength>,
        response: &Self::Response,
    ) -> Option<Self::Announcement> {
        let e = challenge_scalar(challenge);
        Some(RistrettoPoint::vartime_double_scalar_mul_basepoint(
            &-e, statement, response,
        ))
    }

    fn hash_statement<H: Update>(&self, hash: &mut H, statement: &Self::Statement) {
        hash.update(statement.compress().as_bytes());
    }

    fn hash_announcement<H: Update>(&self, hash: &mut H, announcement: &Self::Announcement) {
        hash.update(announcement.compress().as_bytes());
    }

    fn hash_witness<H: Update>(&self, hash: &mut H, witness: &Self::Witness) {
        hash.update(witness.as_bytes());
    }
}

impl<L> Writable for DL<L> {
    fn write_to<W: core::fmt::Write>(&self, w: &mut W) -> core::fmt::Result {
        write!(w, "DL(ristretto)")
    }
}

impl<L> Writable for DLG<L> {
    fn write_to<W: core::fmt::Write>(&self, w: &mut W) -> core::fmt::Result {
        write!(w, "DLG(ristretto)")
    }
}

fn challenge_scalar<L: ArrayLength<u8>>(challenge: &GenericArray<u8, L>) -> ScalarQ {
    let mut bytes = [0u8; 32];
    bytes[..challenge.len()].copy_from_slice(challenge.as_slice());
    Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(bytes))
        .expect("U31 challenge is canonical")
}

/// Wire form. Points are compressed so we never serde a dalek type.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DleqProof {
    sum_blinding_secp: [u8; 32],
    sum_blinding_ristretto: [u8; 32],
    /// secp256k1 compressed (33 bytes) then Ristretto compressed (32 bytes).
    commitments: Vec<(Vec<u8>, [u8; 32])>,
    /// bincode of sigma_fun's compact proof. Opaque on purpose.
    core: Vec<u8>,
}

#[derive(Clone)]
struct System {
    core: FiatShamir<CoreProof, Transcript>,
    powers_of_two: Vec<(PointP, RistrettoPoint)>,
}

fn system() -> &'static System {
    use std::sync::OnceLock;
    static S: OnceLock<System> = OnceLock::new();
    S.get_or_init(|| {
        let hp = (*GP).normalize();
        let hq = RISTRETTO_BASEPOINT_POINT;
        let powers = core::iter::successors(Some((hp, hq)), |(h2p, h2q)| {
            Some((
                g!(h2p + h2p)
                    .normalize()
                    .non_zero()
                    .expect("2^i * G never identity"),
                h2q + h2q,
            ))
        })
        .take(COMMITMENT_BITS)
        .collect();
        System {
            core: FiatShamir::<CoreProof, Transcript>::default(),
            powers_of_two: powers,
        }
    })
}

fn secp_scalar_from_ristretto(secret: &ScalarQ) -> ScalarP {
    let mut bytes = secret.to_bytes();
    bytes.reverse();
    ScalarP::from_bytes(bytes)
        .expect("ristretto order < secp order")
        .non_zero()
        .expect("non-zero")
}

fn to_bits(secret: &ScalarQ) -> [bool; COMMITMENT_BITS] {
    let bytes = secret.as_bytes();
    let mut bits = [false; COMMITMENT_BITS];
    for (i, byte) in bytes.iter().enumerate() {
        for j in 0..8 {
            let index = i * 8 + j;
            if index >= COMMITMENT_BITS {
                return bits;
            }
            bits[index] = (byte & (1 << j)) != 0;
        }
    }
    bits
}

/// Prove `S = s·G_ristretto` and `T = s·G_secp` share `s`.
///
/// Panics if `s` is zero or ≥ 2^252 — [`crate::swap::SwapShare::generate`]
/// already samples under that bound.
pub fn prove(
    secret: &ScalarQ,
    rng: &mut (impl CryptoRng + RngCore),
) -> (DleqProof, RistrettoPoint, PointP) {
    assert_eq!(
        secret.as_bytes()[31] & 0b0001_0000,
        0,
        "scalar must be below 2^252"
    );
    assert_ne!(*secret, ScalarQ::ZERO);

    let sys = system();
    let secp_secret = secp_scalar_from_ristretto(secret);
    let claim_p = g!(secp_secret * GP).normalize();
    let claim_q = secret * RISTRETTO_BASEPOINT_POINT;
    debug_assert_eq!(claim_q, secret * generator_g());

    let blinds: Vec<(ScalarP, ScalarQ)> = (0..COMMITMENT_BITS)
        .map(|_| (ScalarP::random(rng), ScalarQ::random(rng)))
        .collect();

    let sum_b = blinds
        .iter()
        .fold((ScalarP::zero(), ScalarQ::ZERO), |(ap, aq), (rp, rq)| {
            (s!(ap + rp), aq + rq)
        });
    let sum_b = (sum_b.0.public(), sum_b.1);
    let bits = to_bits(secret);

    let commitments: Vec<(PointP, RistrettoPoint)> = sys
        .powers_of_two
        .iter()
        .zip(bits.iter())
        .zip(blinds.iter())
        .map(|(((h2p, h2q), bit), (rp, rq))| {
            let zero_p = g!(rp * GP).normalize();
            let one_p = g!(zero_p + h2p)
                .normalize()
                .non_zero()
                .expect("random + H");
            let zero_q = rq * RISTRETTO_BASEPOINT_POINT;
            let one_q = zero_q + h2q;
            let b = *bit as u8;
            (
                <PointP as sigma_fun::secp256k1::fun::subtle::ConditionallySelectable>::conditional_select(
                    &zero_p,
                    &one_p,
                    sigma_fun::secp256k1::fun::subtle::Choice::from(b),
                ),
                <RistrettoPoint as subtle::ConditionallySelectable>::conditional_select(
                    &zero_q,
                    &one_q,
                    subtle::Choice::from(b),
                ),
            )
        })
        .collect();

    let statement = generate_statement(&sum_b, &(claim_p, claim_q), &commitments)
        .expect("prover statement is valid");

    let witness = (
        blinds
            .into_iter()
            .zip(bits.iter())
            .map(|((rp, rq), bit)| match bit {
                false => Either::Left((rp, rq)),
                true => Either::Right((rp, rq)),
            })
            .collect(),
        (secp_secret, *secret),
    );

    let core = sys.core.prove(&witness, &statement, Some(rng));
    let core_bytes = bincode_proof(&core);

    let proof = DleqProof {
        sum_blinding_secp: sum_b.0.to_bytes(),
        sum_blinding_ristretto: sum_b.1.to_bytes(),
        commitments: commitments
            .iter()
            .map(|(p, q)| (p.to_bytes().to_vec(), q.compress().to_bytes()))
            .collect(),
        core: core_bytes,
    };
    (proof, claim_q, claim_p)
}

/// Verify that `s_bytes` (Ristretto compressed) and `t_bytes` (secp compressed)
/// have the same 252-bit discrete log.
pub fn verify(proof: &DleqProof, s_bytes: &[u8; 32], t_bytes: &[u8; 33]) -> bool {
    if proof.commitments.len() != COMMITMENT_BITS {
        return false;
    }
    let Some(claim_q) = curve25519_dalek::ristretto::CompressedRistretto(*s_bytes).decompress()
    else {
        return false;
    };
    let Some(claim_p) = PointP::from_bytes(*t_bytes) else {
        return false;
    };
    let Some(r_p) = ScalarP::<Public, Zero>::from_bytes(proof.sum_blinding_secp) else {
        return false;
    };
    let Some(r_q) =
        Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(proof.sum_blinding_ristretto))
    else {
        return false;
    };

    let mut commits = Vec::with_capacity(COMMITMENT_BITS);
    for (p_b, q_b) in &proof.commitments {
        if p_b.len() != 33 {
            return false;
        }
        let mut p33 = [0u8; 33];
        p33.copy_from_slice(p_b);
        let Some(p) = PointP::from_bytes(p33) else {
            return false;
        };
        let Some(q) = curve25519_dalek::ristretto::CompressedRistretto(*q_b).decompress() else {
            return false;
        };
        commits.push((p, q));
    }

    let Some(statement) = generate_statement(&(r_p, r_q), &(claim_p, claim_q), &commits) else {
        return false;
    };
    let Some(core) = unbincode_proof(&proof.core) else {
        return false;
    };
    system().core.verify(&statement, &core)
}

fn generate_statement(
    (rp, rq): &(ScalarP<Public, Zero>, ScalarQ),
    (xp, xq): &(PointP, RistrettoPoint),
    commitments: &[(PointP, RistrettoPoint)],
) -> Option<<CoreProof as Sigma>::Statement> {
    let sys = system();
    let gq = RISTRETTO_BASEPOINT_POINT;
    let commitment_statement = sys
        .powers_of_two
        .iter()
        .zip(commitments)
        .map(|((h2p, h2q), (cp, cq))| {
            g!(cp - h2p)
                .normalize()
                .non_zero()
                .map(|cp_sub| ((*cp, *cq), (cp_sub, cq - h2q)))
        })
        .collect::<Option<Vec<_>>>()?;

    let (sum_p, sum_q) = commitments.iter().fold(
        (PointP::zero(), RistrettoPoint::identity()),
        |(ap, aq), (cp, cq)| (g!(ap + cp), aq + cq),
    );
    let unblinded_p = g!(sum_p - rp * GP).normalize().non_zero()?;
    let unblinded_q = sum_q - rq * gq;
    let dleq = (
        (*xp, ((*GP).normalize(), unblinded_p)),
        (*xq, (gq, unblinded_q)),
    );
    Some((commitment_statement, dleq))
}

fn bincode_proof(
    proof: &sigma_fun::CompactProof<
        <CoreProof as Sigma>::Response,
        <CoreProof as Sigma>::ChallengeLength,
    >,
) -> Vec<u8> {
    serde_json::to_vec(proof).expect("compact proof serializes")
}

fn unbincode_proof(
    bytes: &[u8],
) -> Option<
    sigma_fun::CompactProof<<CoreProof as Sigma>::Response, <CoreProof as Sigma>::ChallengeLength>,
> {
    serde_json::from_slice(bytes).ok()
}

/// Compressed secp256k1 encoding of `s · G_secp`.
pub fn secp_point_bytes(secret: &ScalarQ) -> [u8; 33] {
    secp_scalar_from_ristretto(secret);
    let p = g!(secp_scalar_from_ristretto(secret) * GP).normalize();
    p.to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swap::SwapShare;
    use rand::rngs::OsRng;

    #[test]
    fn honest_share_verifies() {
        let share = SwapShare::generate();
        let mut rng = OsRng;
        let (proof, s, t) = prove(&share.secret(), &mut rng);
        assert!(verify(&proof, &s.compress().to_bytes(), &t.to_bytes()));
        assert_eq!(s, share.public());
    }

    #[test]
    fn wrong_claim_is_rejected() {
        let a = SwapShare::generate();
        let b = SwapShare::generate();
        let mut rng = OsRng;
        let (proof, _, t) = prove(&a.secret(), &mut rng);
        assert!(!verify(
            &proof,
            &b.public().compress().to_bytes(),
            &t.to_bytes()
        ));
    }

    #[test]
    fn generator_g_is_the_basepoint_the_proof_uses() {
        assert_eq!(generator_g(), RISTRETTO_BASEPOINT_POINT);
    }
}

/// Adversarial tests. The honest-path test only shows the prover and verifier
/// agree; these ask whether the proof binds anything.
///
/// A cross-curve DLEQ that accepts mismatched scalars is worse than no proof:
/// it lets one party put a key on Bitcoin that has nothing to do with the key
/// on NIGHT, take the coins, and never reveal a usable secret.
#[cfg(test)]
mod binding_tests {
    use super::*;
    use rand::rngs::OsRng;

    fn small_secret() -> ScalarQ {
        let mut b = [0u8; 32];
        OsRng.fill_bytes(&mut b);
        b[31] &= 0x0F;
        Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(b)).expect("canonical")
    }

    /// The whole point of the proof.
    #[test]
    fn a_proof_does_not_verify_against_a_foreign_secp_point() {
        let s = small_secret();
        let (proof, ris, _secp) = prove(&s, &mut OsRng);

        let other = small_secret();
        let foreign_t = secp_point_bytes(&other);

        assert!(
            !verify(&proof, &ris.compress().to_bytes(), &foreign_t),
            "S and T from different scalars must not verify — this is the \
             attack the proof exists to stop"
        );
    }

    /// Same, from the other side.
    #[test]
    fn a_proof_does_not_verify_against_a_foreign_ristretto_point() {
        let s = small_secret();
        let (proof, _ris, secp) = prove(&s, &mut OsRng);

        let other = small_secret();
        let foreign_s = (other * RISTRETTO_BASEPOINT_POINT).compress().to_bytes();

        assert!(
            !verify(&proof, &foreign_s, &secp.to_bytes()),
            "a Ristretto point with a different discrete log must not verify"
        );
    }

    /// A proof for one key must not be reusable for another.
    #[test]
    fn proofs_are_not_transferable_between_shares() {
        let s1 = small_secret();
        let s2 = small_secret();
        let (proof1, _r1, _t1) = prove(&s1, &mut OsRng);
        let (_proof2, r2, t2) = prove(&s2, &mut OsRng);

        assert!(
            !verify(&proof1, &r2.compress().to_bytes(), &t2.to_bytes()),
            "proof for one share must not verify a different share"
        );
    }

    /// Tampering with any commitment must break it.
    #[test]
    fn a_mangled_commitment_breaks_the_proof() {
        let s = small_secret();
        let (mut proof, ris, secp) = prove(&s, &mut OsRng);
        assert!(verify(&proof, &ris.compress().to_bytes(), &secp.to_bytes()));

        proof.commitments[7].1[0] ^= 0xFF;
        assert!(
            !verify(&proof, &ris.compress().to_bytes(), &secp.to_bytes()),
            "a flipped commitment byte must invalidate the proof"
        );
    }

    /// And the sum of blindings is load-bearing too.
    #[test]
    fn a_mangled_blinding_sum_breaks_the_proof() {
        let s = small_secret();
        let (mut proof, ris, secp) = prove(&s, &mut OsRng);
        proof.sum_blinding_ristretto[0] ^= 0x01;
        assert!(
            !verify(&proof, &ris.compress().to_bytes(), &secp.to_bytes()),
            "a flipped blinding-sum byte must invalidate the proof"
        );
    }

    /// The endianness claim in the report, tested rather than trusted:
    /// the secp scalar must be the same integer as the Ristretto one.
    #[test]
    fn the_two_curves_see_the_same_integer() {
        let s = small_secret();
        let (_proof, ris, secp) = prove(&s, &mut OsRng);

        // Recompute the secp point independently from the little-endian bytes.
        let expected = secp_point_bytes(&s);
        assert_eq!(secp.to_bytes(), expected);

        // And the Ristretto side is the same scalar on our own generator.
        assert_eq!(ris, s * generator_g());
    }
}

/// The bit decomposition feeds every Pedersen commitment in the proof. If it
/// were wrong the proof would simply fail to verify rather than prove something
/// false — but a silent off-by-one here would be very hard to read back out of
/// a failing proof, so it is pinned directly.
#[cfg(test)]
mod bit_tests {
    use super::*;

    fn from_bits(bits: &[bool; COMMITMENT_BITS]) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, bit) in bits.iter().enumerate() {
            if *bit {
                out[i / 8] |= 1 << (i % 8);
            }
        }
        out
    }

    #[test]
    fn bits_round_trip_for_a_252_bit_scalar() {
        let mut b = [0u8; 32];
        for (i, v) in b.iter_mut().enumerate() {
            *v = (i as u8).wrapping_mul(37).wrapping_add(11);
        }
        b[31] &= 0x0F;
        let s = Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(b)).unwrap();
        assert_eq!(from_bits(&to_bits(&s)), b, "decomposition must be lossless");
    }

    #[test]
    fn the_top_four_bits_are_never_used() {
        let mut b = [0u8; 32];
        b[31] = 0x0F;
        let s = Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(b)).unwrap();
        let bits = to_bits(&s);
        assert_eq!(bits.len(), 252);
        assert!(bits[248] && bits[249] && bits[250] && bits[251]);
    }
}

/// The combinators only work if the leaf's Schnorr equation is the one they
/// expect: implied_announcement = response·G − e·X. A wrong sign here
/// verifies against a different statement than announce produced.
#[cfg(test)]
mod leaf_tests {
    use super::*;
    use rand::rngs::OsRng;
    use sigma_fun::FiatShamir;

    fn small_secret() -> ScalarQ {
        let mut b = [0u8; 32];
        OsRng.fill_bytes(&mut b);
        b[31] &= 0x0F;
        Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(b)).unwrap()
    }

    #[test]
    fn dlg_leaf_verifies_in_isolation() {
        let x = small_secret();
        let xg = x * RISTRETTO_BASEPOINT_POINT;
        let sys = FiatShamir::<DLG<U31>, Transcript>::default();
        let proof = sys.prove(&x, &xg, Some(&mut OsRng));
        assert!(sys.verify(&xg, &proof));
        let other = small_secret() * RISTRETTO_BASEPOINT_POINT;
        assert!(!sys.verify(&other, &proof), "leaf must bind the statement");
    }

    #[test]
    fn dl_leaf_verifies_in_isolation() {
        let x = small_secret();
        let h = ScalarQ::random(&mut OsRng) * RISTRETTO_BASEPOINT_POINT;
        let xh = x * h;
        let sys = FiatShamir::<DL<U31>, Transcript>::default();
        let proof = sys.prove(&x, &(h, xh), Some(&mut OsRng));
        assert!(sys.verify(&(h, xh), &proof));
        assert!(!sys.verify(&(h, h), &proof));
    }
}

/// What an external review of this leaf would start with, written as tests.
/// This is weaker than a person: it cannot invent a new attack. It can
/// refuse the attacks we already know, on random secrets, and refuse
/// garbage from the wire.
#[cfg(test)]
mod hardening_tests {
    use super::*;
    use rand::rngs::OsRng;
    use rand::RngCore;
    use rand::SeedableRng;

    fn small_secret() -> ScalarQ {
        let mut b = [0u8; 32];
        OsRng.fill_bytes(&mut b);
        b[31] &= 0x0F;
        Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(b)).unwrap()
    }

    /// Completeness on several independently sampled secrets.
    #[test]
    fn random_honest_proofs_verify() {
        for _ in 0..4 {
            let s = small_secret();
            let (proof, ris, secp) = prove(&s, &mut OsRng);
            assert!(
                verify(&proof, &ris.compress().to_bytes(), &secp.to_bytes()),
                "an honest proof must verify"
            );
        }
    }

    /// Same seed, same proof. A hidden extra RNG draw would move this.
    #[test]
    fn a_seeded_prover_is_deterministic() {
        let mut secret_bytes = [0u8; 32];
        secret_bytes[0] = 7;
        secret_bytes[31] = 0x0E;
        let secret = Option::<ScalarQ>::from(ScalarQ::from_canonical_bytes(secret_bytes)).unwrap();
        let mut a = ChaCha20Rng::from_seed([9u8; 32]);
        let mut b = ChaCha20Rng::from_seed([9u8; 32]);
        let (p1, s1, t1) = prove(&secret, &mut a);
        let (p2, s2, t2) = prove(&secret, &mut b);
        assert_eq!(p1, p2);
        assert_eq!(s1, s2);
        assert_eq!(t1.to_bytes(), t2.to_bytes());
        assert!(verify(&p1, &s1.compress().to_bytes(), &t1.to_bytes()));
    }

    /// Flipping a byte of the opaque core must not verify. A proof from
    /// the network is attacker-controlled.
    #[test]
    fn garbage_core_bytes_do_not_verify() {
        let s = small_secret();
        let (mut proof, ris, secp) = prove(&s, &mut OsRng);
        let s_b = ris.compress().to_bytes();
        let t_b = secp.to_bytes();
        assert!(verify(&proof, &s_b, &t_b));
        assert!(!proof.core.is_empty());
        let i = proof.core.len() / 2;
        proof.core[i] ^= 0x5A;
        assert!(
            !verify(&proof, &s_b, &t_b),
            "a flipped core byte must fail closed"
        );
    }

    #[test]
    fn random_bytes_are_not_a_proof() {
        let s = small_secret();
        let (_, ris, secp) = prove(&s, &mut OsRng);
        let s_b = ris.compress().to_bytes();
        let t_b = secp.to_bytes();
        for _ in 0..16 {
            let mut core = vec![0u8; 64];
            OsRng.fill_bytes(&mut core);
            let fake = DleqProof {
                sum_blinding_secp: [0u8; 32],
                sum_blinding_ristretto: [0u8; 32],
                commitments: vec![([0u8; 33].to_vec(), [0u8; 32]); COMMITMENT_BITS],
                core,
            };
            assert!(!verify(&fake, &s_b, &t_b));
        }
    }
}
