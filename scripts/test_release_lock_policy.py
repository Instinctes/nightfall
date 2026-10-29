"""Regression guard for the tag-push release dependency lock policy.

This asserts GitHub Actions release build inputs, not byte-for-byte
reproducibility or a security audit. No network or third-party parser needed.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parent.parent
RELEASE = ROOT / ".github" / "workflows" / "release.yml"
MACOS = ROOT / "scripts" / "build-macos-dmgs.sh"
LOCK = ROOT / "Cargo.lock"


def check_policy(release: str, macos: str, lock: str) -> None:
    if not re.search(r"(?m)^version\s*=\s*4\s*$", lock) or "[[package]]" not in lock:
        raise ValueError("Cargo.lock missing or invalid")
    headings = list(re.finditer(r"(?m)^  (windows|linux|macos|publish):\s*$", release))
    names = [h.group(1) for h in headings]
    if names != ["windows", "linux", "macos", "publish"]:
        raise ValueError("unexpected release job structure")
    for i, name in enumerate(names):
        if name == "publish":
            continue
        block = release[headings[i].end():headings[i + 1].start()]
        commands = []
        for line in block.splitlines():
            if line.lstrip().startswith("#"):
                continue
            match = re.search(r"\bcargo\s+(test|build|fetch)\b([^#\n]*)", line)
            if match:
                commands.append((match.group(1), match.group(2).split()))
        kinds = [kind for kind, _ in commands]
        required = {"test", "build"} if name != "macos" else {"test", "fetch"}
        if not required.issubset(kinds):
            raise ValueError(f"{name}: required Cargo command absent: {required - set(kinds)}")
        for kind, args in commands:
            if "--locked" not in args:
                raise ValueError(f"{name}: unlocked cargo {kind}")
    # macOS's actual release build lives in this script, not release.yml.
    builds = [line for line in macos.splitlines()
              if not line.lstrip().startswith("#") and re.search(r"\bcargo\s+build\b", line)]
    if not builds or any("--locked" not in line.split() for line in builds):
        raise ValueError("macos: release build must use --locked")


class ReleaseLockPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = RELEASE.read_text(encoding="utf-8")
        cls.macos = MACOS.read_text(encoding="utf-8")
        cls.lock = LOCK.read_text(encoding="utf-8")

    def test_current_release_uses_locked_dependencies_everywhere(self):
        check_policy(self.workflow, self.macos, self.lock)

    def test_each_platform_test_fails_if_unlocking_is_reintroduced(self):
        needle = "run: cargo test --locked --workspace"
        occurrences = [m.start() for m in re.finditer(re.escape(needle), self.workflow)]
        self.assertEqual(len(occurrences), 3)
        for ordinal, at in enumerate(occurrences):
            mutated = (self.workflow[:at] + needle.replace(" --locked", "")
                       + self.workflow[at + len(needle):])
            with self.subTest(platform=ordinal), self.assertRaisesRegex(ValueError, "unlocked cargo test"):
                check_policy(mutated, self.macos, self.lock)

    def test_each_direct_build_fails_if_unlocking_is_reintroduced(self):
        needle = "cargo build --locked --release --bins -p nightfall-core -p nightfall-node -p nightfall-wallet -p nightfall-nodekit"
        occurrences = [m.start() for m in re.finditer(re.escape(needle), self.workflow)]
        self.assertEqual(len(occurrences), 2)
        for ordinal, at in enumerate(occurrences):
            mutated = (self.workflow[:at] + needle.replace(" --locked", "")
                       + self.workflow[at + len(needle):])
            with self.subTest(build=ordinal), self.assertRaisesRegex(ValueError, "unlocked cargo build"):
                check_policy(mutated, self.macos, self.lock)

    def test_macos_fetch_cannot_drop_lock(self):
        mutated = self.workflow.replace("cargo fetch --locked", "cargo fetch", 1)
        self.assertNotEqual(mutated, self.workflow)
        with self.assertRaisesRegex(ValueError, "unlocked cargo fetch"):
            check_policy(mutated, self.macos, self.lock)

    def test_macos_actual_build_cannot_drop_lock(self):
        mutated = self.macos.replace("cargo build --offline --locked", "cargo build --offline", 1)
        self.assertNotEqual(mutated, self.macos)
        with self.assertRaisesRegex(ValueError, "macos: release build"):
            check_policy(self.workflow, mutated, self.lock)

    def test_missing_lockfile_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Cargo.lock"):
            check_policy(self.workflow, self.macos, "")


if __name__ == "__main__":
    unittest.main()
