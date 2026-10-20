//! Cut-through-compatible one-sided spend-authorization primitives.
//!
//! This module deliberately does NOT alter the consensus input format yet.
//! The current Ko signature remains authoritative until the stealth-excess
//! equation, offsets, retention horizon and storage/replay paths are integrated.
//!
//! The construction follows the LIP-0004 input-key relation:
//!
//! `K_input = Ki + H(Ki || Ko) * Ko`
//!
//! where:
//!
//! - Ki is a fresh ephemeral input key,
//! - Ko is the canonical one-time output key from the UTXO being spent,
//! - the signer knows both corresponding secret scalars.
//!
//! Consensus integration must additionally enforce the stealth balance.
//! This module alone is NOT sufficient to replace the existing Ko authorization.

use curve25519_dalek::{
    ristretto::{CompressedRistretto, RistrettoPoint},
    scalar::Scalar,
    traits::Identity,
};
use nightfall_crypto::{generator_g, hash_multi, sig, Commitment, SchnorrSig};

use serde::{Deserialize, Serialize};

use nightfall_crypto::{generator_h, TxKernel};

use crate::tx::Transaction;

/// Domain separator for the candidate cut-through input authorization.
///
/// This primitive is not consensus-active yet. Once activated, changing this
/// value would be a consensus change.
pub const INPUT_AUTH_CHALLENGE_DOMAIN: &[u8] = b"nightfall:cutthrough:input-challenge:v1";

/// Domain for the independent sender-authorization proof used by the
/// cut-through candidate design.
pub const SENDER_AUTH_DOMAIN: &[u8] = b"nightfall:cutthrough:sender-auth:v1";

/// Independent sender authorization key `Ks`.
///
/// Nightfall's existing output `ephemeral_pk` is the key-exchange key `Ke`.
/// This candidate deliberately does not assume that `Ke` and `Ks` may safely
/// share the same secret.
///
/// This structure is prototype-only and is not part of the active Output wire
/// format or consensus rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderAuthorizationV1 {
    /// Independent sender public key `Ks = ks*G`.
    pub key: [u8; 32],

    /// Proof of knowledge of `ks`, bound to the complete output data and Ks.
    pub signature: SchnorrSig,
}

impl SenderAuthorizationV1 {
    /// Construct an independent sender authorization for an existing output.
    pub fn sign(output: &nightfall_crypto::Output, sender_secret: &Scalar) -> Self {
        let key = (generator_g() * *sender_secret).compress().to_bytes();

        let msg = sender_authorization_message(output, &key);

        let signature = sig::sign(sender_secret, &generator_g(), &msg);

        Self { key, signature }
    }

    /// Verify sender knowledge and binding to the complete output.
    pub fn verify(&self, output: &nightfall_crypto::Output) -> bool {
        let Some(public) = CompressedRistretto(self.key).decompress() else {
            return false;
        };

        let msg = sender_authorization_message(output, &self.key);

        sig::verify(&public, &generator_g(), &msg, &self.signature)
    }

    /// `Ks || R || s`.
    pub fn canonical_bytes(&self) -> [u8; 96] {
        let mut out = [0u8; 96];

        out[..32].copy_from_slice(&self.key);
        out[32..64].copy_from_slice(&self.signature.r);
        out[64..96].copy_from_slice(&self.signature.s);

        out
    }
}

/// Sender authorization transcript.
///
/// The output's existing `commitment_bytes()` already contains:
///
/// * feature
/// * commitment
/// * Ke
/// * Ko
/// * view tag
/// * range proof
/// * encrypted payload
///
/// Ks itself is appended explicitly, so replacing the sender authorization
/// key changes the signed transcript.
pub fn sender_authorization_message(
    output: &nightfall_crypto::Output,
    sender_key: &[u8; 32],
) -> Vec<u8> {
    let output_bytes = output.commitment_bytes();

    hash_multi(SENDER_AUTH_DOMAIN, &[&output_bytes, sender_key])
        .0
        .to_vec()
}

/// Fixed-size candidate representation of cut-through-compatible input
/// authorization.
///
/// `Ko` is deliberately absent. The verifier must obtain canonical `Ko` from
/// trusted ledger state rather than accepting attacker-supplied replacement
/// metadata.
///
/// This type is serializable for prototype work but is NOT consensus-active
/// and is not yet part of [`crate::Input`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendAuthorizationV1 {
    /// Fresh ephemeral input public key `Ki = ki*G`.
    pub ki: [u8; 32],

    /// Schnorr authorization under `Ki + H(Ki || Ko) * Ko`.
    pub signature: SchnorrSig,
}

impl SpendAuthorizationV1 {
    pub fn sign(
        commit: &Commitment,
        ki_secret: &Scalar,
        ko_secret: &Scalar,
        canonical_ko: &[u8; 32],
    ) -> Option<Self> {
        let (ki, signature) = sign_input_authorization(commit, ki_secret, ko_secret, canonical_ko)?;

        Some(Self { ki, signature })
    }

    /// `canonical_ko` must come from authoritative UTXO state.
    pub fn verify(&self, commit: &Commitment, canonical_ko: &[u8; 32]) -> bool {
        verify_input_authorization(commit, &self.ki, &self.signature, canonical_ko)
    }

    /// Fixed representation: `Ki || R || s`.
    ///
    /// Exactly 96 bytes.
    pub fn canonical_bytes(&self) -> [u8; 96] {
        let mut out = [0u8; 96];

        out[..32].copy_from_slice(&self.ki);
        out[32..64].copy_from_slice(&self.signature.r);
        out[64..96].copy_from_slice(&self.signature.s);

        out
    }
}

/// Fixed-size representation of one stealth excess `E'`.
///
/// `Commitment` is reused solely as Nightfall's existing compressed
/// Ristretto-point container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StealthExcessV1 {
    pub point: Commitment,
}

impl StealthExcessV1 {
    pub fn new(point: Commitment) -> Option<Self> {
        point.point()?;
        Some(Self { point })
    }

    pub fn is_well_formed(&self) -> bool {
        self.point.point().is_some()
    }

    pub fn canonical_bytes(&self) -> [u8; 32] {
        self.point.0
    }
}

/// Canonically encoded aggregate stealth offset `x'`.
///
/// The scalar is retained as its canonical 32-byte representation so malformed
/// encodings are rejected instead of silently reduced modulo the group order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StealthOffsetV1 {
    pub bytes: [u8; 32],
}

impl StealthOffsetV1 {
    pub fn from_scalar(value: &Scalar) -> Self {
        Self {
            bytes: value.to_bytes(),
        }
    }

    pub fn scalar(&self) -> Option<Scalar> {
        Option::<Scalar>::from(Scalar::from_canonical_bytes(self.bytes))
    }

    pub fn is_canonical(&self) -> bool {
        self.scalar().is_some()
    }

    /// Aggregate offsets as field elements:
    ///
    /// `x'_agg = x'_1 + x'_2 mod p`.
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        let left = self.scalar()?;
        let right = other.scalar()?;

        Some(Self::from_scalar(&(left + right)))
    }

    pub fn canonical_bytes(&self) -> [u8; 32] {
        self.bytes
    }
}

/// Domain separator for binding a Nightfall value kernel to one stealth
/// excess `E'`.
pub const KERNEL_STEALTH_BIND_DOMAIN: &[u8] = b"nightfall:cutthrough:kernel-stealth-bind:v1";

/// Prototype proof that a specific Nightfall value kernel authorizes a
/// specific stealth excess `E'`.
///
/// Nightfall's value kernel excess is a discrete log with respect to `H`,
/// whereas stealth keys and `E'` live with respect to `G`. Therefore this
/// prototype does not assume that the two points can be collapsed into one
/// ordinary Schnorr verification key.
///
/// Instead, the holder of the existing kernel excess secret explicitly signs
/// the complete existing kernel plus `E'` under generator `H`.
///
/// This is not consensus-active yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelStealthBindingV1 {
    /// The exact stealth excess authorized by this kernel.
    pub stealth_excess: StealthExcessV1,

    /// Proof under the kernel excess key, using generator H.
    pub signature: SchnorrSig,
}

