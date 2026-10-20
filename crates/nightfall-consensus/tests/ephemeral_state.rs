use nightfall_consensus::SignedEphemeralProposal;
use nightfall_consensus::{Chain, CompactHeader, EphemeralChainAnchor};
use nightfall_consensus::{EphemeralStateError, EphemeralStatePool};
use nightfall_crypto::WalletKeys;
use nightfall_types::{Height, NetworkId};

#[test]
fn ephemeral_state_expires_exactly_at_its_height_boundary() {
    let mut pool = EphemeralStatePool::new();

    let id = pool
        .propose(b"tx_temp_9981", b"nightfall-wizard:42", Height(100), 4)
        .unwrap();

    assert!(pool.contains(&id));

    assert_eq!(pool.expire(Height(100)), 0);
    assert!(pool.contains(&id));

    assert_eq!(pool.expire(Height(101)), 0);
    assert!(pool.contains(&id));

    assert_eq!(pool.expire(Height(102)), 0);
    assert!(pool.contains(&id));

    assert_eq!(pool.expire(Height(103)), 0);
    assert!(pool.contains(&id));

    assert_eq!(pool.expire(Height(104)), 1);
    assert!(!pool.contains(&id));
}

#[test]
fn identical_ephemeral_state_produces_identical_id() {
    let mut a = EphemeralStatePool::new();
    let mut b = EphemeralStatePool::new();

    let id_a = a
        .propose(b"payment-A", b"payload-123", Height(500), 8)
        .unwrap();

    let id_b = b
        .propose(b"payment-A", b"payload-123", Height(500), 8)
        .unwrap();

    assert_eq!(id_a, id_b);
}

#[test]
fn changing_payload_changes_ephemeral_state_id() {
    /*
     * Separate pools are intentional:
     *
     * Inside one pool the same logical key may have only one live state.
     * Here we test only the cryptographic property that changing the payload
     * changes the deterministic state ID.
     */
    let mut a = EphemeralStatePool::new();
    let mut b = EphemeralStatePool::new();

    let id_a = a
        .propose(b"payment-A", b"amount=42", Height(1000), 10)
        .unwrap();

    let id_b = b
        .propose(b"payment-A", b"amount=43", Height(1000), 10)
        .unwrap();

    assert_ne!(
        id_a, id_b,
        "different payloads must produce different state IDs"
    );

    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
}

#[test]
fn zero_ttl_is_rejected() {
    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose(b"invalid", b"payload", Height(10), 0,),
        Err(EphemeralStateError::ZeroTtl)
    );

    assert!(pool.is_empty());
}

#[test]
fn expiration_order_does_not_affect_surviving_state() {
    let mut pool = EphemeralStatePool::new();

    let short = pool.propose(b"short", b"A", Height(100), 2).unwrap();

    let long = pool.propose(b"long", b"B", Height(100), 5).unwrap();

    assert_eq!(pool.expire(Height(102)), 1);

    assert!(!pool.contains(&short));
    assert!(pool.contains(&long));

    assert_eq!(pool.expire(Height(105)), 1);
    assert!(pool.is_empty());
}

#[test]
fn height_overflow_is_rejected_without_mutating_pool() {
    let mut pool = EphemeralStatePool::new();

    let result = pool.propose(b"overflow", b"payload", Height(u64::MAX), 1);

    assert_eq!(result, Err(EphemeralStateError::HeightOverflow));

    assert!(pool.is_empty());
}

#[test]
fn reproposing_identical_state_is_idempotent() {
    let mut pool = EphemeralStatePool::new();

    let first = pool
        .propose(b"same-key", b"same-payload", Height(100), 5)
        .unwrap();

    let second = pool
        .propose(b"same-key", b"same-payload", Height(100), 5)
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(pool.len(), 1);

    assert_eq!(pool.lifetime(&first), Some((Height(100), Height(105))));
}

#[test]
fn rejected_proposal_does_not_damage_existing_state() {
    let mut pool = EphemeralStatePool::new();

    let valid = pool.propose(b"valid", b"keep-me", Height(50), 10).unwrap();

    let result = pool.propose(b"overflow", b"must-not-enter", Height(u64::MAX), 1);

    assert_eq!(result, Err(EphemeralStateError::HeightOverflow));

    assert_eq!(pool.len(), 1);
    assert!(pool.contains(&valid));

    let (key, payload) = pool.state(&valid).expect("valid state disappeared");

    assert_eq!(key, &b"valid"[..]);
    assert_eq!(payload, &b"keep-me"[..]);
}

#[test]
fn state_round_trip_is_exact() {
    let mut pool = EphemeralStatePool::new();

    let key = b"\x00nightfall\xff";
    let payload = b"\x00\x01\x02payload\xfe\xff";

    let id = pool.propose(key, payload, Height(777), 23).unwrap();

    let (stored_key, stored_payload) = pool.state(&id).unwrap();

    assert_eq!(stored_key, key);
    assert_eq!(stored_payload, payload);

    assert_eq!(pool.lifetime(&id), Some((Height(777), Height(800))));
}

