//! Pre-release encrypted wallet container shared by native and WASM clients.
//!
//! This module does not read/write files, migrate legacy storage or run timers.
//! Callers must serialize save/lock/unlock, durably save `snapshot()` before
//! discarding the vault, and clear their own phrase/password/preview buffers.
//! A lock is not protection from hostile application code or a compromised OS.
use crate::Wallet;
use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use nightfall_types::{NetworkId, PROTOCOL_VERSION};
use rand::{rngs::OsRng, RngCore};
use thiserror::Error;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"NFVAULT\0";
const FORMAT: u8 = 1;
const PROFILE: u8 = 1;
const PREFIX_LEN: usize = 31;
const HEADER_LEN: usize = PREFIX_LEN + 24;
const TAG_LEN: usize = 16;
/// Profile 1: Argon2id v1.3, 64 MiB, 3 passes, 4 lanes, 32-byte key.
/// Fixed costs cannot be raised or weakened by an attacker-controlled file.
const MEMORY_KIB: u32 = 65_536;
const PASSES: u32 = 3;
const LANES: u32 = 4;
pub const MAX_PLAINTEXT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_VAULT_BYTES: usize = MAX_PLAINTEXT_BYTES + HEADER_LEN + TAG_LEN;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("Not a supported Nightfall vault.")]
    InvalidEnvelope,
    #[error("Unsupported vault format, KDF profile or protocol. Do not overwrite this file.")]
    UnsupportedFormat,
    #[error("This vault belongs to a different network.")]
    WrongNetwork,
    #[error("Wrong password or damaged vault. Nothing was changed.")]
    Authentication,
    #[error("The vault contains an invalid wallet state.")]
    InvalidWallet,
    #[error("Use at least 12 characters; passwords are limited to 1024 UTF-8 bytes. Do not use your recovery words.")]
    PasswordPolicy,
    #[error("The vault exceeds this format's size limit.")]
    SizeLimit,
    #[error("The wallet is locked.")]
    Locked,
    #[error("The wallet is already unlocked.")]
    AlreadyUnlocked,
    #[error("Secure random generation is unavailable.")]
    RandomUnavailable,
    #[error("Unable to allocate or derive the vault key.")]
    KeyDerivation,
    #[error("Unable to encrypt the wallet state.")]
    Encryption,
}

struct Unlocked {
    key: Zeroizing<[u8; 32]>,
    prefix: [u8; PREFIX_LEN],
    wallet: Wallet,
}

/// No Debug/Serialize/Clone implementation: do not log or duplicate sessions.
/// `sealed` always remains available, including after a lock. It is a snapshot,
/// not proof that a host application durably persisted the latest changes.
pub struct Vault {
    sealed: Vec<u8>,
    unlocked: Option<Unlocked>,
}

fn network_byte(network: NetworkId) -> u8 {
    match network {
        NetworkId::Mainnet => 0,
        NetworkId::Testnet => 1,
        NetworkId::Devnet => 2,
    }
}

fn parse_header(blob: &[u8]) -> Result<NetworkId, VaultError> {
    if blob.len() > MAX_VAULT_BYTES {
        return Err(VaultError::SizeLimit);
    }
    if blob.len() < HEADER_LEN + TAG_LEN || &blob[..8] != MAGIC {
        return Err(VaultError::InvalidEnvelope);
    }
    if blob[8] != FORMAT || blob[9] != PROFILE || blob[11..15] != PROTOCOL_VERSION.to_le_bytes() {
        return Err(VaultError::UnsupportedFormat);
    }
    match blob[10] {
        0 => Ok(NetworkId::Mainnet),
        1 => Ok(NetworkId::Testnet),
        2 => Ok(NetworkId::Devnet),
        _ => Err(VaultError::InvalidEnvelope),
    }
}

fn password_policy(password: &str) -> Result<(), VaultError> {
    if password.len() > 1024 || password.chars().count() < 12 || password.trim().is_empty() {
        return Err(VaultError::PasswordPolicy);
    }
    Ok(())
}

fn random_bytes<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut bytes = [0u8; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| VaultError::RandomUnavailable)?;
    Ok(bytes)
}