impl KernelStealthBindingV1 {
    /// Construct a binding only when `kernel_excess_secret` really opens the
    /// canonical kernel excess and both the existing kernel and E' are valid.
    pub fn sign(
        kernel: &TxKernel,
        kernel_excess_secret: &Scalar,
        stealth_excess: StealthExcessV1,
    ) -> Option<Self> {
        if !stealth_excess.is_well_formed() {
            return None;
        }

        if kernel.check_shape().is_err() || !kernel.verify_signature() {
            return None;
        }

        let kernel_excess_point = kernel.excess.point()?;

        if generator_h() * *kernel_excess_secret != kernel_excess_point {
            return None;
        }

        let msg = kernel_stealth_binding_message(kernel, &stealth_excess);

        let signature = sig::sign(kernel_excess_secret, &generator_h(), &msg);

        Some(Self {
            stealth_excess,
            signature,
        })
    }

    /// Verify the existing value kernel and its authorization of E'.
    pub fn verify(&self, kernel: &TxKernel) -> bool {
        if !self.stealth_excess.is_well_formed() {
            return false;
        }

        if kernel.check_shape().is_err() || !kernel.verify_signature() {
            return false;
        }

        let Some(kernel_excess_point) = kernel.excess.point() else {
            return false;
        };

        let msg = kernel_stealth_binding_message(kernel, &self.stealth_excess);

        sig::verify(&kernel_excess_point, &generator_h(), &msg, &self.signature)
    }

    /// Fixed representation: `E' || R || s`.
    pub fn canonical_bytes(&self) -> [u8; 96] {
        let mut out = [0u8; 96];

        out[..32].copy_from_slice(&self.stealth_excess.point.0);
        out[32..64].copy_from_slice(&self.signature.r);
        out[64..96].copy_from_slice(&self.signature.s);

        out
    }
}

/// Canonical transcript binding the entire current Nightfall kernel object to
/// one stealth excess.
///
/// `TxKernel::signing_message()` already binds:
///
/// * feature
/// * fee
/// * reward
/// * lock height
/// * value excess E
///
/// The existing kernel signature bytes are included too, so this proof binds
/// the concrete kernel rather than merely another object with identical
/// unsigned fields.
pub fn kernel_stealth_binding_message(
    kernel: &TxKernel,
    stealth_excess: &StealthExcessV1,
) -> Vec<u8> {
    let kernel_message = kernel.signing_message();

    hash_multi(
        KERNEL_STEALTH_BIND_DOMAIN,
        &[
            &kernel_message,
            &kernel.excess_sig.r,
            &kernel.excess_sig.s,
            &stealth_excess.point.0,
        ],
    )
    .0
    .to_vec()
}

/// Kernel-bound version of the cut-through authorization bundle.
///
/// Unlike [`AuthorizationBundleV1`], stealth excesses cannot exist here as
/// loose points: every `E'` is carried inside a [`KernelStealthBindingV1`].
///
/// The `kernels` slice passed to [`Self::validate`] is positionally paired
/// with `kernel_bindings`. This is still prototype-only and is deliberately
/// not part of the active transaction or block format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelBoundAuthorizationBundleV1 {
    pub sender_authorizations: Vec<SenderAuthorizationV1>,
    pub input_authorizations: Vec<SpendAuthorizationV1>,

    /// Every retained stealth excess must be authorized by a concrete value
    /// kernel.
    pub kernel_bindings: Vec<KernelStealthBindingV1>,

    pub stealth_offset: StealthOffsetV1,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KernelBoundAuthorizationBundleError {
    #[error("kernel binding count mismatch: got {got}, expected {expected}")]
    KernelBindingCountMismatch { got: usize, expected: usize },

    #[error("duplicate kernel binding at index {index}")]
    DuplicateKernelBinding { index: usize },

    #[error("invalid kernel/stealth binding at index {index}")]
    InvalidKernelStealthBinding { index: usize },

    #[error("malformed stealth excess at index {index}")]
    MalformedBoundStealthExcess { index: usize },

    #[error("non-canonical stealth offset")]
    NonCanonicalStealthOffset,

    #[error("authorization bundle validation failed: {0}")]
    Authorization(AuthorizationBundleError),
}

impl KernelBoundAuthorizationBundleV1 {
    /// Validate material that does not require transaction or kernel context.
    fn validate_intrinsic(&self) -> Result<Scalar, KernelBoundAuthorizationBundleError> {
        for (index, binding) in self.kernel_bindings.iter().enumerate() {
            if !binding.stealth_excess.is_well_formed() {
                return Err(
                    KernelBoundAuthorizationBundleError::MalformedBoundStealthExcess { index },
                );
            }
        }

        self.stealth_offset
            .scalar()
            .ok_or(KernelBoundAuthorizationBundleError::NonCanonicalStealthOffset)
    }

    /// Validate the complete kernel-bound authorization package.
    ///
    /// `canonical_kos` must come from authoritative UTXO state.
    ///
    /// `kernels[index]` is the exact kernel authorized by
    /// `kernel_bindings[index]`.
    pub fn validate(
        &self,
        outputs: &[nightfall_crypto::Output],
        input_commits: &[Commitment],
        canonical_kos: &[[u8; 32]],
        kernels: &[TxKernel],
    ) -> Result<(), KernelBoundAuthorizationBundleError> {
        if self.kernel_bindings.len() != kernels.len() {
            return Err(
                KernelBoundAuthorizationBundleError::KernelBindingCountMismatch {
                    got: self.kernel_bindings.len(),
                    expected: kernels.len(),
                },
            );
        }

        let stealth_offset = self.validate_intrinsic()?;

        let mut seen_kernels = std::collections::BTreeSet::new();

        for (index, (binding, kernel)) in
            self.kernel_bindings.iter().zip(kernels.iter()).enumerate()
        {
            // Match the identity used by TxKernel::id(): excess + nonce R.
            let identity = (kernel.excess.0, kernel.excess_sig.r);

            if !seen_kernels.insert(identity) {
                return Err(KernelBoundAuthorizationBundleError::DuplicateKernelBinding { index });
            }

            if !binding.verify(kernel) {
                return Err(
                    KernelBoundAuthorizationBundleError::InvalidKernelStealthBinding { index },
                );
            }
        }

        let loose = AuthorizationBundleV1 {
            sender_authorizations: self.sender_authorizations.clone(),

            input_authorizations: self.input_authorizations.clone(),

            stealth_excesses: self
                .kernel_bindings
                .iter()
                .map(|binding| binding.stealth_excess)
                .collect(),

            stealth_offset: StealthOffsetV1::from_scalar(&stealth_offset),
        };

        loose
            .validate(outputs, input_commits, canonical_kos)
            .map_err(KernelBoundAuthorizationBundleError::Authorization)
    }

    /// Aggregate two kernel-bound bundles without changing either source.
    ///
    /// Kernel bindings stay paired with their E' values and are never split.
    pub fn checked_aggregate(
        &self,
        other: &Self,
    ) -> Result<Self, KernelBoundAuthorizationBundleError> {
        let left_offset = self.validate_intrinsic()?;

        let right_offset = other.validate_intrinsic()?;

        let mut sender_authorizations = Vec::with_capacity(
            self.sender_authorizations.len() + other.sender_authorizations.len(),
        );

        sender_authorizations.extend(self.sender_authorizations.iter().copied());

        sender_authorizations.extend(other.sender_authorizations.iter().copied());

        let mut input_authorizations =
            Vec::with_capacity(self.input_authorizations.len() + other.input_authorizations.len());

        input_authorizations.extend(self.input_authorizations.iter().copied());

        input_authorizations.extend(other.input_authorizations.iter().copied());

        let mut kernel_bindings =
            Vec::with_capacity(self.kernel_bindings.len() + other.kernel_bindings.len());

        kernel_bindings.extend(self.kernel_bindings.iter().copied());

        kernel_bindings.extend(other.kernel_bindings.iter().copied());

        Ok(Self {
            sender_authorizations,
            input_authorizations,
            kernel_bindings,

            stealth_offset: StealthOffsetV1::from_scalar(&(left_offset + right_offset)),
        })
    }
}