#[test]
fn changing_key_changes_id_even_with_identical_payload() {
    let mut pool = EphemeralStatePool::new();

    let a = pool
        .propose(b"key-A", b"identical-payload", Height(100), 10)
        .unwrap();

    let b = pool
        .propose(b"key-B", b"identical-payload", Height(100), 10)
        .unwrap();

    assert_ne!(a, b);
}

#[test]
fn lifetime_is_cryptographically_bound_into_id() {
    /*
     * Again use independent pools so conflict policy does not interfere
     * with the hash-property being tested.
     */
    let mut base_pool = EphemeralStatePool::new();
    let mut height_pool = EphemeralStatePool::new();
    let mut ttl_pool = EphemeralStatePool::new();

    let base = base_pool
        .propose(b"key", b"payload", Height(100), 10)
        .unwrap();

    let different_creation_height = height_pool
        .propose(b"key", b"payload", Height(101), 10)
        .unwrap();

    let different_ttl = ttl_pool
        .propose(b"key", b"payload", Height(100), 11)
        .unwrap();

    assert_ne!(
        base, different_creation_height,
        "creation height must be bound into the ID"
    );

    assert_ne!(
        base, different_ttl,
        "expiry lifetime must be bound into the ID"
    );

    assert_ne!(different_creation_height, different_ttl);
}

#[test]
fn empty_key_is_rejected() {
    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose(b"", b"payload", Height(100), 1,),
        Err(EphemeralStateError::EmptyKey)
    );

    assert!(pool.is_empty());
}

#[test]
fn oversized_key_is_rejected() {
    let mut pool = EphemeralStatePool::new();

    let key = vec![0u8; EphemeralStatePool::MAX_KEY_BYTES + 1];

    let result = pool.propose(&key, b"payload", Height(100), 1);

    assert_eq!(
        result,
        Err(EphemeralStateError::KeyTooLarge {
            len: EphemeralStatePool::MAX_KEY_BYTES + 1,
            max: EphemeralStatePool::MAX_KEY_BYTES,
        })
    );

    assert!(pool.is_empty());
}

#[test]
fn oversized_payload_is_rejected() {
    let mut pool = EphemeralStatePool::new();

    let payload = vec![0u8; EphemeralStatePool::MAX_STATE_BYTES + 1];

    let result = pool.propose(b"key", &payload, Height(100), 1);

    assert_eq!(
        result,
        Err(EphemeralStateError::StateTooLarge {
            len: EphemeralStatePool::MAX_STATE_BYTES + 1,
            max: EphemeralStatePool::MAX_STATE_BYTES,
        })
    );

    assert!(pool.is_empty());
}

#[test]
fn excessive_ttl_is_rejected() {
    let mut pool = EphemeralStatePool::new();

    let ttl = EphemeralStatePool::MAX_TTL_BLOCKS + 1;

    assert_eq!(
        pool.propose(b"key", b"payload", Height(100), ttl,),
        Err(EphemeralStateError::TtlTooLarge {
            got: ttl,
            max: EphemeralStatePool::MAX_TTL_BLOCKS,
        })
    );

    assert!(pool.is_empty());
}

#[test]
fn full_pool_rejects_new_state_but_accepts_idempotent_reproposal() {
    let mut pool = EphemeralStatePool::new();

    let mut first_id = None;

    for i in 0..EphemeralStatePool::MAX_ENTRIES {
        let key = (i as u64).to_le_bytes();

        let id = pool.propose(&key, b"x", Height(100), 10).unwrap();

        if i == 0 {
            first_id = Some(id);
        }
    }

    assert_eq!(pool.len(), EphemeralStatePool::MAX_ENTRIES);

    let result = pool.propose(b"one-too-many", b"x", Height(100), 10);

    assert_eq!(
        result,
        Err(EphemeralStateError::PoolFull {
            max: EphemeralStatePool::MAX_ENTRIES,
        })
    );

    let first_key = 0u64.to_le_bytes();

    let duplicate = pool.propose(&first_key, b"x", Height(100), 10).unwrap();

    assert_eq!(duplicate, first_id.unwrap());

    assert_eq!(pool.len(), EphemeralStatePool::MAX_ENTRIES);
}

#[test]
fn expired_states_free_capacity_before_new_proposal() {
    let mut pool = EphemeralStatePool::new();

    for i in 0..EphemeralStatePool::MAX_ENTRIES {
        let key = (i as u64).to_le_bytes();

        pool.propose(&key, b"x", Height(100), 1).unwrap();
    }

    assert_eq!(pool.len(), EphemeralStatePool::MAX_ENTRIES);

    let new_id = pool
        .propose(b"after-expiry", b"new", Height(101), 10)
        .unwrap();

    assert_eq!(pool.len(), 1);
    assert!(pool.contains(&new_id));
}

