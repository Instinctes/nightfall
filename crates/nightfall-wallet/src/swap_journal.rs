//! Encrypted-snapshot payload for the future Vault swap executor.
//!
//! This is a storage boundary, not an enabled swap engine. A checkpoint combines
//! handshake secrets and execution state in ONE revision. The host must edit
//! through VaultStore::update and act only after its durable commit succeeds.
//! No automatic migration, deletion, backup replay or plaintext writer here.

use crate::Wallet;
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_JOURNAL_BYTES: usize = 8 * 1024 * 1024;
/// Legacy format/import bound. Do not lower this: old recovery material must
/// remain readable even if an older writer overcommitted its growth budget.
pub const MAX_RECORDS: usize = 64;
/// Each newly admitted record reserves its maximum payload size. Shrinking a
/// checkpoint or declaring it terminal does not release this storage budget:
/// the storage layer cannot establish irreversible settlement on either chain.
pub const MAX_RESERVED_RECORDS: usize = MAX_JOURNAL_BYTES / MAX_RECORD_BYTES;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    id: String,
    revision: u64,
    payload: String,
}

impl Drop for Record {
    fn drop(&mut self) {
        self.payload.zeroize();
    }
}

/// Intentionally no Debug or Clone: the payload contains spend secrets.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    v: u32,
    records: Vec<Record>,
}

impl Default for Journal {
    fn default() -> Self {
        Self {
            v: 1,
            records: Vec::new(),
        }
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}

impl Journal {
    pub(crate) fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.v == 1,
            "Unsupported swap journal format. Preserve the original vault."
        );
        ensure!(
            self.records.len() <= MAX_RECORDS,
            "Too many swap recovery records."
        );
        let mut ids = std::collections::BTreeSet::new();
        let mut size = 0usize;
        for record in &self.records {
            ensure!(
                valid_id(&record.id) && ids.insert(&record.id),
                "Invalid or duplicate swap recovery ID."
            );
            ensure!(record.revision > 0, "Invalid swap recovery revision.");
            ensure!(
                !record.payload.is_empty() && record.payload.len() <= MAX_RECORD_BYTES,
                "Invalid swap recovery payload size."
            );
            size = size
                .checked_add(record.payload.len())
                .ok_or_else(|| anyhow::anyhow!("Swap journal size overflow."))?;
        }
        ensure!(
            size <= MAX_JOURNAL_BYTES,
            "Swap journal exceeds its size limit."
        );
        Ok(())
    }
}

impl Wallet {
    pub fn has_swap_recovery(&self) -> bool {
        !self.db.swap_journal.is_empty()
    }

    /// IDs only; does not expose the serialized secrets.
    pub fn swap_recovery_ids(&self) -> Vec<&str> {
        self.db
            .swap_journal
            .records
            .iter()
            .map(|record| record.id.as_str())
            .collect()
    }

    /// The host must not log, display or write this plaintext outside Vault.
    pub fn swap_checkpoint(&self, id: &str) -> Option<(u64, &str)> {
        self.db
            .swap_journal
            .records
            .iter()
            .find(|record| record.id == id)
            .map(|record| (record.revision, record.payload.as_str()))
    }

    /// Compare-and-swap prevents stale worker snapshots overwriting newer state.
    /// expected_revision=0 creates; updates require the exact current revision.
    /// Admission reserves MAX_RECORD_BYTES for every record, including any
    /// terminal records. Commit this reservation before releasing a packet or
    /// funding transaction. The budget bounds journal payload bytes, not the
    /// complete serialized Vault (which also contains escaping and wallet data).
    /// Older overcommitted journals remain readable and updatable under their
    /// actual size limit, but cannot admit new records until capacity is safe.
    /// No deletion API: retiring recovery material needs a separate proof of
    /// terminal settlement and must not be inferred from missing chain data.
    pub fn put_swap_checkpoint(
        &mut self,
        id: &str,
        expected_revision: u64,
        payload: &str,
    ) -> anyhow::Result<u64> {
        ensure!(!self.persist, "Swap recovery records require an encrypted Vault transaction; plaintext storage is refused.");
        ensure!(valid_id(id), "Invalid swap recovery ID.");
        ensure!(
            !payload.is_empty() && payload.len() <= MAX_RECORD_BYTES,
            "Invalid swap recovery payload size."
        );
        self.db.swap_journal.validate()?;
        let journal = &mut self.db.swap_journal;
        let index = journal.records.iter().position(|record| record.id == id);
        let current = index.map_or(0, |i| journal.records[i].revision);
        ensure!(
            current == expected_revision,
            "Stale swap checkpoint. Reload the durable revision before continuing."
        );
        let revision = current
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Swap revision exhausted."))?;
        ensure!(
            index.is_some() || journal.records.len() < MAX_RESERVED_RECORDS,
            "Swap recovery capacity is reserved. A new swap cannot safely reserve its maximum checkpoint size. Existing recovery records were preserved."
        );
        let size: usize = journal
            .records
            .iter()
            .filter(|r| r.id != id)
            .map(|r| r.payload.len())
            .sum();
        ensure!(
            size.checked_add(payload.len())
                .is_some_and(|n| n <= MAX_JOURNAL_BYTES),
            "Swap journal exceeds its size limit."
        );
        let record = Record {
            id: id.into(),
            revision,
            payload: payload.into(),
        };
        if let Some(index) = index {
            journal.records[index] = record;
        } else {
            journal.records.push(record);
        }
        Ok(revision)
    }

