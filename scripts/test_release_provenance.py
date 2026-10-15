import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from check_release_provenance import expected_assets, verify, verify_checksums


class ReleaseProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.folder = Path(self.tmp.name)
        self.tag = "v1.0.7"
        self.binaries, self.sums = expected_assets(self.tag, "linux")
        for name in self.binaries:
            (self.folder / name).write_bytes(b"nightfall fixture: " + name.encode())
        self.write_manifest()

    def write_manifest(self):
        entries = []
        for name in self.binaries:
            digest = hashlib.sha256((self.folder / name).read_bytes()).hexdigest()
            entries.append(f"{digest}  {name}")
        (self.folder / self.sums).write_bytes(("\n".join(entries) + "\n").encode())

    def test_good_mock_signer(self):
        f = Mock(return_value=Mock(returncode=0, stderr=""))
        verify(self.folder, self.tag, "linux", run=f)
        self.assertEqual(f.call_count, 4)
        for args, _ in f.call_args_list:
            self.assertIn("--source-ref", args[0])
            self.assertIn("refs/tags/v1.0.7", args[0])
            self.assertIn("--signer-workflow", args[0])

    def test_signature_failure_rejects(self):
        f = Mock(return_value=Mock(returncode=1, stderr="no attestation"))
        with self.assertRaisesRegex(ValueError, "signed provenance"):
            verify(self.folder, self.tag, "linux", run=f)

    def test_tampering_rejects(self):
        (self.folder / self.binaries[1]).write_bytes(b"replaced executable")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            verify_checksums(self.folder, self.binaries, self.sums)

    def test_duplicate_rejects(self):
        p = self.folder / self.sums
        p.write_bytes(p.read_bytes() + p.read_bytes().splitlines()[0] + b"\n")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            verify_checksums(self.folder, self.binaries, self.sums)

    def test_missing_rejects(self):
        (self.folder / self.binaries[0]).unlink()
        with self.assertRaises(ValueError):
            verify(self.folder, self.tag, "linux", run=Mock())

    def test_windows_expected_names(self):
        files, sums = expected_assets("v1.2.3", "windows")
        self.assertEqual(sums, "SHA256SUMS-1.2.3-windows.txt")
        self.assertTrue(all(f.endswith("-windows-x64.exe") for f in files))

    def test_tag_injection_rejects(self):
        for bad in ("main", "v1.2", "v1.2.3;echo", "v1.2.3/evil", "v1.2.3\n"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                expected_assets(bad, "linux")

    def test_symlink_rejects(self):
        victim = self.folder / self.binaries[0]
        victim.unlink()
        victim.symlink_to(self.folder / self.binaries[1])
        with self.assertRaisesRegex(ValueError, "non-symlink"):
            verify_checksums(self.folder, self.binaries, self.sums)

    def test_crlf_rejects(self):
        p = self.folder / self.sums
        p.write_bytes(p.read_bytes().replace(b"\n", b"\r\n"))
        with self.assertRaisesRegex(ValueError, "control"):
            verify_checksums(self.folder, self.binaries, self.sums)

    def test_extraneous_file_rejects(self):
        (self.folder / "unexpected").write_bytes(b"oops")
        with self.assertRaisesRegex(ValueError, "unexpected/missing"):
            verify(self.folder, self.tag, "linux", run=Mock())


if __name__ == "__main__":
    unittest.main()