#[test]
fn conflicting_states_converge_independent_of_arrival_order() {
    /*
     * First compute both deterministic candidate IDs independently.
     */
    let mut probe_a = EphemeralStatePool::new();
    let mut probe_b = EphemeralStatePool::new();

    let id_a = probe_a
        .propose(b"agent-order-123", b"agent-A", Height(100), 10)
        .unwrap();

    let id_b = probe_b
        .propose(b"agent-order-123", b"agent-B", Height(100), 10)
        .unwrap();

    assert_ne!(id_a, id_b);

    let expected = if id_a.0 < id_b.0 { id_a } else { id_b };

    /*
     * Node 1 receives A then B.
     */
    let mut node_1 = EphemeralStatePool::new();

    let _ = node_1.propose(b"agent-order-123", b"agent-A", Height(100), 10);

    let _ = node_1.propose(b"agent-order-123", b"agent-B", Height(100), 10);

    /*
     * Node 2 receives exactly the same candidates,
     * but in the reverse order.
     */
    let mut node_2 = EphemeralStatePool::new();

    let _ = node_2.propose(b"agent-order-123", b"agent-B", Height(100), 10);

    let _ = node_2.propose(b"agent-order-123", b"agent-A", Height(100), 10);

    assert_eq!(node_1.active_id_for_key(b"agent-order-123"), Some(expected));

    assert_eq!(node_2.active_id_for_key(b"agent-order-123"), Some(expected));

    assert_eq!(
        node_1.active_id_for_key(b"agent-order-123"),
        node_2.active_id_for_key(b"agent-order-123")
    );

    assert_eq!(node_1.len(), 1);
    assert_eq!(node_2.len(), 1);
}

#[test]
fn replay_cannot_extend_live_state_lifetime() {
    let mut pool = EphemeralStatePool::new();

    let original = pool
        .propose(b"authorization-777", b"allow-payment", Height(100), 5)
        .unwrap();

    assert_eq!(pool.lifetime(&original), Some((Height(100), Height(105))));

    /*
     * Same logical authorization replayed one block later.
     * Without conflict protection this could silently extend its lifetime.
     */
    let replay = pool.propose(b"authorization-777", b"allow-payment", Height(101), 5);

    assert_eq!(
        replay,
        Err(EphemeralStateError::KeyConflict { existing: original })
    );

    assert_eq!(pool.lifetime(&original), Some((Height(100), Height(105))));

    assert_eq!(pool.len(), 1);
}

#[test]
fn expired_key_can_be_reused() {
    let mut pool = EphemeralStatePool::new();

    let old = pool
        .propose(b"resource-42", b"agent-A", Height(100), 2)
        .unwrap();

    assert_eq!(pool.active_id_for_key(b"resource-42"), Some(old));

    assert_eq!(pool.expire(Height(102)), 1);

    assert_eq!(pool.active_id_for_key(b"resource-42"), None);

    let new = pool
        .propose(b"resource-42", b"agent-B", Height(102), 4)
        .unwrap();

    assert_ne!(old, new);

    assert_eq!(pool.active_id_for_key(b"resource-42"), Some(new));

    assert_eq!(pool.len(), 1);
}

#[test]
fn independent_keys_can_coexist() {
    let mut pool = EphemeralStatePool::new();

    let a = pool.propose(b"order-A", b"state", Height(500), 10).unwrap();

    let b = pool.propose(b"order-B", b"state", Height(500), 10).unwrap();

    assert_ne!(a, b);
    assert_eq!(pool.len(), 2);

    assert_eq!(pool.active_id_for_key(b"order-A"), Some(a));

    assert_eq!(pool.active_id_for_key(b"order-B"), Some(b));
}

#[test]
fn earlier_creation_height_wins_regardless_of_arrival_order() {
    let key = b"machine-reservation";

    let mut older_probe = EphemeralStatePool::new();
    let older = older_probe.propose(key, b"older", Height(100), 10).unwrap();

    /*
     * Node A sees the old proposal first.
     */
    let mut node_a = EphemeralStatePool::new();

    assert_eq!(
        node_a.propose(key, b"older", Height(100), 10,).unwrap(),
        older
    );

    let later_result = node_a.propose(key, b"later", Height(101), 10);

    assert_eq!(
        later_result,
        Err(EphemeralStateError::KeyConflict { existing: older })
    );

    /*
     * Node B sees the later proposal first, then eventually
     * receives the older canonical proposal.
     */
    let mut node_b = EphemeralStatePool::new();

    node_b.propose(key, b"later", Height(101), 10).unwrap();

    let adopted_older = node_b.propose(key, b"older", Height(100), 10).unwrap();

    assert_eq!(adopted_older, older);

    assert_eq!(node_a.active_id_for_key(key), Some(older));

    assert_eq!(node_b.active_id_for_key(key), Some(older));
}

#[test]
fn longer_ttl_cannot_replace_shorter_ttl_at_same_creation_height() {
    let key = b"temporary-authorization";

    let mut short_probe = EphemeralStatePool::new();

    let short = short_probe
        .propose(key, b"authorized", Height(500), 5)
        .unwrap();

    /*
     * Node 1 receives short TTL first, then attempted extension.
     */
    let mut node_1 = EphemeralStatePool::new();

    node_1.propose(key, b"authorized", Height(500), 5).unwrap();

    assert_eq!(
        node_1.propose(key, b"authorized", Height(500), 20,),
        Err(EphemeralStateError::KeyConflict { existing: short })
    );

    /*
     * Node 2 sees the longer proposal first.
     * Receiving the shorter canonical proposal later must replace it.
     */
    let mut node_2 = EphemeralStatePool::new();

    node_2.propose(key, b"authorized", Height(500), 20).unwrap();

    assert_eq!(
        node_2.propose(key, b"authorized", Height(500), 5,).unwrap(),
        short
    );

    assert_eq!(node_1.active_id_for_key(key), Some(short));

    assert_eq!(node_2.active_id_for_key(key), Some(short));

    assert_eq!(node_1.lifetime(&short), Some((Height(500), Height(505))));

    assert_eq!(node_2.lifetime(&short), Some((Height(500), Height(505))));
}

