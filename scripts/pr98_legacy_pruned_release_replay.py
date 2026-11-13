#!/usr/bin/env python3
"""Release-authored legacy pruned fixture verification."""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = "https://github.com/Instinctes/nightfall.git"

TAGS = {
    "v1.1.2": "7d336540916b96ec7a6679d868106e628c0bee93",
    "v1.1.3": "b0c83258ac83972b40a5b5dbba1c987ae027db98",
}

CHAIN = (
    "blocks.bin",
    "blocks.jsonl",
    "chain-meta.json",
    "headers.jsonl",
    "utxo-horizon.json",
    "chain.json",
    "blocks.jsonl.pre-binary",
)

SOURCE = r'''
use nightfall_consensus::Chain;
use nightfall_crypto::WalletKeys;
use nightfall_storage::{now_unix, ChainStore};
use nightfall_types::NetworkId;
use std::fs;
use std::path::PathBuf;

#[test]
fn emit_legacy_pruned_fixture() {
    let dest = PathBuf::from(
        std::env::var("NIGHTFALL_FIXTURE_OUT")
            .expect("fixture path")
    );

    assert!(!dest.exists());
    fs::create_dir(&dest).unwrap();
    fs::create_dir(dest.join("wallet")).unwrap();

    fs::write(
        dest.join("wallet/owner-sentinel"),
        b"SYNTHETIC-WALLET-NO-SECRETS\n"
    ).unwrap();

    fs::write(
        dest.join("miner.seed"),
        b"SYNTHETIC-TEST-MARKER-NOT-A-KEY\n"
    ).unwrap();

    let miner = WalletKeys::generate().address();
    let mut chain = Chain::new_fair(
        NetworkId::Devnet
    ).unwrap();

    for i in 0..14u64 {
        chain.mine_block(
            &miner,
            vec![],
            now_unix() + i * 15
        ).unwrap();
    }

    assert_eq!(chain.block_count(), 14);
    assert_eq!(chain.prune_keep(4).unwrap(), 10);
    assert_eq!(chain.first_height, 10);

    let store = ChainStore::new(&dest);
    store.save(&chain).unwrap();

    assert!(store.blocks_path().is_file());
    assert!(store.headers_path().is_file());
    assert!(store.horizon_path().is_file());

    assert_eq!(
        store.load_or_new(NetworkId::Devnet)
            .unwrap().tip_hash(),
        chain.tip_hash()
    );

    let meta: serde_json::Value =
        serde_json::from_slice(
            &fs::read(store.meta_path()).unwrap()
        ).unwrap();

    assert_eq!(meta["pruned"], true);
    assert_eq!(meta["first_height"], 10);

    assert!(
        meta.get("validated_headers_fingerprint").is_none()
    );

    assert!(
        meta.get("validated_horizon_fingerprint").is_none()
    );

    println!("RELEASE_PRUNED_FIXTURE_GENERATED");
}
'''


def log(message):
    print(
        "[RELEASE-PRUNED] " + message,
        flush=True
    )


def require(test, message):
    if not test:
        raise RuntimeError(message)


def call(
    args,
    cwd=None,
    env=None,
    capture=False,
    expect=0,
):
    log("$ " + " ".join(map(str, args[:7])))

    p = subprocess.run(
        [str(a) for a in args],
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )

    success = p.returncode == 0
    expected_success = expect == 0

    if expect is not None and success != expected_success:
        if capture:
            print(
                "STDOUT:",
                (p.stdout or "")[-4000:],
                file=sys.stderr
            )
            print(
                "STDERR:",
                (p.stderr or "")[-4000:],
                file=sys.stderr
            )

        raise RuntimeError(
            f"Exit {p.returncode}: {args[:4]}"
        )

    return p


def sha256(path):
    h = hashlib.sha256()

    with open(path, "rb") as f:
        for block in iter(
            lambda: f.read(1024 * 1024),
            b""
        ):
            h.update(block)

    return h.hexdigest()


def fingerprint(directory, require_pruned=True):
    result = {}

    for name in CHAIN:
        path = directory / name

        if path.exists():
            require(
                path.is_file()
                and not path.is_symlink(),
                "Unsafe fixture file: " + str(path)
            )

            result[name] = {
                "sha256": sha256(path),
                "size": path.stat().st_size,
            }

    protected = (
        "wallet/owner-sentinel",
        "miner.seed",
        "install-id",
    )

    for name in protected:
        path = directory / name

        require(
            path.is_file()
            and not path.is_symlink(),
            "Protected fixture marker missing: " + name
        )

        result[name] = {
            "sha256": sha256(path),
            "size": path.stat().st_size,
        }

    require(
        "chain-meta.json" in result,
        "Chain metadata missing"
    )

    if require_pruned:
        require(
            "utxo-horizon.json" in result
            and "headers.jsonl" in result,
            "Fixture is not pruned"
        )

    require(
        "blocks.bin" in result
        or "blocks.jsonl" in result,
        "Block file missing"
    )

    return result


def assert_refusal_reason(result, words, label):
    message = (
        (result.stdout or "")
        + "\n"
        + (result.stderr or "")
    ).lower()

    require(
        any(word in message for word in words),
        label
        + ": unexpected refusal: "
        + message[-700:]
    )


