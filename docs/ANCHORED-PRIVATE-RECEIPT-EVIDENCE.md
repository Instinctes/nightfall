# Anchored private receipt evidence (experimental, offline)

This CLI adds one **narrow, independently auditable step** between a signed
payment claim and a proof-carrying light client. It checks that a voluntarily
disclosed **v2 received/mined receipt** opens the amount, has a valid
spend-key signature, and names an output within a **particular supplied block**.

The tool is deliberately separate from the existing `verify-receipt`
command. It neither connects to a node nor reads/writes a wallet datadir.
It never requests a seed phrase or view key.

## The three distinct trust boundaries

1. **Wallet disclosure**: the holder chooses to reveal a single output's
   opening and a signature. An output opening is financially sensitive.
2. **Local block consistency**: validate the canonical aggregate, its body hash,
   the expected height, the presence of the output and the block-header hash
   against an externally supplied anchor.
3. **Independent chain validation**: the user must obtain **and validate** that
   anchor from a separate, independently trusted *fully validating* chain view.
   A copy of the hash from the same JSON file is **not independent evidence**.

This command implements **(1) + (2)**. It does **not** implement (3).
It does not verify PoW, difficulty adjustment, header ancestry, block validity
or canonical-chain selection. A caller cannot turn it into finality proof by
supplying an unverified hash. It does not prove that an output is unspent,
that the signer exclusively controls it, or that a 'mined' receipt is a
consensus-classified coinbase. A v1 receipt is refused in this stricter mode
because its `kind` was unsigned.

## Build and run

```sh
cargo test -p nightfall-wallet --bin nightfall-receipt-evidence
cargo build --locked --release -p nightfall-wallet --bin nightfall-receipt-evidence
./target/release/nightfall-receipt-evidence \
  --receipt receipt.json \
  --block block.json \
  --anchor 64_lowercase_or_uppercase_hex_characters
```

`receipt.json` is the exported v2 Nightfall receipt. `block.json` must be a
single serialized `nightfall_consensus::Block` object, not an RPC envelope,
not an array of blocks and not just a block header. The local `nightfalld`
`get_blocks` RPC returns an array in `result`; extract exactly the element
at the receipt's height and store only that element in `block.json`.
An archival node is needed for historical blocks whose bodies have been pruned.

For example, **on your own local machine with your own node**, this small
stdlib-only Python utility extracts one block without handling any keys:

```sh
python3 - <<'PY'
import json, socket
from pathlib import Path

receipt = json.loads(Path("receipt.json").read_text())
height = receipt["height"]
assert isinstance(height, int) and 0 <= height < 2**64
with socket.create_connection(("127.0.0.1", 17881), timeout=5) as sock:
    sock.settimeout(30)
    sock.sendall((json.dumps({
        "method": "get_blocks",
        "params": {"from": height, "limit": 1},
        "id": 1
    }) + "\n").encode())
    response = b""
    while not response.endswith(b"\n"):
        piece = sock.recv(65536)
        if not piece:
            raise SystemExit("incomplete local RPC response")
        response += piece
        if len(response) > 16_000_000:
            raise SystemExit("unexpectedly large RPC response")
parsed = json.loads(response)
if parsed.get("error"):
    raise SystemExit(str(parsed["error"]))
blocks = parsed.get("result")
if not isinstance(blocks, list) or len(blocks) != 1:
    raise SystemExit("one archive block not available")
if blocks[0]["header"]["height"] != height:
    raise SystemExit("wrong height")
Path("block.json").write_text(json.dumps(blocks[0]))
print("Saved block.json. Independently validate the anchor before running.")
PY
```

**Security**: do not obtain the `--anchor` value from the block file being
tested. A locally supplied hash can be an independent anchor **only when its
chain position has been independently established**. Two RPC endpoints from
the same operator do not establish decentralization. The full block also
contains public blockchain data; the voluntarily disclosed receipt connects
one of those outputs to a stated value, so share it only with an intended
verifier.

## Acceptance and adversarial checks

- Correct v2 signed amount opening and output membership at the exact hash.
- Fails closed on a missing/wrong anchor, tampered body, wrong height,
  duplicated commitment, altered amount, unsigned legacy receipt or sent claim.
- Bounded receipt/block input sizes; no network calls or wallet mutation.
- Existing consensus rules, protocol 8, emission, P2P and seed formats unchanged.

**Future direction**: compose this primitive with independently reviewed
header/PoW and authenticated-state proofs, rather than treating a remote
node's JSON response as independent evidence. This does not supersede existing
light-verifier RFCs or claim that those PRs are merged.