#[test]
fn signed_ephemeral_proposal_is_accepted() {
    let keys = WalletKeys::from_seed([11u8; 32]);

    let proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &keys,
        b"agent-order-9001",
        b"reserve-gpu",
        Height(100),
        10,
    );

    assert!(proposal.verify_signature());

    let mut pool = EphemeralStatePool::new();

    let id = pool
        .propose_signed(NetworkId::Devnet, proposal.created_height, &proposal)
        .unwrap();

    assert!(pool.contains(&id));
    assert_eq!(pool.len(), 1);
}

#[test]
fn tampered_signed_payload_is_rejected() {
    let keys = WalletKeys::from_seed([12u8; 32]);

    let mut proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &keys,
        b"payment-intent",
        b"amount=42",
        Height(200),
        20,
    );

    assert!(proposal.verify_signature());

    /*
     * Attacker changes the state after the owner signed it.
     */
    proposal.state_data = b"amount=42000000".to_vec();

    assert!(!proposal.verify_signature());

    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose_signed(NetworkId::Devnet, proposal.created_height, &proposal,),
        Err(EphemeralStateError::BadSignature)
    );

    assert!(pool.is_empty());
}

#[test]
fn signed_proposal_cannot_cross_networks() {
    let keys = WalletKeys::from_seed([13u8; 32]);

    let proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &keys,
        b"authorization",
        b"allow",
        Height(300),
        5,
    );

    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose_signed(NetworkId::Mainnet, proposal.created_height, &proposal,),
        Err(EphemeralStateError::WrongNetwork)
    );

    assert!(pool.is_empty());
}

#[test]
fn replacing_authority_invalidates_signature() {
    let alice = WalletKeys::from_seed([14u8; 32]);
    let bob = WalletKeys::from_seed([15u8; 32]);

    let mut proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &alice,
        b"reservation",
        b"resource-77",
        Height(400),
        12,
    );

    assert!(proposal.verify_signature());

    /*
     * A relay cannot rewrite Alice's proposal to claim Bob authorised it.
     */
    proposal.authority = bob.address();

    assert!(!proposal.verify_signature());

    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose_signed(NetworkId::Devnet, proposal.created_height, &proposal,),
        Err(EphemeralStateError::BadSignature)
    );
}

#[test]
fn authorities_have_separate_key_namespaces() {
    let alice = WalletKeys::from_seed([16u8; 32]);
    let bob = WalletKeys::from_seed([17u8; 32]);

    let alice_proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &alice,
        b"order-1",
        b"alice-state",
        Height(500),
        10,
    );

    let bob_proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &bob,
        b"order-1",
        b"bob-state",
        Height(500),
        10,
    );

    assert_ne!(alice_proposal.scoped_key(), bob_proposal.scoped_key());

    let mut pool = EphemeralStatePool::new();

    let alice_id = pool
        .propose_signed(
            NetworkId::Devnet,
            alice_proposal.created_height,
            &alice_proposal,
        )
        .unwrap();

    let bob_id = pool
        .propose_signed(
            NetworkId::Devnet,
            bob_proposal.created_height,
            &bob_proposal,
        )
        .unwrap();

    assert_ne!(alice_id, bob_id);
    assert_eq!(pool.len(), 2);
}

#[test]
fn future_dated_signed_proposal_is_rejected() {
    let legitimate_keys = WalletKeys::from_seed([21u8; 32]);
    let attacker_keys = WalletKeys::from_seed([22u8; 32]);

    let legitimate = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &legitimate_keys,
        b"legitimate-state",
        b"keep-alive",
        Height(100),
        20,
    );

    let future = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &attacker_keys,
        b"future-state",
        b"attack",
        Height(10_000),
        10,
    );

    let mut pool = EphemeralStatePool::new();

    let legitimate_id = pool
        .propose_signed(NetworkId::Devnet, Height(100), &legitimate)
        .unwrap();

    assert!(pool.contains(&legitimate_id));
    assert_eq!(pool.len(), 1);

    assert_eq!(
        pool.propose_signed(NetworkId::Devnet, Height(100), &future,),
        Err(EphemeralStateError::FutureHeight {
            proposal: 10_000,
            observed: 100,
        })
    );

    /*
     * Critical invariant:
     * the malicious future timestamp must not expire or mutate
     * legitimate current state.
     */
    assert!(pool.contains(&legitimate_id));
    assert_eq!(pool.len(), 1);

    assert_eq!(
        pool.lifetime(&legitimate_id),
        Some((Height(100), Height(120)))
    );
}

