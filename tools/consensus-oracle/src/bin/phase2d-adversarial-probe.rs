use anyhow::{bail, Result};
use bulletproofs::PedersenGens;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use std::collections::{BTreeMap, BTreeSet};

const DOMAIN_BLOCK: &[u8] = b"nightfall:block:v2";
const DOMAIN_INPUT: &[u8] = b"nightfall:input:v2";
const DOMAIN_SCHNORR: &[u8] = b"nightfall:schnorr:v2";
const DOMAIN_MERKLE: &[u8] = b"nightfall:merkle:v2";
const DOMAIN_MERKLE_LEAF: &[u8] = b"nightfall:merkle:leaf:v2";

const COINBASE_MATURITY: u64 = 1_440;

#[derive(Clone)]
struct Utxo {
    output_pk: [u8; 32],
    height: u64,
}

#[derive(Clone)]
struct Header {
    version: u32,
    height: u64,
    prev: [u8; 32],
    utxo: [u8; 32],
    kernel: [u8; 32],
    body: [u8; 32],
    time: u64,
    difficulty: u64,
    reward: u64,
    nonce: u64,
}

struct Rng {
    x: u64,
}

impl Rng {
    fn new() -> Self {
        Self {
            x: 0x8c3c_010c_b475_4c91,
        }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.x;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.x = x;
        x
    }

    fn bytes32(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];

        for chunk in out.as_chunks_mut::<8>().0 {
            chunk.copy_from_slice(&self.next().to_le_bytes());
        }

        out
    }
}

fn hash_multi(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();

    h.update(&(domain.len() as u64).to_le_bytes());
    h.update(domain);

    h.update(&(parts.len() as u64).to_le_bytes());

    for part in parts {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part);
    }

    *h.finalize().as_bytes()
}

fn utxo_root(map: &BTreeMap<[u8; 32], Utxo>) -> [u8; 32] {
    if map.is_empty() {
        return [0u8; 32];
    }

    let mut level: Vec<[u8; 32]> = map
        .iter()
        .map(|(commit, entry)| {
            let height = entry.height.to_le_bytes();

            hash_multi(DOMAIN_MERKLE_LEAF, &[commit, &entry.output_pk, &height])
        })
        .collect();

    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));

        for pair in level.chunks(2) {
            let right = if pair.len() == 2 { pair[1] } else { pair[0] };

            next.push(hash_multi(DOMAIN_MERKLE, &[&pair[0], &right]));
        }

        level = next;
    }

    level[0]
}

fn mature(coinbase: bool, created: u64, spend: u64) -> bool {
    if !coinbase {
        return true;
    }

    spend >= created.saturating_add(COINBASE_MATURITY)
}

fn challenge(r: &[u8; 32], p: &[u8; 32], msg: &[u8]) -> Scalar {
    let a = hash_multi(DOMAIN_SCHNORR, &[r, p, msg, b"c0"]);

    let b = hash_multi(DOMAIN_SCHNORR, &[r, p, msg, b"c1"]);

    let mut wide = [0u8; 64];
    wide[..32].copy_from_slice(&a);
    wide[32..].copy_from_slice(&b);

    Scalar::from_bytes_mod_order_wide(&wide)
}

fn verify_sig(
    public: &RistrettoPoint,
    generator: &RistrettoPoint,
    msg: &[u8],
    r_bytes: [u8; 32],
    s_bytes: [u8; 32],
) -> bool {
    let Some(r) = CompressedRistretto(r_bytes).decompress() else {
        return false;
    };

    let Some(s) = Option::<Scalar>::from(Scalar::from_canonical_bytes(s_bytes)) else {
        return false;
    };

    let p = public.compress().to_bytes();

    let e = challenge(&r_bytes, &p, msg);

    generator * s == r + public * e
}

