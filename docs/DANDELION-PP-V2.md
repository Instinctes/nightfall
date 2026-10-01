# Dandelion++ relay design

Status: development foundation only. Not consensus-active.

## Problem

The current relay is Dandelion-class rather than full Dandelion++:

- stem and fluff use the ordinary transaction relay path;
- stem routing is chosen per relay rather than from an epoch-stable privacy graph;
- stem transactions share the ordinary mempool;
- there is no explicit stem/fluff wire state.

Independent per-transaction routing creates a network fingerprint surface.
A stem transaction also should not become mineable merely because it is still
inside its anonymity phase.

## Required invariants

1. Choose at most two live outbound Dandelion destinations per epoch.
2. Keep one stable route for locally originated transactions for that epoch.
3. Map each inbound edge to one stable outbound destination.
4. Balance new inbound mappings across the least-loaded destinations.
5. Never deliberately route a stem back to its source.
6. Keep stem transactions outside the ordinary mining mempool.
7. Seeing a fluff copy cancels the corresponding stem embargo.
8. Expired embargoes fail safe by promoting and fluffing the transaction.
9. Peer churn repairs dead routes without changing consensus state.
10. Epoch changes clear routing metadata, not wallet or ledger state.

## Next integration boundary

After this routing core passes independently:

- add backward-compatible stem/fluff semantics to the P2P transaction message;
- instantiate `StemPool<Transaction>` in the node runtime;
- move locally originated and incoming stem transactions out of `Mempool`;
- promote only on fluff or embargo expiry;
- replace per-transaction `pick_stem_peer` routing with `DandelionRouter`;
- remove the deterministic first-peer privacy fallback;
- add real multi-process privacy-graph, loop, black-hole and peer-churn tests.

This subsystem must not alter block validity, emission, supply invariants,
genesis, proof of work or transaction authorization.


## Stage 2A — explicit wire phase

Implemented after the routing-core prototype:

- `PeerMsg::Tx` now carries an optional `stem` phase bit.
- ordinary/fluff transactions omit the bit and retain the legacy wire shape;
- a new reader defaults a legacy message to `stem = false`;
- an old serde reader ignores `stem = true`;
- the session layer can send to one exact route selected by the epoch router;
- the old random stem sender also emits an actual stem message.

This stage deliberately does not yet change `NodeInner`, the public mempool or
mining. Runtime integration remains a separate review boundary so wire
compatibility can fail independently of transaction-state handling.


## Stage 2B — runtime stempool separation

Runtime integration now enforces the privacy boundary:

- locally originated transactions enter the stempool before relay;
- a stem transaction is not present in the mining mempool;
- relay routing uses the epoch-scoped `DandelionRouter`;
- own transactions always stem, regardless of relay/fluff epoch mode;
- inbound stem edges keep their epoch-stable outbound mapping;
- ordinary diffusion cancels the matching stem entry;
- receiving the same stem twice acts as a loop circuit breaker and fluffs;
- an expired embargo revalidates the transaction before promotion;
- block acceptance removes matching entries from both mempool and stempool;
- the stempool is hard-capped at 10,000 entries;
- no-live-session startup fallback selects one peer randomly rather than
  deterministically taking the first address;
- lack of a valid stem route fails open to diffusion rather than silently
  black-holing a payment.

Mining remains unchanged: block construction reads only the ordinary mempool.
Therefore a transaction cannot become mineable merely by being observed during
its Dandelion anonymity phase.


## Stage 2C — negotiated stem capability

Stem semantics are now explicitly negotiated during the existing handshake.

Compatibility rules:

- new nodes advertise `dandelion_stem_v1 = true`;
- the field uses `serde(default)`, so an old peer is read as `false`;
- ordinary transaction relay remains fully compatible;
- only explicitly capable sessions are eligible Dandelion stem routes;
- `stem_tx_to` refuses a legacy session even if called incorrectly;
- an unnegotiated incoming `stem = true` is treated as ordinary diffusion;
- direct startup fallback sends a stem only after the remote handshake
  explicitly advertises support;
- a legacy fallback peer receives no stem; the local embargo later promotes
  and diffuses the transaction normally.

This prevents a mixed-version network from silently converting a supposedly
anonymous stem hop into an immediately public/mineable legacy transaction.


## Stage 2D — adversarial invariant harness

Stage 2D intentionally adds no new relay behavior. It hardens the evidence for
the existing privacy boundary.

The harness verifies:

- a transaction held only in `StemPool` is invisible to the real consensus
  `Mempool::select_for_block()` path;
- embargo promotion changes that visibility exactly once;
- seeing fluff cancels the pending embargo before public promotion;
- a duplicate stem acts as a loop signal while mempool deduplication leaves
  only one public/mineable copy;
- a session without negotiated `dandelion_stem_v1` cannot send or be selected
  as a stem route;
- legacy `HelloOk` messages with no capability field decode as
  `dandelion_stem_v1 = false`;
- new `HelloOk` messages preserve an explicit stem capability.

