# Nightfall Node Kit

Run your own node without asking anyone for help. Status: shipped in 1.1.0.

The kit is one program, `nightfall-nodekit`. It reads, or it writes one new
file you named. It never needs root, never touches a running node's data, never
installs anything, and talks to nothing except the node address you give it.

```
nightfall-nodekit genesis                      # what chain this build expects
nightfall-nodekit doctor                       # is my data directory healthy?
nightfall-nodekit status                       # is my running node keeping up?
nightfall-nodekit verify --sums SHA256SUMS…    # are my downloads intact?
nightfall-nodekit service --kind systemd …     # write a unit file to review
```

Every command takes `--network mainnet|testnet|devnet` (default mainnet) and
`--json` for scripts. Exit code 0 is healthy, 1 means a check failed, 2 means
the command itself could not run.

## Where the expected chain comes from

`genesis` prints a hash computed by the same consensus code the node runs.
Nothing in the kit contains a hash typed in by hand, so a kit built from a
given source tree can never expect a chain that tree would not produce. For
mainnet it is:

```
061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de
```

If your kit prints anything else for mainnet, it was not built from this
project's source. Stop.

## 1. Install

**Download and verify before running anything.** From the release page, fetch
the binaries and the matching `SHA256SUMS-<version>.txt` (or the `-linux` /
`-windows` list), put them in one directory, then:

```
nightfall-nodekit verify --sums SHA256SUMS-1.0.5-linux.txt
```

Every line must say `OK`. `MISMATCH` means do not run that file — the download
is corrupt or has been replaced. `not here` only means you did not download
that one. A checksum proves the file is the one listed, not that the software
is safe; the list itself should come from the release page over HTTPS.

**Pick a data directory and an account.** On Linux, a dedicated unprivileged
user (`nightfall`) owning `/var/lib/nightfall` is the convention. On macOS the
default is `~/Library/Application Support/nightfall/mainnet/n8`.

**Write a service file, read it, then install it yourself.**

```
nightfall-nodekit service --kind systemd \
  --binary /usr/local/bin/nightfalld \
  --datadir /var/lib/nightfall \
  --user nightfall --public \
  --out ./nightfall-mainnet.service
```

The kit refuses to write into `/etc`, `/usr`, `/Library` and similar, refuses
to overwrite an existing file, refuses relative paths, refuses to run as
`root`, and refuses a path with a space for systemd (systemd would split it).
The unit keeps RPC on `127.0.0.1` whatever you ask for — it is an administrative
port and the kit will not publish it. `--public` only opens the peer-to-peer
port. The first lines of the file tell you the two commands that install it.

For macOS use `--kind launchd`; it writes a LaunchAgent plist and the install
lines are at its top.

## 2. Check it

```
nightfall-nodekit doctor --datadir /var/lib/nightfall
```

Reads the directory and reports, line by line: whether the chain belongs to the
network you named, whether its genesis matches this build, whether
`blocks.bin` matches its own validation record (shorter means truncated or
replaced; longer is normal while the node is appending), whether a node holds
the directory lock, whether any seed file is readable by others, and how many
peers are remembered.

It opens files read-only and takes no lock, so it is safe beside a running
node. It never deletes a lock, never repairs a file, never changes a
permission — it tells you the command to run instead.

```
nightfall-nodekit status
```

Connects to the node's RPC port (default `127.0.0.1:17881` on mainnet) and
refuses to describe a node that is on a different chain. Reports height, the
best peer's height, how far behind you are, and a warning if you have no peers
— mining with no peers is how a private fork starts.

## 3. Update

1. Download the new version and **verify** it as in step 1.
2. Stop the node: `sudo systemctl stop nightfall-mainnet` (or
   `launchctl unload …`).
3. Keep the old binary: `sudo mv /usr/local/bin/nightfalld /usr/local/bin/nightfalld.previous`.
4. Install the new binary in its place.
5. Start the node, then run `doctor` and `status`. A node that has to
   re-verify its chain after an update reports `loading` for a while; that is
   expected, not a failure.

The chain data directory is not part of an update. Do not delete it to
"start clean" — that throws away a verified chain and costs a full re-sync.

## 4. Roll back

If the new version misbehaves:

1. Stop the node.
2. `sudo mv /usr/local/bin/nightfalld.previous /usr/local/bin/nightfalld`
3. Start it, run `doctor`.

Rolling back a binary is safe within the same protocol version (the chain file
format is `n8` throughout the 1.x line). Release notes say explicitly when a
version changes something a rollback cannot undo; none in 1.x does.

## 5. Recover

Run `doctor` first. Its finding tells you which case you are in.

**`genesis … different chain`** — the directory holds a chain from another
network or an abandoned genesis. Point the node at the right directory. Do not
edit `chain-meta.json` to make the error go away; the file is a record, and
changing it disables verification.

**`blocks.bin … shorter than the validated record`** — the chain file was cut
short (disk full, crash during copy). Stop the node, move the directory aside
rather than deleting it (`mv /var/lib/nightfall /var/lib/nightfall.damaged`),
start the node on a fresh directory and let it sync. Keep the damaged copy
until the new node is healthy.

**`chain-meta.json is not valid JSON`** — same procedure: move aside, re-sync.

**`leftover record from pid …, which is no longer running`** — normal after a
crash or a hard power-off. The node's lock is an operating-system lock that the
kernel releases when the process dies; the file only remembers who held it
last. Start the node normally and it takes the lock over. There is nothing to
delete. `doctor` tells a live owner from a dead one by asking whether that pid
exists, without taking the lock itself.

**`pid … is running and owns this directory`** — a node is using it. Do not
start a second one on the same directory; the second start will refuse, and
that refusal is what keeps two writers off one chain file.

**No peers** — check that the P2P port (17891 on mainnet) is reachable if you
used `--public`, and that outbound connections are allowed. A node with no
peers still runs; it just is not part of the network.

## What the kit does not do

It does not install services, open firewall ports, update itself, delete
files, change permissions, read seed contents, contact seed nodes, or report
anything anywhere. Every one of those is something an operator should see
happen and do on purpose.

## Status against the roadmap

Implemented and tested locally: diagnostics, network/genesis pinning derived
from consensus, download verification with path-traversal refusal, service
generation for systemd and launchd with loopback-only RPC, and this runbook.
20 unit tests, run in private temporary directories. `doctor` was also run
read-only against a real mainnet directory and left it unchanged — and that
run caught a bug the unit tests had not: the first version reported every
leftover lock file as "held", when the node's lock is a kernel lock that dies
with its process. It now checks whether the recorded pid is alive.

**Not yet evidenced:** the roadmap's acceptance criterion is that *independent
operators* complete installation and a recovery exercise using this guide.
That requires people outside the project and has not happened. Until it does,
the Node Kit is built, not proven.