def clone_release(tag, sha, directory):
    # Separate clone prevents shallow-fetch state conflicts.
    call([
        "git",
        "clone",
        "--quiet",
        "--depth", "1",
        "--branch", tag,
        "--single-branch",
        REPO,
        directory,
    ])

    actual = call(
        ["git", "rev-parse", "HEAD"],
        cwd=directory,
        capture=True,
    ).stdout.strip()

    require(
        actual == sha,
        f"{tag}: release commit mismatch"
    )

    # Original storage and consensus source remain unchanged.
    # Only an ephemeral integration test is added.
    test = (
        directory
        / "crates/nightfall-storage/tests/pr98_release_fixture.rs"
    )

    require(
        not test.exists(),
        "Test file collision"
    )

    test.parent.mkdir(
        parents=True,
        exist_ok=True
    )

    test.write_text(SOURCE)


def check_release(
    tag,
    sha,
    home,
    kit_binary,
    node_binary,
    env,
):
    log("Verifying " + tag)

    source = home / ("source-" + tag)
    fixture = home / ("fixture-" + tag)

    clone_release(
        tag,
        sha,
        source
    )

    local_env = env.copy()

    local_env["NIGHTFALL_FIXTURE_OUT"] = str(
        fixture
    )

    call([
        "cargo", "test", "--locked",
        "-p", "nightfall-storage",
        "--test", "pr98_release_fixture",
        "--", "--test-threads=1",
    ], cwd=source, env=local_env)

    original = fingerprint(fixture)

    meta = json.loads(
        (fixture / "chain-meta.json").read_text()
    )

    require(
        meta.get("pruned") is True
        and meta.get("first_height") == 10,
        f"{tag}: invalid pruned metadata"
    )

    require(
        not meta.get("validated_horizon_fingerprint")
        and not meta.get("validated_headers_fingerprint"),
        f"{tag}: unexpected semantic seals"
    )

    # Check 1: --force omission must refuse.
    refused = home / ("refusal-" + tag)

    shutil.copytree(
        fixture,
        refused,
        symlinks=True
    )

    before = fingerprint(refused)

    denied = call([
        kit_binary,
        "--network", "devnet",
        "--json",
        "upgrade-pruned",
        "--datadir", refused,
    ], capture=True, expect=1)

    assert_refusal_reason(
        denied,
        ("--force", "legacy pruned"),
        f"{tag}: without force"
    )

    require(
        fingerprint(refused) == before,
        f"{tag}: refusal modified files"
    )

    # Check 2: Current node must refuse unsealed legacy state.
    denied = call([
        node_binary,
        "--network", "devnet",
        "--datadir", refused,
        "status",
    ], capture=True, expect=1)

    assert_refusal_reason(
        denied,
        ("pruned", "validation record", "seal"),
        f"{tag}: legacy startup"
    )

    require(
        fingerprint(refused) == before,
        f"{tag}: refused startup changed chain"
    )

    # Check 3: Wrong-network --force must refuse.
    wrong = home / ("wrong-network-" + tag)

    shutil.copytree(
        fixture,
        wrong,
        symlinks=True
    )

    wrong_before = fingerprint(wrong)

    denied = call([
        kit_binary,
        "--network", "mainnet",
        "--json",
        "upgrade-pruned",
        "--datadir", wrong,
        "--force",
    ], capture=True, expect=1)

    assert_refusal_reason(
        denied,
        ("network", "genesis"),
        f"{tag}: wrong network"
    )

    require(
        fingerprint(wrong) == wrong_before,
        f"{tag}: wrong network changed chain"
    )

    # Check 4: Interrupted recovery must refuse startup.
    interrupted = home / ("journal-" + tag)

    shutil.copytree(
        fixture,
        interrupted,
        symlinks=True
    )

    (
        interrupted / ".recovery-journal.json"
    ).write_text(
        "synthetic interruption marker\n"
    )

    interrupted_before = fingerprint(
        interrupted
    )

    denied = call([
        node_binary,
        "--network", "devnet",
        "--datadir", interrupted,
        "status",
    ], capture=True, expect=1)

    assert_refusal_reason(
        denied,
        ("interrupted", "recovery"),
        f"{tag}: interrupted recovery"
    )

    require(
        fingerprint(interrupted) == interrupted_before,
        f"{tag}: interrupted startup changed chain"
    )

    # Check 5: Explicit backup/resync on disposable copy.
    upgrade = home / ("upgrade-" + tag)

    shutil.copytree(
        fixture,
        upgrade,
        symlinks=True
    )

    upgrade_before = fingerprint(upgrade)

    response = call([
        kit_binary,
        "--network", "devnet",
        "--json",
        "upgrade-pruned",
        "--datadir", upgrade,
        "--force",
    ], capture=True)

    report = json.loads(response.stdout)

    require(
        report.get("resync_required") is True,
        f"{tag}: missing resync requirement"
    )

    require(
        report.get("wallet_preserved") is True,
        f"{tag}: wallet preservation not reported"
    )

    backup = Path(
        report["backup"]
    ).resolve()

    require(
        backup.is_relative_to(upgrade.resolve())
        and backup.is_dir(),
        f"{tag}: backup outside test directory"
    )

    require(
        not (
            upgrade / ".recovery-journal.json"
        ).exists(),
        f"{tag}: recovery journal remains"
    )

    # Check 6: Every original chain file must be backed up.
    for name in CHAIN:
        if name in upgrade_before:
            path = backup / name

            require(
                path.is_file()
                and sha256(path)
                == upgrade_before[name]["sha256"],
                f"{tag}: backup differs for {name}"
            )

    after = fingerprint(
        upgrade,
        require_pruned=False
    )

    for name in (
        "wallet/owner-sentinel",
        "miner.seed",
        "install-id",
    ):
        require(
            after[name] == upgrade_before[name],
            f"{tag}: identity changed: {name}"
        )

    # Check 7: The new binary must reload fresh genesis.
    status = call([
        node_binary,
        "--network", "devnet",
        "--datadir", upgrade,
        "status",
    ], capture=True)

    require(
        "blocks......... 0" in status.stdout,
        f"{tag}: fresh genesis not loaded"
    )

    fresh_meta = json.loads(
        (upgrade / "chain-meta.json").read_text()
    )

    require(
        fresh_meta.get("block_count") == 0,
        f"{tag}: reset did not reach genesis"
    )

    log(tag + " PASS")

    return {
        "release": tag,
        "source_commit": sha,
        "synthetic_devnet_only": True,
        "old_height": meta["block_count"] - 1,
        "old_first_height": meta["first_height"],
        "release_authored_chain_files": sorted(
            n for n in original if n in CHAIN
        ),
        "no_force_refusal_unchanged": True,
        "unsealed_startup_refused": True,
        "wrong_network_refused": True,
        "interrupted_recovery_refused": True,
        "backup_byte_identical": True,
        "wallet_and_miner_preserved": True,
        "fresh_genesis_reloaded": True,
        "resync_required": True,
        "old_meta_sha256": original[
            "chain-meta.json"
        ]["sha256"],
    }