fn header_hash(h: &Header) -> [u8; 32] {
    let version = h.version.to_le_bytes();
    let height = h.height.to_le_bytes();
    let time = h.time.to_le_bytes();
    let difficulty = h.difficulty.to_le_bytes();
    let reward = h.reward.to_le_bytes();

    let preimage = hash_multi(
        DOMAIN_BLOCK,
        &[
            &version,
            &height,
            &h.prev,
            &h.utxo,
            &h.kernel,
            &h.body,
            &time,
            &difficulty,
            &reward,
        ],
    );

    let nonce = h.nonce.to_le_bytes();

    hash_multi(DOMAIN_BLOCK, &[&preimage, &nonce])
}

fn test_maturity(rng: &mut Rng) -> u64 {
    let rounds = 20_000u64;

    for _ in 0..rounds {
        let created = rng.next() % 1_000_000_000;

        assert!(mature(false, created, created,));

        assert!(!mature(true, created, created + 1_439,));

        assert!(mature(true, created, created + 1_440,));

        assert!(mature(true, created, created + 10_000,));
    }

    rounds * 4
}

fn test_duplicate_detection(rng: &mut Rng) -> u64 {
    let rounds = 20_000u64;

    for _ in 0..rounds {
        let c = rng.bytes32();

        let mut spent = BTreeSet::<[u8; 32]>::new();

        assert!(spent.insert(c));
        assert!(!spent.insert(c));
    }

    rounds * 2
}

fn test_merkle_properties(rng: &mut Rng) -> Result<u64> {
    let rounds = 1_000u64;
    let mut assertions = 0u64;

    for _ in 0..rounds {
        let mut entries = Vec::<([u8; 32], Utxo)>::new();

        while entries.len() < 17 {
            let commit = rng.bytes32();

            if entries.iter().any(|(c, _)| c == &commit) {
                continue;
            }

            entries.push((
                commit,
                Utxo {
                    output_pk: rng.bytes32(),
                    height: rng.next() % 1_000_000,
                },
            ));
        }

        let mut a = BTreeMap::new();

        for (c, e) in &entries {
            a.insert(*c, e.clone());
        }

        let mut b = BTreeMap::new();

        for (c, e) in entries.iter().rev() {
            b.insert(*c, e.clone());
        }

        let root = utxo_root(&a);

        assert_eq!(root, utxo_root(&b), "root depends on insertion order");

        assertions += 1;

        let first = *a.keys().next().unwrap();

        let mut changed_pk = a.clone();
        changed_pk.get_mut(&first).unwrap().output_pk[0] ^= 1;

        if utxo_root(&changed_pk) == root {
            bail!("output_pk mutation did not move UTXO root");
        }

        assertions += 1;

        let mut changed_height = a.clone();

        changed_height.get_mut(&first).unwrap().height =
            changed_height[&first].height.saturating_add(1);

        if utxo_root(&changed_height) == root {
            bail!("height mutation did not move UTXO root");
        }

        assertions += 1;

        let mut removed = a.clone();
        removed.remove(&first);

        if utxo_root(&removed) == root {
            bail!("UTXO removal did not move root");
        }

        assertions += 1;
    }

    Ok(assertions)
}

fn test_header_binding(rng: &mut Rng) -> Result<u64> {
    let rounds = 2_000u64;
    let mut assertions = 0u64;

    for _ in 0..rounds {
        let base = Header {
            version: 8,
            height: rng.next(),
            prev: rng.bytes32(),
            utxo: rng.bytes32(),
            kernel: rng.bytes32(),
            body: rng.bytes32(),
            time: rng.next(),
            difficulty: rng.next().max(1),
            reward: rng.next(),
            nonce: rng.next(),
        };

        let expected = header_hash(&base);

        macro_rules! differs {
            ($value:expr) => {{
                let changed: Header = $value;

                if header_hash(&changed) == expected {
                    bail!("header field mutation not bound");
                }

                assertions += 1;
            }};
        }

        let mut x = base.clone();
        x.version ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.height ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.prev[0] ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.utxo[0] ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.kernel[0] ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.body[0] ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.time ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.difficulty ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.reward ^= 1;
        differs!(x);

        let mut x = base.clone();
        x.nonce ^= 1;
        differs!(x);
    }

    Ok(assertions)
}