/// Prototype container joining all cut-through authorization material.
///
/// This remains deliberately separate from [`crate::Transaction`] and
/// [`crate::BlockBody`]. It is not consensus-active yet.
///
/// Canonical `Ko` values are intentionally NOT stored here. They must be
/// supplied by authoritative UTXO state during validation.
///
/// Kernel-to-stealth-excess binding is also intentionally deferred to the
/// kernel-integration phase. This type proves that the authorization material
/// itself can be validated and aggregated atomically before changing the
/// active transaction format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationBundleV1 {
    /// One independent `Ks` authorization for each retained output.
    pub sender_authorizations: Vec<SenderAuthorizationV1>,

    /// One `Ki` spend authorization for each retained input.
    pub input_authorizations: Vec<SpendAuthorizationV1>,

    /// Retained aggregate stealth excess material `E'`.
    pub stealth_excesses: Vec<StealthExcessV1>,

    /// Aggregate stealth offset `x'`.
    pub stealth_offset: StealthOffsetV1,
}

/// Failure while validating or aggregating an authorization bundle.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthorizationBundleError {
    #[error("sender authorization count mismatch: got {got}, expected {expected}")]
    SenderCountMismatch { got: usize, expected: usize },

    #[error("input authorization count mismatch: got {got}, expected {expected}")]
    InputCountMismatch { got: usize, expected: usize },

    #[error("canonical Ko count mismatch: got {got}, expected {expected}")]
    CanonicalKoCountMismatch { got: usize, expected: usize },

    #[error("invalid sender authorization at index {index}")]
    InvalidSenderAuthorization { index: usize },

    #[error("invalid input authorization at index {index}")]
    InvalidInputAuthorization { index: usize },

    #[error("malformed stealth excess at index {index}")]
    MalformedStealthExcess { index: usize },

    #[error("non-canonical stealth offset")]
    NonCanonicalStealthOffset,

    #[error("aggregate stealth authorization balance does not hold")]
    StealthBalanceMismatch,
}

impl AuthorizationBundleV1 {
    /// Perform context-free checks that are safe without UTXO/output context.
    fn validate_intrinsic(&self) -> Result<Scalar, AuthorizationBundleError> {
        for (index, excess) in self.stealth_excesses.iter().enumerate() {
            if !excess.is_well_formed() {
                return Err(AuthorizationBundleError::MalformedStealthExcess { index });
            }
        }

        self.stealth_offset
            .scalar()
            .ok_or(AuthorizationBundleError::NonCanonicalStealthOffset)
    }

    /// Validate the complete authorization bundle against authoritative
    /// transaction/output context.
    ///
    /// `outputs` supplies the complete output data signed by each `Ks`.
    ///
    /// `input_commits` supplies the commitment referenced by each input.
    ///
    /// `canonical_kos` MUST come from authoritative UTXO state. A caller must
    /// never substitute a Ko supplied by an untrusted input.
    pub fn validate(
        &self,
        outputs: &[nightfall_crypto::Output],
        input_commits: &[Commitment],
        canonical_kos: &[[u8; 32]],
    ) -> Result<(), AuthorizationBundleError> {
        if self.sender_authorizations.len() != outputs.len() {
            return Err(AuthorizationBundleError::SenderCountMismatch {
                got: self.sender_authorizations.len(),
                expected: outputs.len(),
            });
        }

        if self.input_authorizations.len() != input_commits.len() {
            return Err(AuthorizationBundleError::InputCountMismatch {
                got: self.input_authorizations.len(),
                expected: input_commits.len(),
            });
        }

        if canonical_kos.len() != input_commits.len() {
            return Err(AuthorizationBundleError::CanonicalKoCountMismatch {
                got: canonical_kos.len(),
                expected: input_commits.len(),
            });
        }

        let stealth_offset = self.validate_intrinsic()?;

        for (index, (auth, output)) in self
            .sender_authorizations
            .iter()
            .zip(outputs.iter())
            .enumerate()
        {
            if !auth.verify(output) {
                return Err(AuthorizationBundleError::InvalidSenderAuthorization { index });
            }
        }

        for index in 0..self.input_authorizations.len() {
            if !self.input_authorizations[index]
                .verify(&input_commits[index], &canonical_kos[index])
            {
                return Err(AuthorizationBundleError::InvalidInputAuthorization { index });
            }
        }

        let sender_keys: Vec<[u8; 32]> = self
            .sender_authorizations
            .iter()
            .map(|auth| auth.key)
            .collect();

        let input_keys: Vec<[u8; 32]> = self
            .input_authorizations
            .iter()
            .map(|auth| auth.ki)
            .collect();

        let excesses: Vec<Commitment> = self
            .stealth_excesses
            .iter()
            .map(|excess| excess.point)
            .collect();

        if !verify_stealth_balance(
            &sender_keys,
            &input_keys,
            canonical_kos,
            &excesses,
            &stealth_offset,
        ) {
            return Err(AuthorizationBundleError::StealthBalanceMismatch);
        }

        Ok(())
    }

    /// Aggregate two bundles without mutating either source.
    ///
    /// Failure is atomic: malformed excess/offset material returns an error and
    /// neither original bundle is changed.
    pub fn checked_aggregate(&self, other: &Self) -> Result<Self, AuthorizationBundleError> {
        let left_offset = self.validate_intrinsic()?;
        let right_offset = other.validate_intrinsic()?;

        let mut sender_authorizations = Vec::with_capacity(
            self.sender_authorizations.len() + other.sender_authorizations.len(),
        );

        sender_authorizations.extend(self.sender_authorizations.iter().copied());
        sender_authorizations.extend(other.sender_authorizations.iter().copied());

        let mut input_authorizations =
            Vec::with_capacity(self.input_authorizations.len() + other.input_authorizations.len());

        input_authorizations.extend(self.input_authorizations.iter().copied());
        input_authorizations.extend(other.input_authorizations.iter().copied());

        let mut stealth_excesses =
            Vec::with_capacity(self.stealth_excesses.len() + other.stealth_excesses.len());

        stealth_excesses.extend(self.stealth_excesses.iter().copied());
        stealth_excesses.extend(other.stealth_excesses.iter().copied());

        Ok(Self {
            sender_authorizations,
            input_authorizations,
            stealth_excesses,
            stealth_offset: StealthOffsetV1::from_scalar(&(left_offset + right_offset)),
        })
    }
}

/// H(Ki || Ko), interpreted as a scalar.
///
/// Point validity is checked by [`input_verification_key`]. Keeping challenge
/// derivation independent makes the exact transcript explicit and testable.
pub fn input_auth_challenge(ki: &[u8; 32], ko: &[u8; 32]) -> Scalar {
    let h = hash_multi(INPUT_AUTH_CHALLENGE_DOMAIN, &[ki, ko]);
    Scalar::from_bytes_mod_order(h.0)
}

/// Calculate:
///
/// `K_input = Ki + H(Ki || Ko) * Ko`
///
/// Both Ki and Ko must be valid canonical compressed Ristretto points.
pub fn input_verification_key(ki: &[u8; 32], ko: &[u8; 32]) -> Option<RistrettoPoint> {
    let ki_point = CompressedRistretto(*ki).decompress()?;
    let ko_point = CompressedRistretto(*ko).decompress()?;
    let challenge = input_auth_challenge(ki, ko);

    Some(ki_point + ko_point * challenge)
}