This stage is deliberately test-only. Socket delivery acknowledgement, peer
failure repair after an asynchronous write, and multi-process topology tests
remain separate hardening work rather than being hidden inside the invariant
harness.


## Stage 2E — failure-aware asynchronous stem relay

Stem delivery remains asynchronous so a slow or dead peer cannot hold the
global node-state lock.

The send path now distinguishes two events:

1. the stem write was successfully scheduled;
2. the socket write actually succeeded.

Actual write, handshake, or direct-dial failures are fed back through a small
in-memory failure queue. If the transaction is still on exactly that failed
route, the router receives one opportunity to choose another negotiated
Dandelion peer while excluding both the failed edge and the upstream source.

Each transaction receives at most one immediate route repair. A second failure,
or lack of a safe alternate edge, leaves the original randomized embargo
armed. This avoids fast retry loops and timing fingerprints while preserving
the existing availability fail-safe.


## Stage 2F1 — bounded failure metadata

The asynchronous stem-delivery failure queue is now both bounded and
deduplicated.

Properties:

- at most 10,000 pending failure events are retained;
- identical `(txid, route)` failures occupy one slot;
- overflow evicts the oldest failure metadata;
- eviction does not lose the transaction because the stempool's randomized
  embargo remains the final availability mechanism;
- draining the queue also clears its deduplication index;
- an accidental duplicate `remove_stem_included()` call in block acceptance
  was removed.

This closes the memory-growth surface introduced by failure-aware relay without
turning socket writes synchronous or weakening the embargo fallback.


## Stage 2F2B — logical peer identity and anti-echo

Socket identity and privacy identity are now separate concepts.

Nightfall intentionally permits an inbound and an outbound TCP session to the
same node at the same time. Their session keys must stay different so one
socket cannot overwrite the other.

Dandelion routing, however, must treat those sockets as one logical peer.

For direct TCP peers the logical identity is derived from the observed remote
IP plus the listen port already advertised in the handshake. For SOCKS/Tor
outbound connections the dial target is retained because `peer_addr()` names
the proxy rather than the remote node.

Consequences:

- a stem received over `in:IP:ephemeral-port` cannot be sent back over
  `out:IP:listen-port` to the same logical peer;
- failure repair preserves the same exclusion;
- ordinary fluff also avoids echoing through a sibling session to the same
  logical peer;
- session map keys remain unchanged, preserving the existing anti-collision
  behavior.

This is local transport identity, not a new globally linkable cryptographic
node identifier.

## Stage 2F3 — real three-node TCP topology

A deliberately slow integration test now starts three complete Devnet
`NodeHandle` instances with real loopback TCP sockets.

The topology contains the privacy edge case that unit-only routing tests cannot
fully reproduce:

    A outbound -> B
    B outbound -> A
    B outbound -> C

B therefore owns both an inbound source session from A and a separate outbound
session back to the same logical A peer.

The harness:

- negotiates Dandelion capability over the real handshake;
- creates a valid mature Devnet spend against identical consensus state;
- submits through the real `NodeInner::submit_tx` path;
- proves A keeps the transaction outside its mining mempool;
- proves B receives it through the real stem transaction transport;
- when B's epoch is in stem mode, proves its selected route belongs to C rather
  than A's sibling socket;
- when B's epoch is in fluff mode, exercises the same logical-peer exclusion
  through normal diffusion;
- proves C receives the transaction;
- proves C's diffusion does not return to A before A's minimum embargo.

Consensus setup is injected identically into the three in-memory Devnet chains
so the test measures Dandelion transport rather than initial-block-download
performance. Block construction itself still uses the real consensus
`Chain::mine_block` path.

The test is marked ignored because it opens multiple live nodes and performs
real Devnet mining. It is run explicitly as a networking/deploy gate.


## Stage 2F4 — explicit real-TCP STEM and FLUFF coverage

The three-node transport harness now exercises both production Dandelion relay
states explicitly.

No production control flag, environment variable, protocol field, RPC method or
alternate routing implementation was added. The integration harness advances
the existing router through synthetic epoch boundaries until the requested
production mode is selected.

Two ignored real-network integration tests now exist:

- real three-node TCP STEM;
- real three-node TCP FLUFF.

Both use the same duplicate-peer topology:

    A outbound -> B
    B outbound -> A
    B outbound -> C

The STEM case proves that B retains the transaction outside its mining mempool
and forwards the stem to logical C rather than through A's sibling socket.

The FLUFF case proves that B intentionally enters public diffusion while still
excluding every socket belonging to logical A.

Both cases use complete nodes, real loopback TCP, real handshake capability
negotiation, the production transaction codec and a valid mature Devnet spend.

Stage 2F4 changes only integration-test and documentation code.


## Stage 2G1 — logical-peer destination diversity

The Dandelion anonymity graph now receives at most one outbound transport
route for each logical peer identity.

Nightfall intentionally permits multiple TCP sessions to one remote node.
Those sessions remain independent in the session map and continue to use
different `in:...` / `out:...` transport keys.

They no longer count as independent anonymity destinations.