#[test]
fn already_expired_signed_proposal_cannot_be_resurrected() {
    let keys = WalletKeys::from_seed([23u8; 32]);

    let proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &keys,
        b"old-agent-intent",
        b"expired",
        Height(100),
        5,
    );

    assert!(proposal.verify_signature());

    let mut pool = EphemeralStatePool::new();

    /*
     * Lifetime is [100,105).
     * At canonical height 105 the proposal is already dead.
     */
    assert_eq!(
        pool.propose_signed(NetworkId::Devnet, Height(105), &proposal,),
        Err(EphemeralStateError::ProposalExpired {
            expires: 105,
            observed: 105,
        })
    );

    assert!(pool.is_empty());
}

#[test]
fn empty_ephemeral_state_has_deterministic_root() {
    let a = EphemeralStatePool::new();
    let b = EphemeralStatePool::new();

    assert_eq!(a.state_root(), b.state_root());

    assert_eq!(a.ordered_state_ids(), Vec::new());
}

#[test]
fn ephemeral_root_is_independent_of_insertion_order() {
    let mut node_a = EphemeralStatePool::new();
    let mut node_b = EphemeralStatePool::new();

    node_a
        .propose(b"order-A", b"state-A", Height(100), 10)
        .unwrap();

    node_a
        .propose(b"order-B", b"state-B", Height(100), 10)
        .unwrap();

    /*
     * Same complete state set, reverse insertion order.
     */
    node_b
        .propose(b"order-B", b"state-B", Height(100), 10)
        .unwrap();

    node_b
        .propose(b"order-A", b"state-A", Height(100), 10)
        .unwrap();

    assert_eq!(node_a.ordered_state_ids(), node_b.ordered_state_ids());

    assert_eq!(node_a.state_root(), node_b.state_root());
}

#[test]
fn ephemeral_root_changes_when_state_changes() {
    let mut a = EphemeralStatePool::new();
    let mut b = EphemeralStatePool::new();

    a.propose(b"payment", b"amount=42", Height(100), 10)
        .unwrap();

    b.propose(b"payment", b"amount=43", Height(100), 10)
        .unwrap();

    assert_ne!(a.state_root(), b.state_root());
}

#[test]
fn ephemeral_root_at_excludes_expired_state() {
    let mut pool = EphemeralStatePool::new();

    let short = pool.propose(b"short-lived", b"A", Height(100), 2).unwrap();

    let long = pool.propose(b"long-lived", b"B", Height(100), 10).unwrap();

    let before = pool.state_root_at(Height(100));

    assert!(pool.contains(&short));
    assert!(pool.contains(&long));

    let after = pool.state_root_at(Height(102));

    assert_ne!(before, after);

    assert!(!pool.contains(&short));
    assert!(pool.contains(&long));

    assert_eq!(pool.ordered_state_ids(), vec![long]);
}

#[test]
fn converged_conflicts_produce_identical_state_roots() {
    let key = b"shared-agent-intent";

    let mut node_a = EphemeralStatePool::new();
    let mut node_b = EphemeralStatePool::new();

    /*
     * Node A observes candidate A and then candidate B.
     */
    let _ = node_a.propose(key, b"candidate-A", Height(700), 10);

    let _ = node_a.propose(key, b"candidate-B", Height(700), 10);

    /*
     * Node B observes exactly the same candidates in reverse order.
     */
    let _ = node_b.propose(key, b"candidate-B", Height(700), 10);

    let _ = node_b.propose(key, b"candidate-A", Height(700), 10);

    assert_eq!(node_a.active_id_for_key(key), node_b.active_id_for_key(key));

    assert_eq!(node_a.ordered_state_ids(), node_b.ordered_state_ids());

    assert_eq!(node_a.state_root(), node_b.state_root());
}

fn synthetic_chain_header(
    height: u64,
    hash_byte: u8,
    prev_hash: nightfall_types::Hash256,
) -> CompactHeader {
    CompactHeader {
        height,
        hash: nightfall_types::Hash256([hash_byte; 32]),
        prev_hash,
        timestamp_unix: height,
        difficulty: 1,
    }
}

#[test]
fn chain_bound_signed_proposal_is_accepted() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    let h0 = synthetic_chain_header(0, 31, chain.genesis_hash);

    chain.headers.push(h0);

    let anchor = EphemeralChainAnchor::from_chain(&chain);

    let keys = WalletKeys::from_seed([31u8; 32]);

    let proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"gpu-reservation",
        b"reserved",
        anchor,
        Height(0),
        10,
    );

    assert!(proposal.verify_signature());

    let mut pool = EphemeralStatePool::new();

    let id = pool.propose_signed_on_chain(&chain, &proposal).unwrap();

    assert!(pool.contains(&id));

    assert_eq!(pool.anchor_for_id(&id), Some(anchor));
}

#[test]
fn unanchored_proposal_is_rejected_by_chain_bound_path() {
    let chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    let keys = WalletKeys::from_seed([32u8; 32]);

    let proposal = SignedEphemeralProposal::sign(
        NetworkId::Devnet,
        &keys,
        b"unanchored",
        b"state",
        Height(0),
        10,
    );

    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose_signed_on_chain(&chain, &proposal,),
        Err(EphemeralStateError::MissingChainAnchor)
    );

    assert!(pool.is_empty());
}

