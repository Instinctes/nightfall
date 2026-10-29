"""No-root Node Kit source-identity and dependency-lock regression tests.

No official installer is run. Only the read-only source-identification
fragment is executed against disposable local Git repositories.
"""
import hashlib
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
INSTALLERS = (
    ROOT / "scripts/install-node-linux.sh",
    ROOT / "scripts/install-seed-node-linux.sh",
)


def validate_policy(source):
    clone = 'git clone --depth 1 --branch "$NIGHTFALL_RELEASE_TAG" "$REPO" "$BUILD/nightfall"'
    revision = 'SOURCE_REV="$(git -C "$BUILD/nightfall" rev-parse --verify HEAD)"'
    lock = 'LOCK_SHA="$(sha256sum "$BUILD/nightfall/Cargo.lock" | awk \'{print $1}\')"'
    revisions = [
        ('NIGHTFALL_RELEASE_TAG="v1.0.12"', "pinned release tag"),
        ('NIGHTFALL_RELEASE_SHA="5819f6cbf057bb48118a0616bacd6c25e2b576a4"', "pinned release commit"),
        ('[ "$SOURCE_REV" = "$NIGHTFALL_RELEASE_SHA" ] || die "release tag does not match pinned commit"', "fail-closed tag verification"),
        (clone, "source clone"),
        (revision, "Git revision capture"),
        (lock, "Cargo.lock checksum capture"),
        ('info "source commit: $SOURCE_REV"', "Git revision output"),
        ('info "Cargo.lock SHA256: $LOCK_SHA"', "Cargo.lock checksum output"),
    ]
    for needle, name in revisions:
        if source.count(needle) != 1:
            raise ValueError(f"missing or duplicated {name}")
    if not (source.index(clone) < source.index(revision) <
            source.index(lock) < source.index("JOBS=1")):
        raise ValueError("source identity not captured before the build")
    builds = [
        line for line in source.splitlines()
        if not line.lstrip().startswith("#") and re.search(r"\bcargo\s+build\b", line)
    ]
    if len(builds) != 1:
        raise ValueError("expected exactly one active cargo build")
    args = shlex.split(builds[0].split("cargo build", 1)[1].split(")", 1)[0])
    if "--locked" not in args:
        raise ValueError("installer build must use --locked")
    if "nightfall-node" not in args or "--release" not in args:
        raise ValueError("installer must build the release node")


def identity_fragment(source):
    start = source.index('SOURCE_REV="')
    end = source.index("JOBS=1", start)
    return source[start:end]