`SessionPool` now collapses Dandelion-capable outbound sessions by `peer_id`
before exposing candidate routes to the epoch router. If multiple live sockets
belong to the same logical peer, the lexicographically smallest route key is
used as a deterministic representative. If that socket disappears, another
live sibling can become the representative.

Consequences:

- two aliases or sockets to one node cannot occupy both Dandelion destination
  slots;
- locally originated stems inherit logical-peer diversity automatically;
- incoming stems inherit the same property after source-peer exclusion;
- failure repair receives the same peer-unique candidate set;
- legacy peers remain excluded;
- socket/session ownership semantics are unchanged.

The router remains transport-route based internally, but its input set now
enforces the stronger invariant that every candidate route represents a
different logical peer.


## Stage 2G2 — real-TCP alias identity deduplication

The logical-peer destination invariant is now also verified through the real
network stack rather than only through synthetic `SessionPool` construction.

A Devnet integration test opens two independent outbound TCP connections from
one node to the same remote Nightfall node using different dial strings:

    127.0.0.1:PORT
    localhost:PORT

The transport sessions intentionally retain distinct session keys.

After the real TCP connection and Nightfall handshake, both sessions must
derive the same logical `peer_id` from the observed remote endpoint and the
peer-advertised listening port.

A third distinct Nightfall node is connected simultaneously.

The harness proves that three Dandelion-capable outbound sockets produce only
two Dandelion candidates:

- one representative for logical peer A;
- one route for logical peer C.

It also proves that excluding logical A removes both of A's real transport
aliases at once.

No production code is changed in Stage 2G2.


## Stage 2G4 — SOCKS/Onion logical-identity isolation

The outbound SOCKS identity boundary is now exercised through a real
SOCKS5 forwarding transport.

Two complete Devnet Nightfall nodes are exposed behind one local test
SOCKS proxy. A third Nightfall node connects through that same proxy
using two different synthetic `.onion` dial targets.

The proxy forwards each hostname to its corresponding local Devnet
listener, so the normal Nightfall outbound connection, handshake,
capability negotiation and session-registration paths execute.

The integration harness verifies that:

- Onion targets reach SOCKS as hostnames rather than local DNS lookups;
- two connections may share one physical proxy endpoint;
- the SOCKS endpoint does not become the Dandelion logical identity;
- each outbound Onion dial target remains a distinct `peer_id`;
- the two Onion peers remain distinct Dandelion candidates;
- excluding one Onion logical peer leaves the other eligible.

No peer-advertised Onion identity, cryptographic node identity, RPC
override or production test hook is introduced.

Inbound hidden-service identity remains intentionally unresolved because
the current Nightfall wire protocol has no authenticated node identity
from which such an identity can safely be derived.

Stage 2G4 changes only integration-test and documentation code.


## Stage 2H1 — local-origin privacy under stempool exhaustion

The global Dandelion stempool remains hard-capped at 10,000 entries,
but remote traffic can no longer consume every privacy slot and thereby
force an unrelated locally created transaction directly into public
diffusion.

Local-origin admission now follows these rules:

- duplicate detection still happens before any eviction;
- below the hard cap, admission is unchanged;
- a remote stem never evicts any existing entry;
- when the pool is exactly full, a local origin may replace one remote
  stem whose `source` is present;
- local stems never evict other local stems;
- if every slot is local-origin, the new local stem still receives
  `StemInsert::Full`;
- the hard cap therefore remains exactly 10,000 entries.

When local admission needs a remote victim, the entry with the earliest
local embargo deadline is selected. Transaction id is used as a stable
tie-breaker.

The displaced remote transaction is not promoted or fluffed by this
node as part of the admission decision. Its previous Dandelion hop
retains its own independent embargo failsafe. This avoids converting
resource pressure into an immediate public-diffusion event.

Both local submission paths — established-session stem routing and the
startup dial fallback — use the same local-preemption policy.

This stage does not add a per-peer remote quota. Such quotas remain a
separate denial-of-service hardening question; Stage 2H1 specifically
removes remote stempool exhaustion as a cause of local privacy downgrade.


## Stage 2H2 — bounded remote admission under exhaustion

The global Dandelion stempool remains hard-capped at 10,000 entries.

Remote stems may occupy at most 75% of that pool. The remaining capacity is
reserved against remote exhaustion but is not a separate local pool: locally
originated transactions may use the complete global capacity.

The remote ceiling is deliberately global rather than keyed to `peer_id`.
Nightfall does not currently cryptographically authenticate a Sybil-resistant
node identity, so a per-peer quota would be bypassable by identity rotation
and could also penalize an honest relay that happens to carry a large share of
the stem traffic.

Remote occupancy is accounted incrementally in O(1). Explicit removal, block
cleanup, fluff cancellation, embargo expiry and Stage 2H1 local preemption all
converge through the same accounting path.

If an incoming remote stem cannot be admitted because the remote budget or
global hard cap is exhausted, this node drops that anonymous copy locally
instead of converting resource pressure into an immediate public fluff event.
The previous Dandelion hop still retains its independently randomized embargo,
which remains the availability fallback.

This is resource-exhaustion containment, not Sybil resistance.