#[test]
fn anchored_state_survives_normal_chain_extension() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    let h0 = synthetic_chain_header(0, 41, chain.genesis_hash);

    chain.headers.push(h0);

    let anchor = EphemeralChainAnchor::from_chain(&chain);

    let keys = WalletKeys::from_seed([33u8; 32]);

    let proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"agent-intent",
        b"active",
        anchor,
        Height(0),
        10,
    );

    let mut pool = EphemeralStatePool::new();

    let id = pool.propose_signed_on_chain(&chain, &proposal).unwrap();

    let h1 = synthetic_chain_header(1, 42, chain.headers[0].hash);

    chain.headers.push(h1);

    assert_eq!(pool.reconcile_with_chain(&chain), 0);

    assert!(pool.contains(&id));

    assert_eq!(pool.anchor_for_id(&id), Some(anchor));
}

#[test]
fn reorg_that_removes_anchor_drops_ephemeral_state() {
    let mut original_chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    original_chain
        .headers
        .push(synthetic_chain_header(0, 51, original_chain.genesis_hash));

    let anchor = EphemeralChainAnchor::from_chain(&original_chain);

    let keys = WalletKeys::from_seed([34u8; 32]);

    let proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"temporary-order",
        b"reserved",
        anchor,
        Height(0),
        20,
    );

    let mut pool = EphemeralStatePool::new();

    let id = pool
        .propose_signed_on_chain(&original_chain, &proposal)
        .unwrap();

    assert!(pool.contains(&id));

    /*
     * Alternative canonical chain replaces height 0.
     */
    let mut reorged_chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    reorged_chain
        .headers
        .push(synthetic_chain_header(0, 52, reorged_chain.genesis_hash));

    assert_eq!(pool.reconcile_with_chain(&reorged_chain), 1);

    assert!(!pool.contains(&id));

    assert_eq!(pool.anchor_for_id(&id), None);
}

#[test]
fn reorg_above_anchor_keeps_ephemeral_state() {
    let mut original_chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    original_chain
        .headers
        .push(synthetic_chain_header(0, 61, original_chain.genesis_hash));

    let anchor = EphemeralChainAnchor::from_chain(&original_chain);

    let keys = WalletKeys::from_seed([35u8; 32]);

    let proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"persistent-through-upper-reorg",
        b"state",
        anchor,
        Height(0),
        20,
    );

    let mut pool = EphemeralStatePool::new();

    let id = pool
        .propose_signed_on_chain(&original_chain, &proposal)
        .unwrap();

    /*
     * Both chains retain the anchored height-0 block.
     * Only height 1 differs.
     */
    let mut reorged_chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    reorged_chain
        .headers
        .push(synthetic_chain_header(0, 61, reorged_chain.genesis_hash));

    reorged_chain
        .headers
        .push(synthetic_chain_header(1, 63, reorged_chain.headers[0].hash));

    assert_eq!(pool.reconcile_with_chain(&reorged_chain), 0);

    assert!(pool.contains(&id));
}

#[test]
fn tampering_chain_anchor_invalidates_signature() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    chain
        .headers
        .push(synthetic_chain_header(0, 71, chain.genesis_hash));

    let anchor = EphemeralChainAnchor::from_chain(&chain);

    let keys = WalletKeys::from_seed([36u8; 32]);

    let mut proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"anchor-authentication",
        b"state",
        anchor,
        Height(0),
        10,
    );

    assert!(proposal.verify_signature());

    proposal.anchor = Some(EphemeralChainAnchor {
        tip_hash: nightfall_types::Hash256([99u8; 32]),
        tip_height: Some(Height(0)),
    });

    assert!(!proposal.verify_signature());
}

#[test]
fn signed_ephemeral_proposal_survives_wire_roundtrip() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    chain
        .headers
        .push(synthetic_chain_header(0, 81, chain.genesis_hash));

    let anchor = EphemeralChainAnchor::from_chain(&chain);

    let keys = WalletKeys::from_seed([81u8; 32]);

    let original = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"wire-order-1",
        b"reserve-compute-slot",
        anchor,
        Height(0),
        25,
    );

    assert!(original.verify_signature());

    /*
     * This is the same JSON representation the P2P layer will
     * eventually place inside a network message.
     */
    let encoded = serde_json::to_vec(&original).unwrap();

    let decoded: SignedEphemeralProposal = serde_json::from_slice(&encoded).unwrap();

    /*
     * No authenticated field may change during transport.
     */
    assert_eq!(decoded, original);

    assert_eq!(decoded.signing_message(), original.signing_message());

    assert_eq!(decoded.scoped_key(), original.scoped_key());

    assert_eq!(decoded.anchor, Some(anchor));

    assert!(decoded.verify_signature());

    /*
     * And the deserialised object must pass the real
     * chain-bound admission path.
     */
    let mut pool = EphemeralStatePool::new();

    let id = pool.propose_signed_on_chain(&chain, &decoded).unwrap();

    assert!(pool.contains(&id));

    assert_eq!(pool.anchor_for_id(&id), Some(anchor));
}

