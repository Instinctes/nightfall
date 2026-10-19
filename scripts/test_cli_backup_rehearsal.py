#!/usr/bin/env python3
"""Isolated, offline Devnet CLI seed-backup/recovery regression test.

Usage: python3 scripts/test_cli_backup_rehearsal.py --wallet /path/to/nightfall-wallet
Only disposable keys are created. Never takes an existing datadir or seed path.
"""

import argparse
import os
from pathlib import Path
import secrets
import shutil
import stat
import subprocess
import tempfile
import unittest

SEED = "rehearsal.seed"
DB = SEED + ".outputs.json"
WALLET = None


class BackupRehearsal(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if WALLET is None or not WALLET.is_file():
            raise RuntimeError("Provide an existing nightfall-wallet binary with --wallet")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="nightfall-devnet-rehearsal-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.origin = self.root / "original"
        self.origin.mkdir(mode=0o700)
        self.cli(self.origin, "init")
        self.address = self.cli(self.origin, "address").stdout.strip()
        self.assertTrue(self.address.startswith("nf1") and len(self.address) == 139,
                        "CLI did not produce a valid-length test address")
        self.assertTrue((self.origin / SEED).is_file())
        self.assertTrue((self.origin / DB).is_file())

    def cli(self, datadir, action, success=True):
        # No network operations: only init, address and balance are allowed.
        self.assertIn(action, ("init", "address", "balance"))
        result = subprocess.run(
            [str(WALLET), "--network", "devnet", "--datadir", str(datadir),
             "--seed-file", SEED, "--rpc", "127.0.0.1:0", action],
            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            timeout=25, check=False,
        )
        if success:
            self.assertEqual(result.returncode, 0,
                             "disposable Devnet wallet command failed: " + action)
        else:
            self.assertNotEqual(result.returncode, 0,
                                "invalid disposable backup was accepted: " + action)
        return result

    def restored_dir(self, copy_seed=True, copy_db=True):
        target = self.root / "restored"
        target.mkdir(mode=0o700)
        for name, enabled in ((SEED, copy_seed), (DB, copy_db)):
            if enabled:
                shutil.copyfile(self.origin / name, target / name)
                if name == SEED:
                    (target / name).chmod(0o600)
        return target

    def test_full_backup_recovers_same_identity_and_zero_balance(self):
        target = self.restored_dir()
        self.assertEqual(self.cli(target, "address").stdout.strip(), self.address)
        balance = self.cli(target, "balance").stdout.splitlines()
        self.assertTrue(balance and balance[0].strip() == "0.00000000",
                        "restored disposable wallet has unexpected balance")
        if os.name == "posix":
            self.assertEqual(stat.S_IMODE((target / SEED).stat().st_mode), 0o600)

    def test_missing_seed_with_database_fails_closed(self):
        target = self.restored_dir(copy_seed=False)
        before = (target / DB).read_bytes()
        self.cli(target, "address", success=False)
        self.assertFalse((target / SEED).exists(), "a replacement seed was generated")
        self.assertEqual((target / DB).read_bytes(), before)

    def test_truncated_seed_fails_without_replacement(self):
        target = self.restored_dir()
        bad = b"ab" * 31  # 31 bytes, not a valid 32-byte seed
        (target / SEED).write_bytes(bad)
        self.cli(target, "address", success=False)
        self.assertEqual((target / SEED).read_bytes(), bad)

    def test_wrong_but_well_formed_seed_is_detected_by_address_comparison(self):
        target = self.restored_dir()
        original_seed = (target / SEED).read_bytes()
        wrong_seed = secrets.token_hex(32).encode("ascii")
        while wrong_seed == original_seed:
            wrong_seed = secrets.token_hex(32).encode("ascii")
        (target / SEED).write_bytes(wrong_seed)
        (target / SEED).chmod(0o600)
        # Identity verification is essential: a syntactically valid wrong seed
        # can open a different empty wallet without indicating the mismatch.
        restored_address = self.cli(target, "address").stdout.strip()
        self.assertNotEqual(restored_address, self.address)

    def test_interrupted_database_save_is_rejected(self):
        target = self.restored_dir()
        pending = target / (DB + ".tmp")
        pending.write_text("incomplete disposable test write", encoding="utf-8")
        self.cli(target, "address", success=False)
        self.assertTrue(pending.is_file(), "CLI unexpectedly removed recovery evidence")

    def test_seed_only_restores_identity_but_not_scan_history(self):
        target = self.restored_dir(copy_db=False)
        self.assertFalse((target / DB).exists())
        self.assertEqual(self.cli(target, "address").stdout.strip(), self.address)
        self.assertTrue((target / DB).exists())
        # This proves key identity only. It does NOT prove past payments are
        # recovered: rescan/reconciliation requires independently validated chain data.


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wallet", required=True,
                        help="Path to an already built nightfall-wallet executable")
    args = parser.parse_args()
    WALLET = Path(args.wallet).expanduser().resolve()
    unittest.main(argv=[__file__], verbosity=2)