/// Construct the corresponding secret:
///
/// `k_input = ki + H(Ki || Ko) * ko`
///
/// The supplied `ko_secret` must actually open canonical Ko. This guard is
/// wallet-side defence in depth; consensus verification uses only public data.
pub fn input_signing_secret(
    ki_secret: &Scalar,
    ko_secret: &Scalar,
    ko: &[u8; 32],
) -> Option<Scalar> {
    let canonical_ko = CompressedRistretto(*ko).decompress()?;

    if generator_g() * *ko_secret != canonical_ko {
        return None;
    }

    let ki = (generator_g() * *ki_secret).compress().to_bytes();
    let challenge = input_auth_challenge(&ki, ko);

    Some(*ki_secret + challenge * *ko_secret)
}

/// Produce candidate LIP-0004-style input authorization.
///
/// Returns `(Ki, signature)`.
///
/// This does not modify [`crate::Input`] and therefore has no effect on current
/// consensus validation.
pub fn sign_input_authorization(
    commit: &Commitment,
    ki_secret: &Scalar,
    ko_secret: &Scalar,
    canonical_ko: &[u8; 32],
) -> Option<([u8; 32], SchnorrSig)> {
    let ki = (generator_g() * *ki_secret).compress().to_bytes();

    let signing_secret = input_signing_secret(ki_secret, ko_secret, canonical_ko)?;

    let msg = Transaction::input_message(commit);
    let signature = sig::sign(&signing_secret, &generator_g(), &msg);

    Some((ki, signature))
}

/// Verify candidate input authorization against canonical Ko.
///
/// The caller must obtain `canonical_ko` from trusted UTXO state, never from
/// attacker-controlled replacement metadata.
pub fn verify_input_authorization(
    commit: &Commitment,
    ki: &[u8; 32],
    signature: &SchnorrSig,
    canonical_ko: &[u8; 32],
) -> bool {
    let Some(key) = input_verification_key(ki, canonical_ko) else {
        return false;
    };

    let msg = Transaction::input_message(commit);

    sig::verify(&key, &generator_g(), &msg, signature)
}

/// Sum compressed Ristretto public keys, failing closed if any key is
/// malformed.
///
/// This helper is intentionally private: callers should use the complete
/// stealth-balance verifier rather than constructing partial equations.
fn sum_public_keys(keys: &[[u8; 32]]) -> Option<RistrettoPoint> {
    let mut sum = RistrettoPoint::identity();

    for key in keys {
        sum += CompressedRistretto(*key).decompress()?;
    }

    Some(sum)
}

/// Construct the scalar stealth offset
///
/// `x' = Σks + Σki - Σko - Σe'`
///
/// corresponding to the public balance equation
///
/// `ΣKs + ΣKi - ΣKo = ΣE' + x'G`.
///
/// This is a construction helper only. Consensus must verify the public
/// equation independently.
pub fn stealth_offset_secret(
    sender_secrets: &[Scalar],
    input_ephemeral_secrets: &[Scalar],
    spent_output_secrets: &[Scalar],
    stealth_excess_secrets: &[Scalar],
) -> Scalar {
    let mut offset = Scalar::ZERO;

    for secret in sender_secrets {
        offset += *secret;
    }

    for secret in input_ephemeral_secrets {
        offset += *secret;
    }

    for secret in spent_output_secrets {
        offset -= *secret;
    }

    for secret in stealth_excess_secrets {
        offset -= *secret;
    }

    offset
}

/// Verify the aggregate stealth-authorization balance:
///
/// `ΣKs + ΣKi - ΣKo = ΣE' + x'G`.
///
/// * `sender_keys` are the sender authorization keys of retained outputs.
/// * `input_ephemeral_keys` are the `Ki` values carried by retained inputs.
/// * `spent_output_keys` are the canonical `Ko` values of those inputs.
/// * `stealth_excesses` are the retained stealth excess points `E'`.
/// * `stealth_offset` is the aggregate scalar offset `x'`.
///
/// Every compressed point is decompressed here and malformed material fails
/// closed. This function is deliberately independent of transaction grouping,
/// which allows offsets and excesses to compose under aggregation.
pub fn verify_stealth_balance(
    sender_keys: &[[u8; 32]],
    input_ephemeral_keys: &[[u8; 32]],
    spent_output_keys: &[[u8; 32]],
    stealth_excesses: &[Commitment],
    stealth_offset: &Scalar,
) -> bool {
    let Some(sender_sum) = sum_public_keys(sender_keys) else {
        return false;
    };

    let Some(input_sum) = sum_public_keys(input_ephemeral_keys) else {
        return false;
    };

    let Some(spent_sum) = sum_public_keys(spent_output_keys) else {
        return false;
    };

    let mut excess_sum = RistrettoPoint::identity();

    for excess in stealth_excesses {
        let Some(point) = excess.point() else {
            return false;
        };

        excess_sum += point;
    }

    let lhs = sender_sum + input_sum - spent_sum;
    let rhs = excess_sum + generator_g() * *stealth_offset;

    lhs == rhs
}

#[cfg(test)]
mod tests {
    use super::*;
    use curve25519_dalek::scalar::Scalar;
    use nightfall_crypto::{create_output, scan_output, WalletKeys};
    use nightfall_types::NetworkId;
    use rand::rngs::OsRng;