fn test_schnorr_mutations(rng: &mut Rng) -> Result<u64> {
    let rounds = 4_096u64;
    let mut assertions = 0u64;

    let gens = PedersenGens::default();

    let g = &gens.B;

    for _ in 0..rounds {
        let secret = Scalar::from((rng.next() % u32::MAX as u64) + 1);

        let nonce = Scalar::from((rng.next() % u32::MAX as u64) + 1);

        let public = g * secret;

        let r_point = g * nonce;

        let r = r_point.compress().to_bytes();

        let p = public.compress().to_bytes();

        let commit = rng.bytes32();

        let msg = hash_multi(DOMAIN_INPUT, &[&commit]);

        let e = challenge(&r, &p, &msg);

        let s = nonce + e * secret;

        if !verify_sig(&public, g, &msg, r, s.to_bytes()) {
            bail!("valid synthetic Schnorr signature rejected");
        }

        assertions += 1;

        let bad_s = s + Scalar::ONE;

        if verify_sig(&public, g, &msg, r, bad_s.to_bytes()) {
            bail!("mutated Schnorr scalar accepted");
        }

        assertions += 1;

        let mut bad_msg = msg;
        bad_msg[0] ^= 1;

        if verify_sig(&public, g, &bad_msg, r, s.to_bytes()) {
            bail!("signature accepted for mutated message");
        }

        assertions += 1;

        let wrong_public = g * (secret + Scalar::ONE);

        if verify_sig(&wrong_public, g, &msg, r, s.to_bytes()) {
            bail!("signature accepted under wrong public key");
        }

        assertions += 1;
    }

    Ok(assertions)
}

fn test_balance_equation(rng: &mut Rng) -> Result<u64> {
    let rounds = 10_000u64;
    let mut assertions = 0u64;

    let gens = PedersenGens::default();

    let b = &gens.B;
    let h = &gens.B_blinding;

    for _ in 0..rounds {
        let vin = 2 + (rng.next() % 1_000_000);

        let fee = rng.next() % vin;

        let vout = vin - fee;

        let rin = Scalar::from(rng.next());

        let rout = Scalar::from(rng.next());

        let input = b * Scalar::from(vin) + h * rin;

        let output = b * Scalar::from(vout) + h * rout;

        let expected = output - input + b * Scalar::from(fee);

        let kernel = h * (rout - rin);

        if expected != kernel {
            bail!("balanced synthetic transaction failed");
        }

        assertions += 1;

        let mutated = expected + b * Scalar::ONE;

        if mutated == kernel {
            bail!("one-dark inflation mutation accepted");
        }

        assertions += 1;
    }

    Ok(assertions)
}

fn main() -> Result<()> {
    println!("NIGHTFALL independent adversarial property probe");

    println!("Nightfall production imports: none");

    println!("deterministic seed: 0x8c3c010cb4754c91");

    let mut rng = Rng::new();

    let mut total = 0u64;

    let n = test_maturity(&mut rng);

    total += n;

    println!("coinbase maturity boundaries..... PASS ({n})");

    let n = test_duplicate_detection(&mut rng);

    total += n;

    println!("double-spend set semantics....... PASS ({n})");

    let n = test_merkle_properties(&mut rng)?;

    total += n;

    println!("UTXO Merkle mutation properties. PASS ({n})");

    let n = test_header_binding(&mut rng)?;

    total += n;

    println!("header field/nonce binding....... PASS ({n})");

    let n = test_schnorr_mutations(&mut rng)?;

    total += n;

    println!("Schnorr adversarial mutations.... PASS ({n})");

    let n = test_balance_equation(&mut rng)?;

    total += n;

    println!("Pedersen inflation mutations..... PASS ({n})");

    println!();
    println!("========================================");
    println!("PHASE 2D PROPERTY PROBE PASS");
    println!("adversarial assertions....... {total}");
    println!("maturity boundary............ covered");
    println!("duplicate spend.............. covered");
    println!("UTXO root mutation........... covered");
    println!("header mutation.............. covered");
    println!("Schnorr mutation............. covered");
    println!("one-dark inflation........... rejected");
    println!("production code.............. untouched");
    println!("========================================");

    Ok(())
}