#[test]
fn tampering_serialized_ephemeral_proposal_breaks_authentication() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    chain
        .headers
        .push(synthetic_chain_header(0, 82, chain.genesis_hash));

    let anchor = EphemeralChainAnchor::from_chain(&chain);

    let keys = WalletKeys::from_seed([82u8; 32]);

    let original = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"wire-auth-test",
        b"amount=42",
        anchor,
        Height(0),
        30,
    );

    let encoded = serde_json::to_vec(&original).unwrap();

    let mut decoded: SignedEphemeralProposal = serde_json::from_slice(&encoded).unwrap();

    assert!(decoded.verify_signature());

    /*
     * Simulate a malicious relay modifying a field after
     * deserialisation.
     */
    decoded.state_data = b"amount=42000000".to_vec();

    assert!(!decoded.verify_signature());

    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose_signed_on_chain(&chain, &decoded,),
        Err(EphemeralStateError::BadSignature)
    );

    assert!(pool.is_empty());
}

#[test]
fn tampering_serialized_anchor_breaks_authentication() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    chain
        .headers
        .push(synthetic_chain_header(0, 83, chain.genesis_hash));

    let anchor = EphemeralChainAnchor::from_chain(&chain);

    let keys = WalletKeys::from_seed([83u8; 32]);

    let original = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"wire-anchor-test",
        b"state",
        anchor,
        Height(0),
        40,
    );

    let encoded = serde_json::to_vec(&original).unwrap();

    let mut decoded: SignedEphemeralProposal = serde_json::from_slice(&encoded).unwrap();

    decoded.anchor = Some(EphemeralChainAnchor {
        tip_hash: nightfall_types::Hash256([200u8; 32]),
        tip_height: Some(Height(0)),
    });

    assert!(!decoded.verify_signature());

    let mut pool = EphemeralStatePool::new();

    assert_eq!(
        pool.propose_signed_on_chain(&chain, &decoded,),
        Err(EphemeralStateError::AnchorNotCanonical)
    );

    assert!(pool.is_empty());
}

#[test]
fn per_authority_quota_bounds_chain_bound_flood() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();

    chain
        .headers
        .push(synthetic_chain_header(0, 91, chain.genesis_hash));

    let anchor = EphemeralChainAnchor::from_chain(&chain);
    let keys = WalletKeys::from_seed([91u8; 32]);
    let mut pool = EphemeralStatePool::new();
    let cap = EphemeralStatePool::MAX_STATES_PER_AUTHORITY;

    for i in 0..cap {
        let key = format!("flood-{i}").into_bytes();
        let prop = SignedEphemeralProposal::sign_anchored(
            NetworkId::Devnet,
            &keys,
            &key,
            b"state",
            anchor,
            Height(0),
            100,
        );
        pool.propose_signed_on_chain(&chain, &prop).unwrap();
    }

    let over = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"flood-over",
        b"state",
        anchor,
        Height(0),
        100,
    );

    assert_eq!(
        pool.propose_signed_on_chain(&chain, &over),
        Err(EphemeralStateError::AuthorityQuotaExceeded { max: cap }),
    );

    let other = WalletKeys::from_seed([92u8; 32]);
    let fresh = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &other,
        b"fresh-authority",
        b"state",
        anchor,
        Height(0),
        100,
    );
    pool.propose_signed_on_chain(&chain, &fresh).unwrap();
}

#[test]
fn anchor_pointing_beyond_chain_tip_is_not_canonical() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 101, chain.genesis_hash));

    let anchor = EphemeralChainAnchor {
        tip_hash: nightfall_types::Hash256([1u8; 32]),
        tip_height: Some(Height(5)),
    };

    let keys = WalletKeys::from_seed([101u8; 32]);
    let proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"future-anchor",
        b"state",
        anchor,
        Height(5),
        10,
    );

    let mut pool = EphemeralStatePool::new();
    assert_eq!(
        pool.propose_signed_on_chain(&chain, &proposal),
        Err(EphemeralStateError::AnchorNotCanonical),
    );
    assert!(pool.is_empty());
}

#[test]
fn anchor_with_none_height_requires_genesis_hash() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 102, chain.genesis_hash));

    let anchor = EphemeralChainAnchor {
        tip_hash: chain.headers[0].hash,
        tip_height: None,
    };

    let keys = WalletKeys::from_seed([102u8; 32]);
    let proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"fake-genesis",
        b"state",
        anchor,
        Height(0),
        10,
    );

    let mut pool = EphemeralStatePool::new();
    assert_eq!(
        pool.propose_signed_on_chain(&chain, &proposal),
        Err(EphemeralStateError::AnchorNotCanonical),
    );
    assert!(pool.is_empty());
}

#[test]
fn anchor_hash_at_wrong_height_is_not_canonical() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 103, chain.genesis_hash));
    chain
        .headers
        .push(synthetic_chain_header(1, 104, chain.headers[0].hash));

    let anchor = EphemeralChainAnchor {
        tip_hash: chain.headers[0].hash,
        tip_height: Some(Height(1)),
    };

    let keys = WalletKeys::from_seed([103u8; 32]);
    let proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"wrong-height",
        b"state",
        anchor,
        Height(1),
        10,
    );

    let mut pool = EphemeralStatePool::new();
    assert_eq!(
        pool.propose_signed_on_chain(&chain, &proposal),
        Err(EphemeralStateError::AnchorNotCanonical),
    );
    assert!(pool.is_empty());
}