class InstallerSourcePolicyTests(unittest.TestCase):
    def test_official_installers_require_locked_build_and_source_evidence(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name):
                validate_policy(path.read_text(encoding="utf-8"))

    def test_mutations_fail_closed(self):
        for path in INSTALLERS:
            source = path.read_text(encoding="utf-8")
            mutations = {
                "unpinned branch": source.replace('--branch "$NIGHTFALL_RELEASE_TAG" ', "", 1),
                "tag check disabled": source.replace('[ "$SOURCE_REV" = "$NIGHTFALL_RELEASE_SHA" ] || die "release tag does not match pinned commit"', ":", 1),
                "unlocked build": source.replace(
                    "cargo build --locked", "cargo build", 1
                ),
                "revision omitted": source.replace(
                    "rev-parse --verify HEAD", "rev-parse --verify HEAD~1", 1
                ),
                "lock checksum omitted": source.replace(
                    'sha256sum "$BUILD/nightfall/Cargo.lock"',
                    'sha256sum "$BUILD/nightfall/other.lock"', 1
                ),
                "revision output omitted": source.replace(
                    'info "source commit: $SOURCE_REV"', ':', 1
                ),
            }
            for name, broken in mutations.items():
                with self.subTest(installer=path.name, mutation=name):
                    self.assertNotEqual(source, broken)
                    with self.assertRaises(ValueError):
                        validate_policy(broken)

    def test_identity_is_observed_from_real_local_git_not_hardcoded(self):
        for path in INSTALLERS:
            with self.subTest(installer=path.name), tempfile.TemporaryDirectory() as d:
                checkout = Path(d) / "nightfall"
                checkout.mkdir()
                subprocess.run(["git", "init", "-q", str(checkout)], check=True)
                lock = checkout / "Cargo.lock"
                lock.write_bytes(b"offline regression fixture; no real dependencies\n")
                subprocess.run(["git", "-C", str(checkout), "add", "Cargo.lock"], check=True)
                subprocess.run([
                    "git", "-C", str(checkout), "-c", "user.name=CI",
                    "-c", "user.email=ci@example.invalid",
                    "commit", "-qm", "local fixture"
                ], check=True)
                revision = subprocess.check_output(
                    ["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True
                ).strip()
                checksum = hashlib.sha256(lock.read_bytes()).hexdigest()
                source = path.read_text(encoding="utf-8")
                fragment = identity_fragment(source)
                runner = (
                    "set -euo pipefail\n"
                    + f'NIGHTFALL_RELEASE_SHA="{revision}"\n'
                    "info() { printf '%s\\n' \"$*\"; }\n"
                    "die() { printf '%s\\n' \"$*\" >&2; exit 17; }\n"
                    + fragment
                )
                env = dict(os.environ, BUILD=d)
                good = subprocess.run(
                    ["bash", "-c", runner], env=env, text=True, capture_output=True
                )
                self.assertEqual(good.returncode, 0, good.stderr)
                self.assertIn("source commit: " + revision, good.stdout)
                self.assertIn("Cargo.lock SHA256: " + checksum, good.stdout)
                wrong = runner.replace(
                    f'NIGHTFALL_RELEASE_SHA="{revision}"',
                    'NIGHTFALL_RELEASE_SHA="' + "0" * 40 + '"',
                    1,
                )
                rejected = subprocess.run(
                    ["bash", "-c", wrong],
                    env=env,
                    text=True,
                    capture_output=True,
                )
                self.assertEqual(
                    rejected.returncode, 17, rejected.stderr
                )
                self.assertIn(
                    "release tag does not match pinned commit",
                    rejected.stderr,
                )
                lock.unlink()
                bad = subprocess.run(
                    ["bash", "-c", runner], env=env, text=True, capture_output=True
                )
                self.assertEqual(bad.returncode, 17)
                self.assertIn("could not hash Cargo.lock", bad.stderr)



    def test_tag_clone_uses_released_commit_not_moving_main(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "origin"
            src.mkdir()

            subprocess.run(
                ["git", "init", "-q", str(src)],
                check=True,
            )

            lock = src / "Cargo.lock"
            lock.write_text("release\\n", encoding="utf-8")

            subprocess.run(
                ["git", "-C", str(src), "add", "Cargo.lock"],
                check=True,
            )

            commit = [
                "git", "-C", str(src),
                "-c", "user.name=Fixture",
                "-c", "user.email=fixture@example.invalid",
                "commit", "-qm",
            ]

            subprocess.run(commit + ["release"], check=True)

            tagged = subprocess.check_output(
                ["git", "-C", str(src), "rev-parse", "HEAD"],
                text=True,
            ).strip()

            subprocess.run(
                ["git", "-C", str(src), "tag", "v1.0.12"],
                check=True,
            )

            lock.write_text(
                "moving development\\n",
                encoding="utf-8",
            )

            subprocess.run(
                ["git", "-C", str(src), "add", "Cargo.lock"],
                check=True,
            )

            subprocess.run(
                commit + ["development"],
                check=True,
            )

            main = subprocess.check_output(
                ["git", "-C", str(src), "rev-parse", "HEAD"],
                text=True,
            ).strip()

            self.assertNotEqual(tagged, main)

            for installer in INSTALLERS:
                with self.subTest(installer=installer.name):
                    build = Path(d) / (
                        installer.name + "-build"
                    )
                    build.mkdir()

                    env = dict(
                        os.environ,
                        REPO=src.as_uri(),
                        BUILD=str(build),
                        NIGHTFALL_RELEASE_TAG="v1.0.12",
                    )

                    clone = (
                        'git clone --depth 1 --branch '
                        '"$NIGHTFALL_RELEASE_TAG" '
                        '"$REPO" "$BUILD/nightfall"'
                    )

                    self.assertIn(
                        clone,
                        installer.read_text(encoding="utf-8"),
                    )

                    subprocess.run(
                        ["bash", "-c", clone],
                        env=env,
                        check=True,
                        capture_output=True,
                    )

                    checkout = subprocess.check_output(
                        [
                            "git", "-C",
                            str(build / "nightfall"),
                            "rev-parse", "HEAD",
                        ],
                        text=True,
                    ).strip()

                    self.assertEqual(checkout, tagged)
                    self.assertNotEqual(checkout, main)

if __name__ == "__main__":
    unittest.main()