    fn owned_output() -> (WalletKeys, nightfall_crypto::Output, Scalar) {
        let receiver = WalletKeys::generate();

        let (output, _) = create_output(
            &receiver.address(),
            42_000,
            "phase2a",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output");

        let discovered =
            scan_output(&receiver.view_key(), &output).expect("receiver must discover output");

        let ko_secret = discovered.spend_secret(&receiver);

        assert_eq!(
            (generator_g() * ko_secret).compress().to_bytes(),
            output.output_pk,
            "recovered spend secret must open canonical Ko",
        );

        (receiver, output, ko_secret)
    }

    #[test]
    fn legitimate_receiver_authorization_verifies() {
        let (_receiver, output, ko_secret) = owned_output();
        let ki_secret = Scalar::random(&mut OsRng);

        let (ki, signature) =
            sign_input_authorization(&output.commit, &ki_secret, &ko_secret, &output.output_pk)
                .expect("legitimate signing material");

        assert!(
            verify_input_authorization(&output.commit, &ki, &signature, &output.output_pk,),
            "legitimate receiver authorization must verify",
        );
    }

    #[test]
    fn authorization_is_bound_to_commitment() {
        let (_receiver, output, ko_secret) = owned_output();
        let ki_secret = Scalar::random(&mut OsRng);

        let (ki, signature) =
            sign_input_authorization(&output.commit, &ki_secret, &ko_secret, &output.output_pk)
                .expect("sign");

        let other_commit = Commitment::new(42_000, &Scalar::random(&mut OsRng));

        assert!(
            !verify_input_authorization(&other_commit, &ki, &signature, &output.output_pk,),
            "authorization must not move to another commitment",
        );
    }

    #[test]
    fn authorization_is_bound_to_canonical_ko() {
        let (_receiver, output, ko_secret) = owned_output();
        let ki_secret = Scalar::random(&mut OsRng);

        let (ki, signature) =
            sign_input_authorization(&output.commit, &ki_secret, &ko_secret, &output.output_pk)
                .expect("sign");

        let replacement_secret = Scalar::random(&mut OsRng);
        let replacement_ko = (generator_g() * replacement_secret).compress().to_bytes();

        assert!(
            !verify_input_authorization(&output.commit, &ki, &signature, &replacement_ko,),
            "substituting Ko must invalidate authorization",
        );
    }

    #[test]
    fn authorization_is_bound_to_ki() {
        let (_receiver, output, ko_secret) = owned_output();
        let ki_secret = Scalar::random(&mut OsRng);

        let (_ki, signature) =
            sign_input_authorization(&output.commit, &ki_secret, &ko_secret, &output.output_pk)
                .expect("sign");

        let replacement_ki = (generator_g() * Scalar::random(&mut OsRng))
            .compress()
            .to_bytes();

        assert!(
            !verify_input_authorization(
                &output.commit,
                &replacement_ki,
                &signature,
                &output.output_pk,
            ),
            "substituting Ki must invalidate authorization",
        );
    }

    #[test]
    fn wrong_ko_secret_is_rejected_before_signing() {
        let (_receiver, output, _ko_secret) = owned_output();

        let ki_secret = Scalar::random(&mut OsRng);
        let wrong_secret = Scalar::random(&mut OsRng);

        assert!(
            sign_input_authorization(&output.commit, &ki_secret, &wrong_secret, &output.output_pk,)
                .is_none(),
            "a secret which does not open canonical Ko must be rejected",
        );
    }

    #[test]
    fn malformed_public_keys_fail_closed() {
        let (_receiver, output, ko_secret) = owned_output();
        let ki_secret = Scalar::random(&mut OsRng);

        let (_ki, signature) =
            sign_input_authorization(&output.commit, &ki_secret, &ko_secret, &output.output_pk)
                .expect("sign");

        let malformed = [0xff; 32];

        assert!(
            !verify_input_authorization(&output.commit, &malformed, &signature, &output.output_pk,),
            "malformed Ki must fail",
        );

        let valid_ki = (generator_g() * ki_secret).compress().to_bytes();

        assert!(
            !verify_input_authorization(&output.commit, &valid_ki, &signature, &malformed,),
            "malformed Ko must fail",
        );
    }

    #[test]
    fn stealth_balance_accepts_exact_equation() {
        let ks = Scalar::random(&mut OsRng);
        let ki = Scalar::random(&mut OsRng);
        let ko = Scalar::random(&mut OsRng);
        let excess_secret = Scalar::random(&mut OsRng);

        let offset = stealth_offset_secret(&[ks], &[ki], &[ko], &[excess_secret]);

        let sender_keys = [(generator_g() * ks).compress().to_bytes()];
        let input_keys = [(generator_g() * ki).compress().to_bytes()];
        let spent_keys = [(generator_g() * ko).compress().to_bytes()];
        let excesses = [Commitment::from_point(generator_g() * excess_secret)];

        assert!(
            verify_stealth_balance(&sender_keys, &input_keys, &spent_keys, &excesses, &offset,),
            "exact stealth equation must verify",
        );
    }

    #[test]
    fn stealth_balance_composes_under_aggregation() {
        let ks1 = Scalar::random(&mut OsRng);
        let ki1 = Scalar::random(&mut OsRng);
        let ko1 = Scalar::random(&mut OsRng);
        let e1 = Scalar::random(&mut OsRng);

        let ks2 = Scalar::random(&mut OsRng);
        let ki2 = Scalar::random(&mut OsRng);
        let ko2 = Scalar::random(&mut OsRng);
        let e2 = Scalar::random(&mut OsRng);

        let x1 = stealth_offset_secret(&[ks1], &[ki1], &[ko1], &[e1]);

        let x2 = stealth_offset_secret(&[ks2], &[ki2], &[ko2], &[e2]);

        let aggregate_offset = x1 + x2;

        let sender_keys = [
            (generator_g() * ks1).compress().to_bytes(),
            (generator_g() * ks2).compress().to_bytes(),
        ];

        let input_keys = [
            (generator_g() * ki1).compress().to_bytes(),
            (generator_g() * ki2).compress().to_bytes(),
        ];

        let spent_keys = [
            (generator_g() * ko1).compress().to_bytes(),
            (generator_g() * ko2).compress().to_bytes(),
        ];

        let excesses = [
            Commitment::from_point(generator_g() * e1),
            Commitment::from_point(generator_g() * e2),
        ];

        assert!(
            verify_stealth_balance(
                &sender_keys,
                &input_keys,
                &spent_keys,
                &excesses,
                &aggregate_offset,
            ),
            "independent stealth balances must compose exactly",
        );
    }

    #[test]
    fn substituted_canonical_ko_breaks_stealth_balance() {
        let ks = Scalar::random(&mut OsRng);
        let ki = Scalar::random(&mut OsRng);
        let ko = Scalar::random(&mut OsRng);
        let e = Scalar::random(&mut OsRng);

        let offset = stealth_offset_secret(&[ks], &[ki], &[ko], &[e]);

        let sender_keys = [(generator_g() * ks).compress().to_bytes()];

        let input_keys = [(generator_g() * ki).compress().to_bytes()];

        let replacement_ko = Scalar::random(&mut OsRng);

        let spent_keys = [(generator_g() * replacement_ko).compress().to_bytes()];

        let excesses = [Commitment::from_point(generator_g() * e)];

        assert!(
            !verify_stealth_balance(&sender_keys, &input_keys, &spent_keys, &excesses, &offset,),
            "canonical Ko substitution must break the balance",
        );
    }

    #[test]
    fn wrong_stealth_offset_breaks_balance() {
        let ks = Scalar::random(&mut OsRng);
        let ki = Scalar::random(&mut OsRng);
        let ko = Scalar::random(&mut OsRng);
        let e = Scalar::random(&mut OsRng);

        let correct = stealth_offset_secret(&[ks], &[ki], &[ko], &[e]);

        let wrong = correct + Scalar::ONE;

        let sender_keys = [(generator_g() * ks).compress().to_bytes()];
        let input_keys = [(generator_g() * ki).compress().to_bytes()];
        let spent_keys = [(generator_g() * ko).compress().to_bytes()];
        let excesses = [Commitment::from_point(generator_g() * e)];

        assert!(
            !verify_stealth_balance(&sender_keys, &input_keys, &spent_keys, &excesses, &wrong,),
            "arbitrary offset adjustment must fail",
        );
    }

    #[test]
    fn malformed_stealth_balance_material_fails_closed() {
        let valid = Scalar::random(&mut OsRng);
        let valid_key = (generator_g() * valid).compress().to_bytes();

        let malformed = [0xff; 32];

        assert!(
            !verify_stealth_balance(&[malformed], &[valid_key], &[valid_key], &[], &Scalar::ZERO,),
            "malformed sender key must fail closed",
        );

        assert!(
            !verify_stealth_balance(&[valid_key], &[malformed], &[valid_key], &[], &Scalar::ZERO,),
            "malformed Ki must fail closed",
        );

        assert!(
            !verify_stealth_balance(&[valid_key], &[valid_key], &[malformed], &[], &Scalar::ZERO,),
            "malformed Ko must fail closed",
        );

        assert!(
            !verify_stealth_balance(
                &[valid_key],
                &[],
                &[valid_key],
                &[Commitment(malformed)],
                &Scalar::ZERO,
            ),
            "malformed stealth excess must fail closed",
        );
    }

    #[test]
    fn candidate_input_authorization_roundtrips_exactly() {
        let (_receiver, output, ko_secret) = owned_output();
        let ki_secret = Scalar::random(&mut OsRng);

        let auth =
            SpendAuthorizationV1::sign(&output.commit, &ki_secret, &ko_secret, &output.output_pk)
                .expect("valid authorization");

        assert!(
            auth.verify(&output.commit, &output.output_pk),
            "typed authorization must verify",
        );

        let canonical = auth.canonical_bytes();

        let encoded = serde_json::to_vec(&auth).expect("serialize authorization");

        let decoded: SpendAuthorizationV1 =
            serde_json::from_slice(&encoded).expect("deserialize authorization");

        assert_eq!(decoded, auth);
        assert_eq!(decoded.canonical_bytes(), canonical);
        assert_eq!(canonical.len(), 96);
    }

    #[test]
    fn candidate_authorization_still_requires_canonical_ko() {
        let (_receiver, output, ko_secret) = owned_output();
        let ki_secret = Scalar::random(&mut OsRng);

        let auth =
            SpendAuthorizationV1::sign(&output.commit, &ki_secret, &ko_secret, &output.output_pk)
                .expect("valid authorization");

        let replacement_secret = Scalar::random(&mut OsRng);

        let replacement_ko = (generator_g() * replacement_secret).compress().to_bytes();

        assert!(
            !auth.verify(&output.commit, &replacement_ko,),
            "typed authorization must remain bound to canonical Ko",
        );
    }

    #[test]
    fn stealth_offset_requires_canonical_scalar_encoding() {
        let value = Scalar::random(&mut OsRng);

        let offset = StealthOffsetV1::from_scalar(&value);

        assert!(offset.is_canonical());
        assert_eq!(offset.scalar(), Some(value));

        let malformed = StealthOffsetV1 { bytes: [0xff; 32] };

        assert!(
            !malformed.is_canonical(),
            "non-canonical scalar encoding must fail closed",
        );

        assert!(malformed.scalar().is_none());
    }

    #[test]
    fn stealth_offsets_compose_as_scalars() {
        let a = Scalar::random(&mut OsRng);
        let b = Scalar::random(&mut OsRng);

        let encoded_a = StealthOffsetV1::from_scalar(&a);

        let encoded_b = StealthOffsetV1::from_scalar(&b);

        let aggregate = encoded_a
            .checked_add(&encoded_b)
            .expect("canonical offsets");

        assert_eq!(
            aggregate.scalar(),
            Some(a + b),
            "offset aggregation must preserve scalar addition",
        );
    }

    #[test]
    fn malformed_offset_cannot_participate_in_aggregation() {
        let valid = StealthOffsetV1::from_scalar(&Scalar::random(&mut OsRng));

        let malformed = StealthOffsetV1 { bytes: [0xff; 32] };

        assert!(malformed.checked_add(&valid).is_none());
        assert!(valid.checked_add(&malformed).is_none());
    }

    #[test]
    fn stealth_offset_roundtrips_without_reduction() {
        let value = Scalar::random(&mut OsRng);

        let offset = StealthOffsetV1::from_scalar(&value);

        let encoded = serde_json::to_vec(&offset).expect("serialize offset");

        let decoded: StealthOffsetV1 =
            serde_json::from_slice(&encoded).expect("deserialize offset");

        assert_eq!(decoded, offset);
        assert_eq!(decoded.scalar(), Some(value));
        assert_eq!(decoded.canonical_bytes(), value.to_bytes(),);
    }

    #[test]
    fn stealth_excess_roundtrips_exactly() {
        let secret = Scalar::random(&mut OsRng);

        let point = Commitment::from_point(generator_g() * secret);

        let excess = StealthExcessV1::new(point).expect("valid excess");

        let encoded = serde_json::to_vec(&excess).expect("serialize excess");

        let decoded: StealthExcessV1 =
            serde_json::from_slice(&encoded).expect("deserialize excess");

        assert_eq!(decoded, excess);
        assert_eq!(decoded.canonical_bytes(), point.0,);
    }

    #[test]
    fn stealth_excess_rejects_malformed_points() {
        assert!(
            StealthExcessV1::new(Commitment([0xff; 32])).is_none(),
            "malformed compressed point must fail closed",
        );
    }

    #[test]
    fn independent_sender_authorization_verifies() {
        let receiver = WalletKeys::generate();

        let (output, _) = create_output(
            &receiver.address(),
            55_000,
            "phase2d",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output");

        let ks = Scalar::random(&mut OsRng);

        let auth = SenderAuthorizationV1::sign(&output, &ks);

        assert!(
            auth.verify(&output),
            "independent Ks authorization must verify",
        );

        assert_eq!(auth.key, (generator_g() * ks).compress().to_bytes(),);
    }

    #[test]
    fn sender_authorization_is_bound_to_output() {
        let receiver = WalletKeys::generate();

        let (output_a, _) = create_output(
            &receiver.address(),
            1_000,
            "a",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output a");

        let (output_b, _) = create_output(
            &receiver.address(),
            1_000,
            "b",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output b");

        let ks = Scalar::random(&mut OsRng);

        let auth = SenderAuthorizationV1::sign(&output_a, &ks);

        assert!(auth.verify(&output_a));

        assert!(
            !auth.verify(&output_b),
            "sender proof must not move to another output",
        );
    }

    #[test]
    fn sender_key_substitution_breaks_authorization() {
        let receiver = WalletKeys::generate();

        let (output, _) = create_output(
            &receiver.address(),
            2_000,
            "ks",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output");

        let ks = Scalar::random(&mut OsRng);

        let auth = SenderAuthorizationV1::sign(&output, &ks);

        let replacement = Scalar::random(&mut OsRng);

        let mut substituted = auth;

        substituted.key = (generator_g() * replacement).compress().to_bytes();

        assert!(
            !substituted.verify(&output),
            "changing Ks must invalidate sender authorization",
        );
    }

    #[test]
    fn malformed_sender_key_fails_closed() {
        let receiver = WalletKeys::generate();

        let (output, _) = create_output(
            &receiver.address(),
            3_000,
            "",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output");

        let ks = Scalar::random(&mut OsRng);

        let mut auth = SenderAuthorizationV1::sign(&output, &ks);

        auth.key = [0xff; 32];

        assert!(!auth.verify(&output), "malformed Ks must fail closed",);
    }

    #[test]
    fn sender_authorization_roundtrips_exactly() {
        let receiver = WalletKeys::generate();

        let (output, _) = create_output(
            &receiver.address(),
            4_000,
            "",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output");

        let ks = Scalar::random(&mut OsRng);

        let auth = SenderAuthorizationV1::sign(&output, &ks);

        let canonical = auth.canonical_bytes();

        let encoded = serde_json::to_vec(&auth).expect("serialize sender auth");

        let decoded: SenderAuthorizationV1 =
            serde_json::from_slice(&encoded).expect("deserialize sender auth");

        assert_eq!(decoded, auth);
        assert_eq!(decoded.canonical_bytes(), canonical,);
        assert!(decoded.verify(&output));
    }

    #[test]
    fn sender_authorization_key_can_feed_stealth_balance() {
        let receiver = WalletKeys::generate();

        let (output, _) = create_output(
            &receiver.address(),
            5_000,
            "",
            NetworkId::Devnet.proof_context(),
        )
        .expect("output");

        let ks = Scalar::random(&mut OsRng);
        let ki = Scalar::random(&mut OsRng);
        let ko = Scalar::random(&mut OsRng);
        let e = Scalar::random(&mut OsRng);

        let sender = SenderAuthorizationV1::sign(&output, &ks);

        assert!(sender.verify(&output));

        let offset = stealth_offset_secret(&[ks], &[ki], &[ko], &[e]);

        let input_keys = [(generator_g() * ki).compress().to_bytes()];

        let spent_keys = [(generator_g() * ko).compress().to_bytes()];

        let excesses = [Commitment::from_point(generator_g() * e)];

        assert!(
            verify_stealth_balance(&[sender.key], &input_keys, &spent_keys, &excesses, &offset,),
            "independent Ks must compose with stealth balance",
        );
    }

    fn valid_authorization_bundle_fixture() -> (
        AuthorizationBundleV1,
        Vec<nightfall_crypto::Output>,
        Vec<Commitment>,
        Vec<[u8; 32]>,
    ) {
        let spend_receiver = WalletKeys::generate();

        let (spent_output, _) = create_output(
            &spend_receiver.address(),
            12_345,
            "spent",
            NetworkId::Devnet.proof_context(),
        )
        .expect("spent output");

        let discovered = scan_output(&spend_receiver.view_key(), &spent_output)
            .expect("receiver discovers spent output");

        let ko_secret = discovered.spend_secret(&spend_receiver);

        assert_eq!(
            (generator_g() * ko_secret).compress().to_bytes(),
            spent_output.output_pk,
        );

        let new_receiver = WalletKeys::generate();

        let (new_output, _) = create_output(
            &new_receiver.address(),
            12_000,
            "new",
            NetworkId::Devnet.proof_context(),
        )
        .expect("new output");

        let ks = Scalar::random(&mut OsRng);
        let ki = Scalar::random(&mut OsRng);
        let excess_secret = Scalar::random(&mut OsRng);

        let sender_auth = SenderAuthorizationV1::sign(&new_output, &ks);

        let input_auth = SpendAuthorizationV1::sign(
            &spent_output.commit,
            &ki,
            &ko_secret,
            &spent_output.output_pk,
        )
        .expect("input authorization");

        let excess = StealthExcessV1::new(Commitment::from_point(generator_g() * excess_secret))
            .expect("stealth excess");

        let offset = stealth_offset_secret(&[ks], &[ki], &[ko_secret], &[excess_secret]);

        let bundle = AuthorizationBundleV1 {
            sender_authorizations: vec![sender_auth],
            input_authorizations: vec![input_auth],
            stealth_excesses: vec![excess],
            stealth_offset: StealthOffsetV1::from_scalar(&offset),
        };

        (
            bundle,
            vec![new_output],
            vec![spent_output.commit],
            vec![spent_output.output_pk],
        )
    }

    #[test]
    fn authorization_bundle_validates_atomically() {
        let (bundle, outputs, commits, kos) = valid_authorization_bundle_fixture();

        assert_eq!(bundle.validate(&outputs, &commits, &kos,), Ok(()),);
    }

    #[test]
    fn authorization_bundle_rejects_sender_tampering() {
        let (mut bundle, outputs, commits, kos) = valid_authorization_bundle_fixture();

        bundle.sender_authorizations[0].key = (generator_g() * Scalar::random(&mut OsRng))
            .compress()
            .to_bytes();

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos,),
            Err(AuthorizationBundleError::InvalidSenderAuthorization { index: 0 }),
        );
    }

    #[test]
    fn authorization_bundle_rejects_input_tampering() {
        let (mut bundle, outputs, commits, kos) = valid_authorization_bundle_fixture();

        bundle.input_authorizations[0].ki = (generator_g() * Scalar::random(&mut OsRng))
            .compress()
            .to_bytes();

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos,),
            Err(AuthorizationBundleError::InvalidInputAuthorization { index: 0 }),
        );
    }

    #[test]
    fn authorization_bundle_rejects_ko_substitution() {
        let (bundle, outputs, commits, mut kos) = valid_authorization_bundle_fixture();

        kos[0] = (generator_g() * Scalar::random(&mut OsRng))
            .compress()
            .to_bytes();

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos,),
            Err(AuthorizationBundleError::InvalidInputAuthorization { index: 0 }),
        );
    }

    #[test]
    fn authorization_bundle_rejects_wrong_offset() {
        let (mut bundle, outputs, commits, kos) = valid_authorization_bundle_fixture();

        let current = bundle.stealth_offset.scalar().expect("canonical");

        bundle.stealth_offset = StealthOffsetV1::from_scalar(&(current + Scalar::ONE));

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos,),
            Err(AuthorizationBundleError::StealthBalanceMismatch),
        );
    }

    #[test]
    fn authorization_bundle_rejects_malformed_excess() {
        let (mut bundle, outputs, commits, kos) = valid_authorization_bundle_fixture();

        bundle.stealth_excesses[0] = StealthExcessV1 {
            point: Commitment([0xff; 32]),
        };

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos,),
            Err(AuthorizationBundleError::MalformedStealthExcess { index: 0 }),
        );
    }

    #[test]
    fn authorization_bundle_rejects_noncanonical_offset() {
        let (mut bundle, outputs, commits, kos) = valid_authorization_bundle_fixture();

        bundle.stealth_offset = StealthOffsetV1 { bytes: [0xff; 32] };

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos,),
            Err(AuthorizationBundleError::NonCanonicalStealthOffset),
        );
    }

    #[test]
    fn authorization_bundles_aggregate_and_still_validate() {
        let (first, mut outputs, mut commits, mut kos) = valid_authorization_bundle_fixture();

        let (second, outputs_b, commits_b, kos_b) = valid_authorization_bundle_fixture();

        let aggregate = first.checked_aggregate(&second).expect("valid aggregation");

        outputs.extend(outputs_b);
        commits.extend(commits_b);
        kos.extend(kos_b);

        assert_eq!(
            aggregate.validate(&outputs, &commits, &kos,),
            Ok(()),
            "aggregated authorization material must remain valid",
        );

        assert_eq!(aggregate.sender_authorizations.len(), 2,);

        assert_eq!(aggregate.input_authorizations.len(), 2,);

        assert_eq!(aggregate.stealth_excesses.len(), 2,);
    }

    #[test]
    fn failed_bundle_aggregation_does_not_mutate_sources() {
        let (first, _outputs_a, _commits_a, _kos_a) = valid_authorization_bundle_fixture();

        let (mut second, _outputs_b, _commits_b, _kos_b) = valid_authorization_bundle_fixture();

        second.stealth_offset = StealthOffsetV1 { bytes: [0xff; 32] };

        let first_before = first.clone();
        let second_before = second.clone();

        assert_eq!(
            first.checked_aggregate(&second),
            Err(AuthorizationBundleError::NonCanonicalStealthOffset),
        );

        assert_eq!(first, first_before);
        assert_eq!(second, second_before);
    }

    #[test]
    fn authorization_bundle_roundtrips_exactly() {
        let (bundle, outputs, commits, kos) = valid_authorization_bundle_fixture();

        let encoded = serde_json::to_vec(&bundle).expect("serialize bundle");

        let decoded: AuthorizationBundleV1 =
            serde_json::from_slice(&encoded).expect("deserialize bundle");

        assert_eq!(decoded, bundle);

        assert_eq!(decoded.validate(&outputs, &commits, &kos,), Ok(()),);
    }

    fn kernel_stealth_fixture() -> (TxKernel, Scalar, StealthExcessV1) {
        let kernel_secret = Scalar::random(&mut OsRng);

        let kernel = nightfall_crypto::build_kernel(
            nightfall_crypto::KernelFeature::Plain,
            7,
            0,
            0,
            &kernel_secret,
        );

        assert!(kernel.verify_signature());

        let stealth_secret = Scalar::random(&mut OsRng);

        let stealth_excess =
            StealthExcessV1::new(Commitment::from_point(generator_g() * stealth_secret))
                .expect("valid stealth excess");

        (kernel, kernel_secret, stealth_excess)
    }

    #[test]
    fn kernel_stealth_binding_verifies() {
        let (kernel, kernel_secret, stealth_excess) = kernel_stealth_fixture();

        let binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
            .expect("valid binding");

        assert!(
            binding.verify(&kernel),
            "kernel must authorize its exact E'",
        );
    }

    #[test]
    fn kernel_stealth_binding_rejects_excess_substitution() {
        let (kernel, kernel_secret, stealth_excess) = kernel_stealth_fixture();

        let mut binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
            .expect("valid binding");

        let replacement_secret = Scalar::random(&mut OsRng);

        binding.stealth_excess =
            StealthExcessV1::new(Commitment::from_point(generator_g() * replacement_secret))
                .expect("replacement point");

        assert!(
            !binding.verify(&kernel),
            "E' substitution must invalidate kernel binding",
        );
    }

    #[test]
    fn kernel_stealth_binding_rejects_kernel_field_mutation() {
        let (kernel, kernel_secret, stealth_excess) = kernel_stealth_fixture();

        let binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
            .expect("valid binding");

        let mut changed = kernel.clone();
        changed.fee_darks += 1;

        assert!(
            !binding.verify(&changed),
            "changing a signed kernel field must invalidate binding",
        );
    }

    #[test]
    fn kernel_stealth_binding_rejects_kernel_signature_mutation() {
        let (kernel, kernel_secret, stealth_excess) = kernel_stealth_fixture();

        let binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
            .expect("valid binding");

        let mut changed = kernel.clone();
        changed.excess_sig.s[0] ^= 1;

        assert!(
            !binding.verify(&changed),
            "binding must be tied to the concrete existing kernel signature",
        );
    }

    #[test]
    fn kernel_stealth_binding_rejects_wrong_kernel_secret() {
        let (kernel, _kernel_secret, stealth_excess) = kernel_stealth_fixture();

        let wrong_secret = Scalar::random(&mut OsRng);

        assert!(
            KernelStealthBindingV1::sign(&kernel, &wrong_secret, stealth_excess,).is_none(),
            "secret not opening E must not authorize E'",
        );
    }

    #[test]
    fn kernel_stealth_binding_rejects_malformed_excess() {
        let (kernel, kernel_secret, _stealth_excess) = kernel_stealth_fixture();

        let malformed = StealthExcessV1 {
            point: Commitment([0xff; 32]),
        };

        assert!(
            KernelStealthBindingV1::sign(&kernel, &kernel_secret, malformed,).is_none(),
            "malformed E' must fail closed",
        );
    }

    #[test]
    fn kernel_stealth_binding_roundtrips_exactly() {
        let (kernel, kernel_secret, stealth_excess) = kernel_stealth_fixture();

        let binding = KernelStealthBindingV1::sign(&kernel, &kernel_secret, stealth_excess)
            .expect("valid binding");

        let canonical = binding.canonical_bytes();

        let encoded = serde_json::to_vec(&binding).expect("serialize binding");

        let decoded: KernelStealthBindingV1 =
            serde_json::from_slice(&encoded).expect("deserialize binding");

        assert_eq!(decoded, binding);
        assert_eq!(decoded.canonical_bytes(), canonical,);

        assert!(
            decoded.verify(&kernel),
            "round-tripped binding must still verify",
        );
    }

    type KernelBoundBundleFixture = (
        KernelBoundAuthorizationBundleV1,
        Vec<nightfall_crypto::Output>,
        Vec<Commitment>,
        Vec<[u8; 32]>,
        Vec<TxKernel>,
    );

    fn kernel_bound_bundle_fixture() -> KernelBoundBundleFixture {
        let (loose, outputs, commits, kos) = valid_authorization_bundle_fixture();

        assert_eq!(
            loose.stealth_excesses.len(),
            1,
            "fixture expects exactly one E'",
        );

        let kernel_secret = Scalar::random(&mut OsRng);

        let kernel = nightfall_crypto::build_kernel(
            nightfall_crypto::KernelFeature::Plain,
            0,
            0,
            0,
            &kernel_secret,
        );

        assert!(kernel.verify_signature());

        let binding =
            KernelStealthBindingV1::sign(&kernel, &kernel_secret, loose.stealth_excesses[0])
                .expect("kernel/E' binding");

        let bundle = KernelBoundAuthorizationBundleV1 {
            sender_authorizations: loose.sender_authorizations,

            input_authorizations: loose.input_authorizations,

            kernel_bindings: vec![binding],

            stealth_offset: loose.stealth_offset,
        };

        (bundle, outputs, commits, kos, vec![kernel])
    }

    #[test]
    fn kernel_bound_bundle_validates_atomically() {
        let (bundle, outputs, commits, kos, kernels) = kernel_bound_bundle_fixture();

        assert_eq!(bundle.validate(&outputs, &commits, &kos, &kernels,), Ok(()),);
    }

    #[test]
    fn kernel_bound_bundle_rejects_kernel_substitution() {
        let (bundle, outputs, commits, kos, mut kernels) = kernel_bound_bundle_fixture();

        let replacement_secret = Scalar::random(&mut OsRng);

        kernels[0] = nightfall_crypto::build_kernel(
            nightfall_crypto::KernelFeature::Plain,
            0,
            0,
            0,
            &replacement_secret,
        );

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos, &kernels,),
            Err(KernelBoundAuthorizationBundleError::InvalidKernelStealthBinding { index: 0 }),
        );
    }

    #[test]
    fn kernel_bound_bundle_rejects_binding_excess_substitution() {
        let (mut bundle, outputs, commits, kos, kernels) = kernel_bound_bundle_fixture();

        let replacement = Scalar::random(&mut OsRng);

        bundle.kernel_bindings[0].stealth_excess =
            StealthExcessV1::new(Commitment::from_point(generator_g() * replacement))
                .expect("replacement E'");

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos, &kernels,),
            Err(KernelBoundAuthorizationBundleError::InvalidKernelStealthBinding { index: 0 }),
        );
    }

    #[test]
    fn kernel_bound_bundle_requires_one_kernel_per_binding() {
        let (bundle, outputs, commits, kos, _kernels) = kernel_bound_bundle_fixture();

        assert_eq!(
            bundle.validate(&outputs, &commits, &kos, &[],),
            Err(
                KernelBoundAuthorizationBundleError::KernelBindingCountMismatch {
                    got: 1,
                    expected: 0,
                }
            ),
        );
    }

    #[test]
    fn kernel_bound_bundle_rejects_duplicate_kernel_binding() {
        let (first, mut outputs, mut commits, mut kos, kernels) = kernel_bound_bundle_fixture();

        let mut duplicate = first.clone();

        // Preserve a mathematically valid doubled stealth balance.
        duplicate.stealth_offset = first.stealth_offset;

        let aggregate = first
            .checked_aggregate(&duplicate)
            .expect("structural aggregation");

        let first_outputs = outputs.clone();
        let first_commits = commits.clone();
        let first_kos = kos.clone();

        outputs.extend(first_outputs);
        commits.extend(first_commits);
        kos.extend(first_kos);

        let duplicate_kernels = vec![kernels[0].clone(), kernels[0].clone()];

        assert_eq!(
            aggregate.validate(&outputs, &commits, &kos, &duplicate_kernels,),
            Err(KernelBoundAuthorizationBundleError::DuplicateKernelBinding { index: 1 }),
        );
    }

    #[test]
    fn kernel_bound_bundles_aggregate_and_validate() {
        let (first, mut outputs, mut commits, mut kos, mut kernels) = kernel_bound_bundle_fixture();

        let (second, outputs_b, commits_b, kos_b, kernels_b) = kernel_bound_bundle_fixture();

        let aggregate = first.checked_aggregate(&second).expect("valid aggregation");

        outputs.extend(outputs_b);
        commits.extend(commits_b);
        kos.extend(kos_b);
        kernels.extend(kernels_b);

        assert_eq!(
            aggregate.validate(&outputs, &commits, &kos, &kernels,),
            Ok(()),
        );

        assert_eq!(aggregate.kernel_bindings.len(), 2,);
    }

    #[test]
    fn failed_kernel_bound_aggregation_is_atomic() {
        let (first, _outputs_a, _commits_a, _kos_a, _kernels_a) = kernel_bound_bundle_fixture();

        let (mut second, _outputs_b, _commits_b, _kos_b, _kernels_b) =
            kernel_bound_bundle_fixture();

        second.stealth_offset = StealthOffsetV1 { bytes: [0xff; 32] };

        let first_before = first.clone();

        let second_before = second.clone();

        assert_eq!(
            first.checked_aggregate(&second),
            Err(KernelBoundAuthorizationBundleError::NonCanonicalStealthOffset),
        );

        assert_eq!(first, first_before);
        assert_eq!(second, second_before);
    }

    #[test]
    fn kernel_bound_bundle_roundtrips_exactly() {
        let (bundle, outputs, commits, kos, kernels) = kernel_bound_bundle_fixture();

        let encoded = serde_json::to_vec(&bundle).expect("serialize kernel-bound bundle");

        let decoded: KernelBoundAuthorizationBundleV1 =
            serde_json::from_slice(&encoded).expect("deserialize kernel-bound bundle");

        assert_eq!(decoded, bundle,);

        assert_eq!(
            decoded.validate(&outputs, &commits, &kos, &kernels,),
            Ok(()),
        );
    }
}