fn new_prefix(network: NetworkId) -> Result<[u8; PREFIX_LEN], VaultError> {
    let mut header = [0u8; PREFIX_LEN];
    header[..8].copy_from_slice(MAGIC);
    header[8] = FORMAT;
    header[9] = PROFILE;
    header[10] = network_byte(network);
    header[11..15].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    header[15..31].copy_from_slice(&random_bytes::<16>()?);
    Ok(header)
}

fn derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    password_policy(password)?;
    let params =
        Params::new(MEMORY_KIB, PASSES, LANES, Some(32)).map_err(|_| VaultError::KeyDerivation)?;
    let mut memory = Zeroizing::new(Vec::<Block>::new());
    memory
        .try_reserve_exact(params.block_count())
        .map_err(|_| VaultError::KeyDerivation)?;
    memory.resize(params.block_count(), Block::default());
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(password.as_bytes(), salt, &mut key[..], &mut memory[..])
        .map_err(|_| VaultError::KeyDerivation)?;
    Ok(key)
}

fn encrypt(unlocked: &Unlocked) -> Result<Vec<u8>, VaultError> {
    if unlocked.prefix[10] != network_byte(unlocked.wallet.network) {
        return Err(VaultError::WrongNetwork);
    }
    let plain = Zeroizing::new(
        unlocked
            .wallet
            .export_state()
            .map_err(|_| VaultError::InvalidWallet)?,
    );
    if plain.len() > MAX_PLAINTEXT_BYTES {
        return Err(VaultError::SizeLimit);
    }
    let nonce = random_bytes::<24>()?;
    let mut header = [0u8; HEADER_LEN];
    header[..PREFIX_LEN].copy_from_slice(&unlocked.prefix);
    header[PREFIX_LEN..].copy_from_slice(&nonce);
    let cipher =
        XChaCha20Poly1305::new_from_slice(&unlocked.key[..]).map_err(|_| VaultError::Encryption)?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain.as_bytes(),
                aad: &header,
            },
        )
        .map_err(|_| VaultError::Encryption)?;
    let mut out = Vec::with_capacity(header.len() + ciphertext.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

impl Vault {
    /// Validate UI input before enrollment creates a no-downgrade marker.
    pub fn validate_password(password: &str) -> Result<(), VaultError> {
        password_policy(password)
    }
    /// Begin an edit that must be durable before anything acts on its result.
    ///
    /// Returns an independent unlocked copy — the *candidate*. The contract is
    /// one order and it is not optional: edit the candidate, take its
    /// `snapshot()`, store that ciphertext durably, and only then [`adopt`] it.
    /// Whatever the edit produced may not be acted on until the store
    /// succeeded.
    ///
    /// [`adopt`]: Self::adopt
    ///
    /// The reason is a payment. Recording a send into the live session and
    /// handing the transaction back in one step means a failure to store
    /// afterwards leaves the transaction in the caller's hands, already
    /// spendable, while the stored wallet still shows the coins unspent — and
    /// no way back except discarding the session and paying the full KDF cost
    /// again. The native adapter has always worked this way
    /// (`VaultStore::update`); this makes the same discipline available to a
    /// browser host, which has no filesystem to lean on and needs it more.
    pub fn begin_edit(&self) -> Result<Self, VaultError> {
        self.copy_session()
    }

    /// Adopt a candidate from [`begin_edit`] whose ciphertext is now stored.
    ///
    /// Refuses a candidate that is not this same wallet. In a host holding
    /// more than one vault, passing the wrong candidate would otherwise
    /// replace one wallet's session with another's, and the two would then
    /// disagree with their own stored ciphertext.
    ///
    /// [`begin_edit`]: Self::begin_edit
    pub fn adopt(&mut self, candidate: Self) -> Result<(), VaultError> {
        let current = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        let next = candidate.unlocked.as_ref().ok_or(VaultError::Locked)?;
        if next.wallet.network != current.wallet.network {
            return Err(VaultError::WrongNetwork);
        }
        if next.wallet.keys.seed != current.wallet.keys.seed
            || next.wallet.address() != current.wallet.address()
        {
            return Err(VaultError::InvalidWallet);
        }
        *self = candidate;
        Ok(())
    }

    /// Private transactional copy for the native persistence adapter. The
    /// candidate is never published until its encrypted state is durable.
    pub(crate) fn copy_session(&self) -> Result<Self, VaultError> {
        let current = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        let plain = Zeroizing::new(
            current
                .wallet
                .export_state()
                .map_err(|_| VaultError::InvalidWallet)?,
        );
        let wallet = Wallet::import_state(&plain).map_err(|_| VaultError::InvalidWallet)?;
        Ok(Self {
            sealed: self.sealed.clone(),
            unlocked: Some(Unlocked {
                key: Zeroizing::new(*current.key),
                prefix: current.prefix,
                wallet,
            }),
        })
    }

    /// Only for a host whose transactional edits have already been persisted.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn forget_unlocked(&mut self) {
        self.unlocked.take();
    }

    /// Copy an existing wallet into a separate in-memory encrypted session.
    /// The original is not altered or retired: migration is a host workflow.
    pub fn create(wallet: &Wallet, password: &str) -> Result<Self, VaultError> {
        password_policy(password)?;
        let plain = Zeroizing::new(
            wallet
                .export_state()
                .map_err(|_| VaultError::InvalidWallet)?,
        );
        if plain.len() > MAX_PLAINTEXT_BYTES {
            return Err(VaultError::SizeLimit);
        }
        let wallet = Wallet::import_state(&plain).map_err(|_| VaultError::InvalidWallet)?;
        let prefix = new_prefix(wallet.network)?;
        let key = derive_key(password, &prefix[15..31])?;
        let unlocked = Unlocked {
            key,
            prefix,
            wallet,
        };
        let sealed = encrypt(&unlocked)?;
        Ok(Self {
            sealed,
            unlocked: Some(unlocked),
        })
    }

    /// Parse size/version/profile before allocation or expensive password work.
    /// Header network is untrusted until unlock succeeds.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VaultError> {
        parse_header(bytes)?;
        Ok(Self {
            sealed: bytes.to_vec(),
            unlocked: None,
        })
    }

    pub fn is_locked(&self) -> bool {
        self.unlocked.is_none()
    }

    /// Last encrypted snapshot, safe to persist but possibly older than edits.
    pub fn sealed_bytes(&self) -> &[u8] {
        &self.sealed
    }

    pub fn wallet(&self) -> Result<&Wallet, VaultError> {
        self.unlocked
            .as_ref()
            .map(|u| &u.wallet)
            .ok_or(VaultError::Locked)
    }

    pub fn wallet_mut(&mut self) -> Result<&mut Wallet, VaultError> {
        self.unlocked
            .as_mut()
            .map(|u| &mut u.wallet)
            .ok_or(VaultError::Locked)
    }

    /// Authenticate first; publish no decrypted state until all validation passes.
    /// Wrong passwords and failures preserve both existing snapshot and lock state.
    pub fn unlock(&mut self, password: &str, expected: NetworkId) -> Result<(), VaultError> {
        if self.unlocked.is_some() {
            return Err(VaultError::AlreadyUnlocked);
        }
        let network = parse_header(&self.sealed)?;
        if network != expected {
            return Err(VaultError::WrongNetwork);
        }
        let mut prefix = [0u8; PREFIX_LEN];
        prefix.copy_from_slice(&self.sealed[..PREFIX_LEN]);
        let key = derive_key(password, &prefix[15..31])?;
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key[..]).map_err(|_| VaultError::Authentication)?;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&self.sealed[PREFIX_LEN..HEADER_LEN]),
                    Payload {
                        msg: &self.sealed[HEADER_LEN..],
                        aad: &self.sealed[..HEADER_LEN],
                    },
                )
                .map_err(|_| VaultError::Authentication)?,
        );
        let text = std::str::from_utf8(&plain).map_err(|_| VaultError::InvalidWallet)?;
        let wallet = Wallet::import_state(text).map_err(|_| VaultError::InvalidWallet)?;
        if wallet.network != expected {
            return Err(VaultError::WrongNetwork);
        }
        self.unlocked = Some(Unlocked {
            key,
            prefix,
            wallet,
        });
        Ok(())
    }

    /// Encrypt current state with a fresh nonce. Never rerun the password KDF
    /// for routine saves. On failure, the previous snapshot/session survives.
    pub fn snapshot(&mut self) -> Result<&[u8], VaultError> {
        if let Some(unlocked) = &self.unlocked {
            self.sealed = encrypt(unlocked)?;
        }
        Ok(&self.sealed)
    }

    /// Retain the latest encrypted snapshot, then drop keys and in-memory wallet.
    /// Callers still must persist the ciphertext and clear their own UI buffers.
    pub fn lock(&mut self) -> Result<(), VaultError> {
        self.snapshot()?;
        self.unlocked.take();
        Ok(())
    }

    /// Rotate salt and key as well as nonce. Does not invalidate old copies:
    /// backups encrypted with the old password remain decryptable with it.
    pub fn change_password(&mut self, password: &str) -> Result<(), VaultError> {
        let current = self.unlocked.as_ref().ok_or(VaultError::Locked)?;
        if current.prefix[10] != network_byte(current.wallet.network) {
            return Err(VaultError::WrongNetwork);
        }
        password_policy(password)?;
        let plain = Zeroizing::new(
            current
                .wallet
                .export_state()
                .map_err(|_| VaultError::InvalidWallet)?,
        );
        let wallet = Wallet::import_state(&plain).map_err(|_| VaultError::InvalidWallet)?;
        let prefix = new_prefix(wallet.network)?;
        let key = derive_key(password, &prefix[15..31])?;
        let replacement = Unlocked {
            key,
            prefix,
            wallet,
        };
        let sealed = encrypt(&replacement)?;
        self.unlocked = Some(replacement);
        self.sealed = sealed;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;
    const PASSWORD: &str = "public test password - not a wallet";
    fn wallet() -> Wallet {
        Wallet::in_memory(NetworkId::Devnet, WalletKeys::from_seed([73; 32]), 42)
    }

    #[test]
    fn roundtrip_lock_and_reopen_preserves_exact_wallet_state() {
        let original = wallet();
        let expected = original.export_state().unwrap();
        let mut vault = Vault::create(&original, PASSWORD).unwrap();
        assert_eq!(vault.wallet().unwrap().export_state().unwrap(), expected);
        assert!(!String::from_utf8_lossy(vault.sealed_bytes())
            .contains(&hex::encode(original.keys.seed)));
        vault.lock().unwrap();
        assert!(vault.is_locked());
        assert!(matches!(vault.wallet(), Err(VaultError::Locked)));
        assert!(matches!(vault.wallet_mut(), Err(VaultError::Locked)));
        let mut reopened = Vault::from_bytes(vault.sealed_bytes()).unwrap();
        reopened.unlock(PASSWORD, NetworkId::Devnet).unwrap();
        assert_eq!(reopened.wallet().unwrap().export_state().unwrap(), expected);
        assert_eq!(
            reopened.unlock(PASSWORD, NetworkId::Devnet),
            Err(VaultError::AlreadyUnlocked)
        );
        assert_eq!(original.export_state().unwrap(), expected);
    }

    #[test]
    fn wrong_password_network_and_tampering_never_publish_state() {
        let vault = Vault::create(&wallet(), PASSWORD).unwrap();
        let bytes = vault.sealed_bytes().to_vec();
        let mut locked = Vault::from_bytes(&bytes).unwrap();
        assert_eq!(
            locked.unlock(PASSWORD, NetworkId::Mainnet),
            Err(VaultError::WrongNetwork)
        );
        assert_eq!(
            locked.unlock("another public password", NetworkId::Devnet),
            Err(VaultError::Authentication)
        );
        assert!(locked.is_locked());
        assert_eq!(locked.sealed_bytes(), bytes);
        for offset in [15, PREFIX_LEN, HEADER_LEN, bytes.len() - 1] {
            let mut damaged = bytes.clone();
            damaged[offset] ^= 1;
            let mut locked = Vault::from_bytes(&damaged).unwrap();
            assert_eq!(
                locked.unlock(PASSWORD, NetworkId::Devnet),
                Err(VaultError::Authentication)
            );
            assert!(locked.is_locked());
            assert_eq!(locked.sealed_bytes(), damaged);
        }
        // A valid network tag is still authenticated, not just syntactically checked.
        let mut forged = bytes;
        forged[10] = 0;
        let mut locked = Vault::from_bytes(&forged).unwrap();
        assert_eq!(
            locked.unlock(PASSWORD, NetworkId::Mainnet),
            Err(VaultError::Authentication)
        );
    }

    #[test]
    fn snapshots_use_fresh_nonces_and_failed_operations_preserve_state() {
        let mut vault = Vault::create(&wallet(), PASSWORD).unwrap();
        let first = vault.sealed_bytes().to_vec();
        let second = vault.snapshot().unwrap().to_vec();
        assert_eq!(&first[..PREFIX_LEN], &second[..PREFIX_LEN]);
        assert_ne!(
            &first[PREFIX_LEN..HEADER_LEN],
            &second[PREFIX_LEN..HEADER_LEN]
        );
        assert_eq!(
            vault.change_password("short"),
            Err(VaultError::PasswordPolicy)
        );
        assert_eq!(vault.sealed_bytes(), second);
        vault.wallet_mut().unwrap().network = NetworkId::Mainnet;
        assert_eq!(vault.lock(), Err(VaultError::WrongNetwork));
        assert!(!vault.is_locked());
        assert_eq!(vault.sealed_bytes(), second);
    }

    #[test]
    fn password_rotation_changes_salt_and_old_password_does_not_unlock_new_copy() {
        let mut vault = Vault::create(&wallet(), PASSWORD).unwrap();
        let before = vault.sealed_bytes().to_vec();
        let expected = vault.wallet().unwrap().export_state().unwrap();
        vault
            .change_password("replacement public test password")
            .unwrap();
        assert_ne!(&before[15..31], &vault.sealed_bytes()[15..31]);
        vault.lock().unwrap();
        assert_eq!(
            vault.unlock(PASSWORD, NetworkId::Devnet),
            Err(VaultError::Authentication)
        );
        vault
            .unlock("replacement public test password", NetworkId::Devnet)
            .unwrap();
        assert_eq!(vault.wallet().unwrap().export_state().unwrap(), expected);
        // Password rotation is not revocation of historical backups.
        let mut old = Vault::from_bytes(&before).unwrap();
        old.unlock(PASSWORD, NetworkId::Devnet).unwrap();
    }

    #[test]
    fn parser_rejects_lengths_unknown_versions_profiles_and_networks_before_kdf() {
        let mut blob = vec![0; HEADER_LEN + TAG_LEN];
        blob[..PREFIX_LEN].copy_from_slice(&new_prefix(NetworkId::Devnet).unwrap());
        for length in 0..HEADER_LEN + TAG_LEN {
            assert!(Vault::from_bytes(&blob[..length]).is_err());
        }
        for index in [0, 8, 9, 10, 11] {
            let mut invalid = blob.clone();
            invalid[index] = 255;
            assert!(Vault::from_bytes(&invalid).is_err());
        }
        assert!(matches!(
            Vault::from_bytes(&vec![0; MAX_VAULT_BYTES + 1]),
            Err(VaultError::SizeLimit)
        ));
        assert!(matches!(
            Vault::from_bytes(b"{\"seed\":\"legacy plaintext\"}"),
            Err(VaultError::InvalidEnvelope)
        ));
    }

    #[test]
    fn password_policy_has_no_silent_normalization_or_truncation() {
        for password in [
            "".to_owned(),
            "tiny".into(),
            " ".repeat(20),
            "x".repeat(1025),
        ] {
            assert_eq!(password_policy(&password), Err(VaultError::PasswordPolicy));
        }
        assert_eq!(password_policy(" twelve chars "), Ok(()));
        assert_eq!(password_policy("öffentliche Testphrase"), Ok(()));
    }

    #[test]
    fn changed_wallet_state_survives_lock_and_saves_do_not_write_legacy_files() {
        let mut vault = Vault::create(&wallet(), PASSWORD).unwrap();
        vault
            .wallet_mut()
            .unwrap()
            .reserve_commits(&["42".repeat(32)])
            .unwrap();
        let expected = vault.wallet().unwrap().export_state().unwrap();
        vault.lock().unwrap();
        let saved = vault.sealed_bytes().to_vec();
        vault.lock().unwrap();
        assert_eq!(
            vault.sealed_bytes(),
            saved,
            "locking a locked session is idempotent"
        );
        vault.unlock(PASSWORD, NetworkId::Devnet).unwrap();
        assert_eq!(vault.wallet().unwrap().export_state().unwrap(), expected);
        assert!(!vault.wallet().unwrap().persist);
        assert!(vault.wallet().unwrap().seed_path.as_os_str().is_empty());
    }

    #[test]
    fn valid_authentication_does_not_allow_invalid_inner_state_or_network() {
        let vault = Vault::create(&wallet(), PASSWORD).unwrap();
        let unlocked = vault.unlocked.as_ref().unwrap();
        let original = unlocked.wallet.export_state().unwrap();
        let mut malformed: serde_json::Value = serde_json::from_str(&original).unwrap();
        malformed["v"] = 255.into();
        let mut cases = vec![
            ("not JSON".to_string(), VaultError::InvalidWallet),
            (malformed.to_string(), VaultError::InvalidWallet),
            (
                original.replace("devnet", "mainnet"),
                VaultError::WrongNetwork,
            ),
        ];
        // Authentication only proves who wrote the bytes. A newer writer may
        // have added custody rules this reader cannot safely discard.
        let mut nested: serde_json::Value = serde_json::from_str(&original).unwrap();
        nested["db"]["history"] = serde_json::json!([{
            "direction": "sent", "amount": 1, "fee": 1, "memo": "public fixture",
            "height": null, "txid": "public fixture", "timestamp": 1
        }]);
        nested["db"]["outputs"] = serde_json::json!([{
            // A Commitment is a newtype over [u8; 32], so it travels as a
            // 32-element array. `json!` takes an expression here; it does not
            // understand Rust's array-repeat syntax.
            "commit": vec![0u8; 32], "value": 1, "blind_hex": "00".repeat(32),
            "key_offset_hex": "00".repeat(32), "memo": "public fixture",
            "height": 0, "spent": false
        }]);
        assert!(
            Wallet::import_state(&nested.to_string()).is_ok(),
            "known-fields fixture must parse"
        );
        for pointer in ["/db", "/db/history/0", "/db/outputs/0"] {
            let mut future = nested.clone();
            future
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("future_safety_hold".into(), true.into());
            cases.push((future.to_string(), VaultError::InvalidWallet));
        }
        for (text, expected) in cases {
            let nonce = random_bytes::<24>().unwrap();
            let mut header = unlocked.prefix.to_vec();
            header.extend_from_slice(&nonce);
            let cipher = XChaCha20Poly1305::new_from_slice(&unlocked.key[..]).unwrap();
            let ciphertext = cipher
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: text.as_bytes(),
                        aad: &header,
                    },
                )
                .unwrap();
            header.extend_from_slice(&ciphertext);
            let mut locked = Vault::from_bytes(&header).unwrap();
            assert_eq!(locked.unlock(PASSWORD, NetworkId::Devnet), Err(expected));
            assert!(locked.is_locked());
        }
    }

    #[test]
    fn legacy_import_refuses_unknown_network_version_duplicate_or_missing_fields() {
        let original = wallet().export_state().unwrap();
        let value: serde_json::Value = serde_json::from_str(&original).unwrap();
        for key in ["v", "network", "seed", "db"] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                Wallet::import_state(&missing.to_string()).is_err(),
                "missing {key}"
            );
        }
        let mut future = value.clone();
        future["v"] = 2.into();
        assert!(Wallet::import_state(&future.to_string()).is_err());
        assert!(Wallet::import_state(&original.replace("devnet", "typonet")).is_err());
        assert!(Wallet::import_state(&original.replacen('{', "{\"v\":1,", 1)).is_err());
        assert!(Wallet::import_state("null").is_err());
    }

    #[test]
    fn kdf_matches_independent_argon2id_vector() {
        // Public test password/salt. Independently computed with Python
        // cryptography's Argon2id (OpenSSL), not this Rust implementation.
        let salt: Vec<u8> = (0..16).collect();
        let key = derive_key("Nightfall public test password", &salt).unwrap();
        assert_eq!(
            hex::encode(&key[..]),
            "4c8faca752ae945090b55b03113db720b64a2466ab165625496dbc39a83eaca1"
        );
    }
}
