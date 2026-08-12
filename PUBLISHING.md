# Publishing this repository

This folder is a complete, self-contained copy of the project, ready to become
a public GitHub repository. It builds and passes all 116 tests on its own.

Delete this file before or after the first push — it is a note to you, not
documentation for users.

---

## 1. Check what you are about to make public

Already verified when this folder was assembled, but check again — it takes ten
seconds and a leaked seed cannot be un-leaked:

```bash
cd github

# No key material, no chain data, no binaries
find . -name '*.seed' -o -name '*.outputs.json' -o -name 'blocks.jsonl'
find . -type f \( -name '*.dmg' -o -name '*.exe' \)

# Nothing that looks like a private key
grep -rIl -E 'BEGIN (RSA|OPENSSH|PRIVATE)' . 2>/dev/null
```

All three should print nothing.

Your wallet seeds live in `~/Library/Application Support/nightfall/` and were
never copied here. Keep it that way.

## 2. Create the repository

On GitHub, create an **empty** repo — no README, no licence, no `.gitignore`,
since this folder already has all three. The name in `Cargo.toml` and
throughout the docs is `instinctes/nightfall`; if you use a different
owner or name, search and replace it first:

```bash
grep -rl 'instinctes/nightfall' . | xargs sed -i '' 's|instinctes/nightfall|YOUR-ORG/YOUR-REPO|g'
```

## 3. Push

```bash
cd github
git init
git add -A
git commit -m "NIGHTFALLCOIN — protocol v5 (Nightproof-beta)

A privacy Layer-1 with a cryptographically provable supply. Confidential
amounts via Pedersen commitments and Bulletproofs, one-sided stealth outputs
so no address ever appears on chain, block-level aggregation, memory-hard
Argon2id proof of work, 90M hard cap, zero premine, 100% fee burn.

Includes the full security audit of the previous protocol version, which was
consensus-broken."

git branch -M main
git remote add origin git@github.com:instinctes/nightfall.git
git push -u origin main
```

## 4. Repository settings worth doing

- **Description:** `Money that refuses to snitch. A privacy Layer-1 with a supply anyone can prove.`
- **Topics:** `cryptocurrency` `privacy` `blockchain` `rust` `mimblewimble` `bulletproofs` `proof-of-work` `argon2`
- **Website:** your site URL
- Enable **Issues** and **Discussions**. Disable **Wiki** and **Projects** unless you will use them.
- Under Security → set up a **private vulnerability reporting** channel so `SECURITY.md` is not the only route.
- Branch protection on `main`: require the CI check to pass. The exploit
  regression suite is the reason CI exists — do not allow merges past a red build.

## 5. First release

```bash
./scripts/build-macos-dmgs.sh
cargo build --release --target x86_64-pc-windows-gnu \
    -p nightfall-core -p nightfall-node -p nightfall-wallet
```

Create a GitHub Release tagged `v0.3.0` and attach:

- `NIGHTFALLCOIN-Core-macOS-arm64.dmg`
- `NIGHTFALLCOIN-Core-macOS-intel.dmg`
- the three Windows `.exe` files

**Publish SHA-256 checksums in the release notes.** The binaries are unsigned,
so a checksum is the only way anyone can tell your build from someone else's:

```bash
shasum -a 256 wallets/*.dmg wallets/windows-x64/*.exe
```

State plainly in the release notes that the builds are unsigned and how to get
past Gatekeeper and SmartScreen. Users who are told to click through a security
warning without explanation learn to click through security warnings.

## 6. The website

`website/` is static and self-contained. Point GitHub Pages at it, or deploy the
folder anywhere. `website/downloads/` is gitignored — copy the release binaries
in at deploy time.

## 7. Once your seed node is running

Your always-on node is what stops new users from silently mining a private
fork. When it has a stable address:

1. Put it in `NetworkId::seed_nodes()` in `crates/nightfall-types/src/lib.rs`:

   ```rust
   Self::Mainnet => &["seed.your-domain.org:17891"],
   ```

2. Use a **DNS name**, not a bare IP. A home connection's address will change
   eventually, and a hardcoded IP turns into a dead seed for everyone running
   that build.

3. Ship a new release. Older builds keep working but cannot find the network
   on their own — mention that in the release notes.

4. Add the address to the website and to `docs/MAINNET.md`.

Forward TCP **17891** to the machine and confirm it is reachable from outside
your own network before publishing it.
