//! Adversarial invariants for Dandelion++.
//!
//! These tests deliberately cross module boundaries. They verify the privacy
//! boundary using the same `Mempool` implementation that mining consumes,
//! rather than testing only the Dandelion helper data structures.

use crate::dandelion::{StemInsert, StemPool};
use crate::session::{outbound_key, stem_tx_to, SessionPool};
use nightfall_consensus::Mempool;
use nightfall_ledger::Transaction;
use std::net::{TcpListener, TcpStream};

fn tx() -> Transaction {
    Transaction {
        version: 8,
        inputs: Vec::new(),
        outputs: Vec::new(),
        kernels: Vec::new(),
    }
}

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let client = TcpStream::connect(addr).unwrap();
    let (server, _) = listener.accept().unwrap();

    (client, server)
}

#[test]
fn stem_is_not_mineable_before_promotion() {
    let tx = tx();
    let id = tx.txid().to_hex();

    let mut stempool = StemPool::default();
    let mut mempool = Mempool::default();

    assert_eq!(
        stempool.insert_new(id.clone(), tx.clone(), "out:privacy-hop".into(), 100,),
        StemInsert::Inserted
    );

    // Critical privacy boundary: mining only sees the ordinary mempool.
    assert!(mempool.select_for_block(16).is_empty());
    assert!(mempool.first_seen(&id).is_none());
    assert!(stempool.contains(&id));

    let mut due = stempool.take_due(100);

    assert_eq!(due.len(), 1);

    let entry = due.pop().unwrap();

    assert!(mempool.insert(entry.value, 100));
    assert!(!stempool.contains(&id));

    let selected = mempool.select_for_block(16);

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].txid().to_hex(), id);
}

#[test]
fn embargo_release_happens_exactly_once() {
    let tx = tx();
    let id = tx.txid().to_hex();

    let mut stempool = StemPool::default();

    assert_eq!(
        stempool.insert_new(id, tx, "out:black-hole".into(), 50,),
        StemInsert::Inserted
    );

    assert!(stempool.take_due(49).is_empty());

    let first = stempool.take_due(50);

    assert_eq!(first.len(), 1);

    // A second timer pass must never re-promote the same transaction.
    assert!(stempool.take_due(50).is_empty());
    assert!(stempool.take_due(u64::MAX).is_empty());
    assert!(stempool.is_empty());
}

#[test]
fn fluff_cancels_the_pending_embargo() {
    let tx = tx();
    let id = tx.txid().to_hex();

    let mut stempool = StemPool::default();
    let mut mempool = Mempool::default();

    assert_eq!(
        stempool.insert_new(id.clone(), tx.clone(), "out:stem-hop".into(), 500,),
        StemInsert::Inserted
    );

    // Equivalent to observing ordinary diffusion for the same transaction.
    let stem = stempool.remove(&id).expect("stem entry");

    assert!(mempool.insert(stem.value, 100));

    // The old embargo may fire later, but there is nothing left to promote.
    assert!(stempool.take_due(500).is_empty());

    let selected = mempool.select_for_block(16);

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].txid().to_hex(), id);
}

#[test]
fn duplicate_stem_loop_breaker_produces_one_public_copy() {
    let tx = tx();
    let id = tx.txid().to_hex();

    let mut stempool = StemPool::default();
    let mut mempool = Mempool::default();

    assert_eq!(
        stempool.insert_new(id.clone(), tx.clone(), "out:first-hop".into(), 100,),
        StemInsert::Inserted
    );

    // A stem returning through the privacy graph is evidence of a loop.
    assert_eq!(
        stempool.insert_new(id.clone(), tx.clone(), "out:second-hop".into(), 200,),
        StemInsert::Duplicate
    );

    let original = stempool.remove(&id).expect("original stem");

    assert!(mempool.insert(original.value, 10));

    // Even if another path attempts the same public promotion, Mempool
    // deduplication ensures one mineable copy.
    assert!(!mempool.insert(tx, 11));

    assert_eq!(mempool.len(), 1);
    assert_eq!(mempool.select_for_block(16).len(), 1);
}

#[test]
fn legacy_session_cannot_be_used_as_a_stem_route() {
    let (writer, _reader) = pair();

    let sessions = SessionPool::new();
    let route = outbound_key("127.0.0.1:17891");

    // Plain insert intentionally means no negotiated Dandelion capability.
    let legacy = sessions.insert(route.clone(), writer, true);

    let tx = tx();

    let err = legacy
        .send_stem_tx(&tx)
        .expect_err("legacy session must reject stem semantics");

    assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);

    assert!(!stem_tx_to(&sessions, &route, &tx,));

    assert!(sessions.dandelion_outbound_keys().is_empty());
}