def main():
    parser = argparse.ArgumentParser()

    parser.add_argument(
        "--workspace",
        type=Path,
        default=Path.cwd()
    )

    parser.add_argument(
        "--report",
        type=Path
    )

    args = parser.parse_args()

    workspace = args.workspace.resolve()

    require(
        (
            workspace
            / "crates/nightfall-nodekit/Cargo.toml"
        ).is_file(),
        "Not a Nightfall workspace"
    )

    env = os.environ.copy()

    env["CARGO_BUILD_JOBS"] = "2"
    env["CARGO_INCREMENTAL"] = "0"
    env["CARGO_TARGET_DIR"] = str(
        workspace / "target"
    )

    log("Compiling current PR98 Node Kit")

    call([
        "cargo", "build", "--locked",
        "-p", "nightfall-nodekit",
        "--bin", "nightfall-nodekit",
    ], cwd=workspace, env=env)

    call([
        "cargo", "build", "--locked",
        "-p", "nightfall-node",
        "--bin", "nightfalld",
    ], cwd=workspace, env=env)

    kit = (
        workspace
        / "target/debug/nightfall-nodekit"
    )

    node = (
        workspace
        / "target/debug/nightfalld"
    )

    require(
        kit.is_file() and node.is_file(),
        "Current binaries missing"
    )

    # Test existing rejection and fault-injection paths.
    call([
        "cargo", "test", "--locked",
        "-p", "nightfall-nodekit",
        "--test", "upgrade_pruned_cli",
    ], cwd=workspace, env=env)

    call([
        "cargo", "test", "--locked",
        "-p", "nightfall-node",
        "--lib", "recovery::tests",
        "--", "--test-threads=1",
    ], cwd=workspace, env=env)

    with tempfile.TemporaryDirectory(
        prefix="nightfall-pr98-legacy-fixtures-"
    ) as temp:
        directory = Path(temp)

        results = [
            check_release(
                tag,
                sha,
                directory,
                kit,
                node,
                env,
            )
            for tag, sha in TAGS.items()
        ]

    report = {
        "format": "nightfall-pr98-release-pruned-compat-v1",
        "current_reference": "v1.1.3",
        "all_assertions_passed": True,
        "synthetic_release_authored_fixtures": True,
        "real_operator_fixture_proven": False,
        "mainnet_scale_proven": False,
        "live_mainnet_data_modified": False,
        "releases": results,
    }

    if args.report:
        destination = args.report.resolve()

        require(
            not destination.exists(),
            "Report already exists; refusing overwrite"
        )

        destination.parent.mkdir(
            parents=True,
            exist_ok=True
        )

        destination.write_text(
            json.dumps(
                report,
                indent=2,
                sort_keys=True
            ) + "\n"
        )

    print(
        json.dumps(
            report,
            indent=2,
            sort_keys=True
        ),
        flush=True
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(
            "[RELEASE-PRUNED] FAIL-CLOSED: " + str(exc),
            file=sys.stderr,
            flush=True
        )
        sys.exit(1)
