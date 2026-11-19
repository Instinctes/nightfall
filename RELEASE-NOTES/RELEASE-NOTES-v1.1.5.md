# NIGHTFALLCOIN 1.1.5 — GPU mining in Core, a wider network

1.1.5 brings GPU mining into the official wallet and helps the network grow
beyond its seeds. Consensus rules, supply, emission, block reward, block time
and proof of work are unchanged, and nodes on 1.1.1 to 1.1.4 stay fully
compatible.

## GPU mining, no fee

- **Mine with your graphics card.** Core can now mine with the GPU: Metal on
  Apple computers, CUDA on NVIDIA cards under Windows and Linux. It is off
  until you switch it on in Mining, and there is no fee: the whole block
  reward goes to your wallet, as with CPU mining.
- **Checked before it is trusted.** When GPU mining starts, the GPU must
  produce exactly the same hash as the CPU in a self-test. Every block the GPU
  finds is computed again on the CPU before it is sent. On any mismatch or
  driver error the GPU switches itself off and the CPU carries on.
- **Nothing extra to install.** The NVIDIA driver is loaded when GPU mining
  starts; no CUDA toolkit is needed. A computer without a supported GPU simply
  shows no GPU option. While on, GPU mining uses about 4 GiB of graphics
  memory.
- Tested on Apple Silicon (M3 Pro). The CUDA path has been checked against
  the consensus hash with a host emulator, but not yet on many NVIDIA cards;
  the self-test and the CPU recheck are there for exactly that reason.

## A wider network

- **Nodes pass on nodes that really answer.** When a node connects to yours,
  your node now dials it back once to see whether it accepts connections. If
  it does, its address goes into the list your node hands to newcomers.
  Addresses not confirmed again within 24 hours drop out of that list, so new
  nodes are no longer sent to machines that went away.
- **Optional UPnP.** A new setting (Settings → Node → Reachability, and
  `nightfalld --upnp`) asks your router to forward the P2P port, so other
  nodes can connect to you. It is off by default and is never used when a
  proxy such as Tor is set.
- **An honest version count.** "What the network is running" now counts
  peers connected now or in the last 30 minutes, instead of every node met
  since the seed started.

## Also

- Checkpoint at 540,000, computed by a node that validated the blocks itself
  and cross-checked against the seed that serves the website's chain view.
- The security advisory for the issues fixed at block 500,000 is published:
  GHSA-6v83-w826-8fw2.

## Upgrading

Close Core completely before replacing the app or binary. Keep your 24 words
and an encrypted wallet backup, and never delete your data folder to update.
Nothing has to be migrated.

## Build and verification

Official macOS arm64/Intel, Windows x64 and Linux x64 release artifacts are
built by GitHub Actions with `--locked`, gated by platform tests, and published
with SHA256 manifests and GitHub build provenance. Verify with
`scripts/check_release_provenance.py` or `gh attestation verify <file> --repo
Instinctes/nightfall`. Provenance is not Apple notarisation, Windows
Authenticode signing or an independent security audit.