    /// Remove a recovery record whose swap is over.
    ///
    /// This is the deletion the note on `put_swap_checkpoint` said would need a
    /// separate proof of terminal settlement, and the proof is not built here:
    /// this layer holds an opaque payload and has no idea what a settled swap
    /// looks like. It enforces the two things it *can* enforce, and the caller
    /// owes the third.
    ///
    /// What is enforced here: the record may only be removed from inside an
    /// encrypted Vault transaction, and only at the exact revision the caller
    /// last read. A caller working from a stale copy cannot delete the newer
    /// state it has not seen.
    ///
    /// What the caller owes: evidence that the swap reached a terminal state,
    /// taken from the record itself. It must never be inferred from the chain
    /// having nothing to show — an RPC outage, a pruned node and a reorg all
    /// look exactly like "settled" from here, and the record is the only way
    /// back if they are not.
    pub fn retire_swap_checkpoint(
        &mut self,
        id: &str,
        expected_revision: u64,
    ) -> anyhow::Result<()> {
        ensure!(
            !self.persist,
            "Swap recovery records require an encrypted Vault transaction; plaintext storage is refused."
        );
        ensure!(valid_id(id), "Invalid swap recovery ID.");
        let journal = &mut self.db.swap_journal;
        let index = journal
            .records
            .iter()
            .position(|record| record.id == id)
            .context("No swap recovery record with that ID.")?;
        ensure!(
            journal.records[index].revision == expected_revision,
            "Stale swap checkpoint. Reload the durable revision before retiring it."
        );
        journal.records.remove(index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nightfall_crypto::WalletKeys;
    use nightfall_types::NetworkId;
    const ID: &str = "00000000-0000-4000-8000-000000000001";

    fn wallet() -> Wallet {
        Wallet::in_memory(NetworkId::Devnet, WalletKeys::from_seed([0; 32]), 0)
    }

    fn id(number: usize) -> String {
        format!("00000000-0000-4000-8000-{number:012x}")
    }

    /// Retiring a record needs the revision the caller actually read.
    ///
    /// The capacity rule is the reason this matters beyond tidiness: a journal
    /// full of finished swaps refuses to admit a new one, so without a way out
    /// the wallet eventually stops being able to start a swap at all. What it
    /// must not become is a way to lose the only copy of a live swap's
    /// secrets, which is why the revision has to match and why the caller in
    /// `nightfall-core` proves the swap is finished before calling.
    #[test]
    fn retiring_a_record_needs_the_revision_that_was_read() {
        let mut wallet = wallet();
        let revision = wallet.put_swap_checkpoint(ID, 0, "first").unwrap();
        let revision = wallet.put_swap_checkpoint(ID, revision, "second").unwrap();

        // A stale holder cannot delete state it has never seen.
        assert!(wallet.retire_swap_checkpoint(ID, revision - 1).is_err());
        assert!(wallet.has_swap_recovery());
        assert_eq!(wallet.swap_checkpoint(ID).unwrap().1, "second");

        // An ID that was never there is not silently "already gone".
        assert!(wallet.retire_swap_checkpoint(&id(9), revision).is_err());

        wallet.retire_swap_checkpoint(ID, revision).unwrap();
        assert!(!wallet.has_swap_recovery());
        assert!(wallet.swap_checkpoint(ID).is_none());
        // And retiring the same record twice is an error, not a silent no-op.
        assert!(wallet.retire_swap_checkpoint(ID, revision).is_err());
    }

    /// Retiring frees the reservation, so the next swap can be admitted.
    #[test]
    fn a_retired_record_gives_its_reserved_capacity_back() {
        let mut wallet = wallet();
        let mut revisions = Vec::new();
        for n in 0..MAX_RESERVED_RECORDS {
            revisions.push((id(n), wallet.put_swap_checkpoint(&id(n), 0, "x").unwrap()));
        }
        // Full: one more swap cannot reserve its maximum checkpoint size.
        assert!(wallet
            .put_swap_checkpoint(&id(MAX_RESERVED_RECORDS), 0, "x")
            .is_err());
        let (first, revision) = revisions.remove(0);
        wallet.retire_swap_checkpoint(&first, revision).unwrap();
        wallet
            .put_swap_checkpoint(&id(MAX_RESERVED_RECORDS), 0, "x")
            .unwrap();
        // The records that were not retired are untouched.
        for (id, _) in &revisions {
            assert!(wallet.swap_checkpoint(id).is_some());
        }
    }

    /// Plaintext storage is refused for deletion exactly as it is for writing.
    #[test]
    fn a_plaintext_wallet_can_neither_write_nor_retire_recovery_records() {
        let mut wallet = wallet();
        wallet.put_swap_checkpoint(ID, 0, "first").unwrap();
        wallet.persist = true;
        assert!(wallet.put_swap_checkpoint(ID, 1, "second").is_err());
        assert!(wallet.retire_swap_checkpoint(ID, 1).is_err());
        wallet.persist = false;
        assert_eq!(wallet.swap_checkpoint(ID).unwrap().1, "first");
    }

    #[test]
    fn admission_reserves_every_records_maximum_growth_before_funding() {
        let mut wallet = wallet();
        for index in 0..MAX_RESERVED_RECORDS {
            wallet
                .put_swap_checkpoint(&id(index), 0, "small first checkpoint")
                .unwrap();
        }
        let before = wallet.export_state().unwrap();
        assert!(wallet
            .put_swap_checkpoint(&id(MAX_RESERVED_RECORDS), 0, "x")
            .is_err());
        assert_eq!(wallet.export_state().unwrap(), before);

        // All admitted records can grow at once, including after a restart.
        let mut wallet = Wallet::import_state(&before).unwrap();
        let maximum = "x".repeat(MAX_RECORD_BYTES);
        for index in 0..MAX_RESERVED_RECORDS {
            assert_eq!(
                wallet.put_swap_checkpoint(&id(index), 1, &maximum).unwrap(),
                2
            );
        }
        wallet.db.swap_journal.validate().unwrap();
        assert_eq!(
            wallet
                .db
                .swap_journal
                .records
                .iter()
                .map(|record| record.payload.len())
                .sum::<usize>(),
            MAX_JOURNAL_BYTES
        );

        // A small/terminal checkpoint still owns its full budget; a reorg may
        // require recovery material to grow again. Storage cannot infer finality.
        wallet
            .put_swap_checkpoint(&id(0), 2, "terminal fixture")
            .unwrap();
        assert!(wallet
            .put_swap_checkpoint(&id(MAX_RESERVED_RECORDS), 0, "x")
            .is_err());
        assert_eq!(wallet.put_swap_checkpoint(&id(0), 3, &maximum).unwrap(), 4);
    }

    #[test]
    fn older_overcommitted_journals_remain_readable_without_new_admission() {
        let mut wallet = wallet();
        // Simulate the previous writer, which admitted many small records.
        for index in 0..MAX_RECORDS {
            wallet.db.swap_journal.records.push(Record {
                id: id(index),
                revision: 1,
                payload: "existing public fixture".into(),
            });
        }
        let state = wallet.export_state().unwrap();
        let mut reopened = Wallet::import_state(&state).unwrap();
        assert_eq!(reopened.swap_recovery_ids().len(), MAX_RECORDS);
        assert!(reopened
            .put_swap_checkpoint(&id(MAX_RECORDS), 0, "new")
            .is_err());
        assert_eq!(reopened.export_state().unwrap(), state);

        let maximum = "x".repeat(MAX_RECORD_BYTES);
        for index in 0..MAX_RESERVED_RECORDS - 1 {
            reopened
                .put_swap_checkpoint(&id(index), 1, &maximum)
                .unwrap();
        }
        let before = reopened.export_state().unwrap();
        assert!(reopened
            .put_swap_checkpoint(&id(MAX_RESERVED_RECORDS - 1), 1, &maximum)
            .is_err());
        assert_eq!(reopened.export_state().unwrap(), before);
        // Smaller recovery updates still work; capacity failure destroys none
        // of the older sessions or their CAS revision.
        assert_eq!(
            reopened
                .put_swap_checkpoint(&id(MAX_RESERVED_RECORDS - 1), 1, "recovery update")
                .unwrap(),
            2
        );
        assert_eq!(reopened.swap_recovery_ids().len(), MAX_RECORDS);
    }

    #[test]
    fn exhausted_or_stale_revision_cannot_replace_recovery_material() {
        let mut wallet = wallet();
        wallet.put_swap_checkpoint(ID, 0, "public fixture").unwrap();
        wallet.db.swap_journal.records[0].revision = u64::MAX;
        let before = wallet.export_state().unwrap();
        for revision in [0, 1, u64::MAX] {
            assert!(wallet
                .put_swap_checkpoint(ID, revision, "replacement")
                .is_err());
            assert_eq!(wallet.export_state().unwrap(), before);
        }
    }

    #[test]
    fn absent_journal_keeps_old_state_compatible_and_stale_edits_fail() {
        let mut wallet = wallet();
        assert!(!wallet.export_state().unwrap().contains("swap_journal"));
        assert_eq!(
            wallet
                .put_swap_checkpoint(ID, 0, "public fixture: checkpoint 1")
                .unwrap(),
            1
        );
        let before = wallet.export_state().unwrap();
        assert!(wallet
            .put_swap_checkpoint(ID, 0, "stale replacement")
            .is_err());
        assert_eq!(wallet.export_state().unwrap(), before);
        let mut reopened = Wallet::import_state(&before).unwrap();
        assert_eq!(
            reopened.swap_checkpoint(ID),
            Some((1, "public fixture: checkpoint 1"))
        );
        assert_eq!(
            reopened
                .put_swap_checkpoint(ID, 1, "public fixture: checkpoint 2")
                .unwrap(),
            2
        );
        assert_eq!(reopened.swap_recovery_ids(), vec![ID]);
    }

    #[test]
    fn invalid_large_and_plaintext_edits_leave_old_state_untouched() {
        let mut wallet = wallet();
        let before = wallet.export_state().unwrap();
        for (id, payload) in [
            ("../escape", "fixture".into()),
            (ID, String::new()),
            (ID, "x".repeat(MAX_RECORD_BYTES + 1)),
        ] {
            assert!(wallet.put_swap_checkpoint(id, 0, &payload).is_err());
            assert_eq!(wallet.export_state().unwrap(), before);
        }
        wallet.persist = true;
        assert!(wallet.put_swap_checkpoint(ID, 0, "fixture").is_err());
        assert_eq!(wallet.export_state().unwrap(), before);
    }

    #[test]
    fn rescan_cannot_discard_swap_recovery() {
        let mut wallet = wallet();
        wallet.put_swap_checkpoint(ID, 0, "public fixture").unwrap();
        let before = wallet.export_state().unwrap();
        assert!(wallet.reset_scan().is_err());
        assert_eq!(wallet.export_state().unwrap(), before);
    }

    #[test]
    fn plaintext_open_rejects_unknown_journal_version_even_when_empty() {
        let root = std::env::temp_dir().join(format!(
            "nf-swap-journal-open-{}",
            hex::encode(rand::random::<[u8; 16]>())
        ));
        std::fs::create_dir(&root).unwrap();
        let wallet = Wallet::open(&root, NetworkId::Devnet, "test.seed").unwrap();
        let mut db = serde_json::to_value(&wallet.db).unwrap();
        db["swap_journal"] = serde_json::json!({"v": 2, "records": []});
        let bytes = serde_json::to_vec(&db).unwrap();
        let path = wallet.db_path.clone();
        drop(wallet);
        std::fs::write(&path, &bytes).unwrap();
        assert!(Wallet::open(&root, NetworkId::Devnet, "test.seed").is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_duplicate_and_invalid_journals_fail_import() {
        let mut wallet = wallet();
        wallet.put_swap_checkpoint(ID, 0, "public fixture").unwrap();
        let base: serde_json::Value =
            serde_json::from_str(&wallet.export_state().unwrap()).unwrap();
        for case in 0..5 {
            let mut state = base.clone();
            let journal = &mut state["db"]["swap_journal"];
            match case {
                0 => journal["v"] = 2.into(),
                1 => {
                    let first = journal["records"][0].clone();
                    journal["records"].as_array_mut().unwrap().push(first);
                }
                2 => journal["records"][0]["revision"] = 0.into(),
                3 => journal["records"][0]["id"] = "not-an-id".into(),
                _ => journal["unknown"] = true.into(),
            }
            assert!(
                Wallet::import_state(&state.to_string()).is_err(),
                "case {case}"
            );
        }
    }
}