#[test]
fn expired_states_free_authority_quota() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 110, chain.genesis_hash));
    let anchor = EphemeralChainAnchor::from_chain(&chain);
    let keys = WalletKeys::from_seed([110u8; 32]);
    let cap = EphemeralStatePool::MAX_STATES_PER_AUTHORITY;

    let mut pool = EphemeralStatePool::new();

    for i in 0..cap {
        let key = format!("expire-{i}").into_bytes();
        let prop = SignedEphemeralProposal::sign_anchored(
            NetworkId::Devnet,
            &keys,
            &key,
            b"state",
            anchor,
            Height(0),
            5,
        );
        pool.propose_signed_on_chain(&chain, &prop).unwrap();
    }

    let denied = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"expire-denied",
        b"state",
        anchor,
        Height(0),
        5,
    );
    assert_eq!(
        pool.propose_signed_on_chain(&chain, &denied),
        Err(EphemeralStateError::AuthorityQuotaExceeded { max: cap }),
    );

    for h in 1..=5u64 {
        chain.headers.push(synthetic_chain_header(
            h,
            (110u64 + h) as u8,
            chain.headers[(h - 1) as usize].hash,
        ));
    }
    let dropped = pool.reconcile_with_chain(&chain);
    assert!(dropped >= cap);

    let fresh = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"expire-fresh",
        b"state",
        anchor,
        Height(0),
        100,
    );
    pool.propose_signed_on_chain(&chain, &fresh).unwrap();
}

#[test]
fn conflict_replacement_does_not_leak_authority_quota() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 120, chain.genesis_hash));
    let anchor = EphemeralChainAnchor::from_chain(&chain);
    let keys = WalletKeys::from_seed([120u8; 32]);
    let cap = EphemeralStatePool::MAX_STATES_PER_AUTHORITY;

    let mut pool = EphemeralStatePool::new();

    // Repeatedly propose different payloads for the SAME logical key
    // from the SAME authority. Later candidates have LONGER TTL and
    // therefore lose the deterministic tie-break; the losing branch
    // must remove all secondary index entries for the incumbent.
    for i in 0..cap {
        let payload = format!("payload-{i}").into_bytes();
        let prop = SignedEphemeralProposal::sign_anchored(
            NetworkId::Devnet,
            &keys,
            b"hot-key",
            &payload,
            anchor,
            Height(0),
            cap as u64 + i as u64,
        );
        match pool.propose_signed_on_chain(&chain, &prop) {
            Ok(_) | Err(EphemeralStateError::KeyConflict { .. }) => {}
            e => panic!("unexpected: {:?}", e),
        }
    }

    // Exactly one live state for the authority.
    for i in 0..(cap - 1) {
        let key = format!("other-key-{i}").into_bytes();
        let prop = SignedEphemeralProposal::sign_anchored(
            NetworkId::Devnet,
            &keys,
            &key,
            b"state",
            anchor,
            Height(0),
            100,
        );
        pool.propose_signed_on_chain(&chain, &prop).unwrap();
    }

    let over = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"over-cap",
        b"state",
        anchor,
        Height(0),
        100,
    );
    assert_eq!(
        pool.propose_signed_on_chain(&chain, &over),
        Err(EphemeralStateError::AuthorityQuotaExceeded { max: cap }),
    );
}

#[test]
fn tampering_creation_height_invalidates_signature() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 130, chain.genesis_hash));
    let anchor = EphemeralChainAnchor::from_chain(&chain);
    let keys = WalletKeys::from_seed([130u8; 32]);
    let mut proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"ch-tamper",
        b"state",
        anchor,
        Height(0),
        10,
    );
    assert!(proposal.verify_signature());
    proposal.created_height = Height(1);
    assert!(!proposal.verify_signature());
}

#[test]
fn tampering_ttl_invalidates_signature() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 131, chain.genesis_hash));
    let anchor = EphemeralChainAnchor::from_chain(&chain);
    let keys = WalletKeys::from_seed([131u8; 32]);
    let mut proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"ttl-tamper",
        b"state",
        anchor,
        Height(0),
        10,
    );
    assert!(proposal.verify_signature());
    proposal.ttl_blocks = 11;
    assert!(!proposal.verify_signature());
}

#[test]
fn tampering_logical_key_invalidates_signature() {
    let mut chain = Chain::new_fair(NetworkId::Devnet).unwrap();
    chain
        .headers
        .push(synthetic_chain_header(0, 132, chain.genesis_hash));
    let anchor = EphemeralChainAnchor::from_chain(&chain);
    let keys = WalletKeys::from_seed([132u8; 32]);
    let mut proposal = SignedEphemeralProposal::sign_anchored(
        NetworkId::Devnet,
        &keys,
        b"key-tamper",
        b"state",
        anchor,
        Height(0),
        10,
    );
    assert!(proposal.verify_signature());
    proposal.logical_key = b"different-key".to_vec();
    assert!(!proposal.verify_signature());
}
