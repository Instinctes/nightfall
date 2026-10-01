use rand::{rngs::OsRng, Rng};
use std::collections::HashMap;

const ORIGIN_ROUTE_KEY: &str = "\0nightfall-origin";

/// Dandelion++ uses a small privacy subgraph rather than choosing a fresh
/// random relay for every transaction. Two outbound destinations provide
/// redundancy without creating a large fingerprint surface.
pub const DANDELION_DESTINATIONS: usize = 2;

/// Randomised epoch around the ten-minute design point.
/// The routing graph is rebuilt only at an epoch boundary or if every chosen
/// destination disappears.
pub const DANDELION_EPOCH_MIN_SECS: u64 = 8 * 60;
pub const DANDELION_EPOCH_MAX_SECS: u64 = 12 * 60;

/// A relay node remains in stem mode for an epoch with 90% probability.
/// The decision is epoch-scoped, not independently redrawn per transaction.
pub const DANDELION_STEM_NUMERATOR: u32 = 9;
pub const DANDELION_STEM_DENOMINATOR: u32 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayMode {
    Stem,
    Fluff,
}

/// Epoch-scoped Dandelion++ privacy graph.
///
/// Important invariants:
/// - at most two outbound Dandelion destinations;
/// - a locally originated transaction keeps one route for the epoch;
/// - a given inbound edge keeps one route for the epoch;
/// - routing never deliberately returns a transaction to its source;
/// - route assignments are balanced across the selected destinations;
/// - peer failure repairs a route without requiring a node restart.
#[derive(Clone, Debug)]
pub struct DandelionRouter {
    epoch: u64,
    epoch_ends_at: u64,
    destinations: Vec<String>,
    routes: HashMap<String, String>,
    mode: RelayMode,
}

impl Default for DandelionRouter {
    fn default() -> Self {
        Self {
            epoch: 0,
            epoch_ends_at: 0,
            destinations: Vec::new(),
            routes: HashMap::new(),
            mode: RelayMode::Stem,
        }
    }
}

impl DandelionRouter {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn epoch_ends_at(&self) -> u64 {
        self.epoch_ends_at
    }

    pub fn mode(&self) -> RelayMode {
        self.mode
    }

    pub fn destinations(&self) -> &[String] {
        &self.destinations
    }

    /// Rebuild the privacy graph only when its epoch expires or every selected
    /// destination vanished. Ordinary peer-list ordering cannot perturb it.
    pub fn refresh(&mut self, now: u64, outbound: &[String]) {
        let no_live_destination = !outbound.is_empty()
            && (self.destinations.is_empty()
                || self
                    .destinations
                    .iter()
                    .all(|d| !contains_peer(outbound, d)));

        if self.epoch_ends_at == 0 || now >= self.epoch_ends_at || no_live_destination {
            self.start_epoch(now, outbound);
        }
    }

    /// Return the stable stem destination for one incoming edge.
    ///
    /// `source == None` represents a transaction originated by this node.
    pub fn route_for(
        &mut self,
        now: u64,
        source: Option<&str>,
        outbound: &[String],
    ) -> Option<String> {
        self.refresh(now, outbound);

        let route_key = source.unwrap_or(ORIGIN_ROUTE_KEY);

        if let Some(existing) = self.routes.get(route_key) {
            if source != Some(existing.as_str()) && contains_peer(outbound, existing) {
                return Some(existing.clone());
            }
        }

        let mut candidates = self
            .destinations
            .iter()
            .filter(|peer| source != Some(peer.as_str()) && contains_peer(outbound, peer))
            .cloned()
            .collect::<Vec<_>>();

        // Peer churn can temporarily remove every selected destination.
        // Availability wins here; the next epoch rebuilds the privacy graph.
        if candidates.is_empty() {
            candidates = canonical_peers(outbound)
                .into_iter()
                .filter(|peer| source != Some(peer.as_str()))
                .collect();
        }

        let chosen = if source.is_none() {
            choose_uniform(&candidates)?
        } else {
            self.choose_least_loaded(&candidates)?
        };

        self.routes.insert(route_key.to_owned(), chosen.clone());
        Some(chosen)
    }

    /// BIP156-style balancing: map a new inbound edge to one of the least
    /// loaded Dandelion destinations, then keep that mapping for the epoch.
    fn choose_least_loaded(&self, candidates: &[String]) -> Option<String> {
        let min_load = candidates
            .iter()
            .map(|candidate| {
                self.routes
                    .values()
                    .filter(|route| *route == candidate)
                    .count()
            })
            .min()?;

        let least_loaded = candidates
            .iter()
            .filter(|candidate| {
                self.routes
                    .values()
                    .filter(|route| *route == *candidate)
                    .count()
                    == min_load
            })
            .cloned()
            .collect::<Vec<_>>();

        choose_uniform(&least_loaded)
    }

    fn start_epoch(&mut self, now: u64, outbound: &[String]) {
        let mut candidates = canonical_peers(outbound);
        let mut rng = OsRng;

        let mut destinations = Vec::with_capacity(DANDELION_DESTINATIONS.min(candidates.len()));

        while destinations.len() < DANDELION_DESTINATIONS && !candidates.is_empty() {
            let idx = rng.gen_range(0..candidates.len());
            destinations.push(candidates.swap_remove(idx));
        }

        self.epoch = self.epoch.saturating_add(1);

        self.epoch_ends_at =
            now.saturating_add(rng.gen_range(DANDELION_EPOCH_MIN_SECS..=DANDELION_EPOCH_MAX_SECS));

        self.destinations = destinations;
        self.routes.clear();

        self.mode = if rng.gen_ratio(DANDELION_STEM_NUMERATOR, DANDELION_STEM_DENOMINATOR) {
            RelayMode::Stem
        } else {
            RelayMode::Fluff
        };
    }
}

fn canonical_peers(peers: &[String]) -> Vec<String> {
    let mut peers = peers.to_vec();
    peers.sort();
    peers.dedup();
    peers
}

fn contains_peer(peers: &[String], needle: &str) -> bool {
    peers.iter().any(|peer| peer == needle)
}

fn choose_uniform(candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }

    let mut rng = OsRng;
    Some(candidates[rng.gen_range(0..candidates.len())].clone())
}

/// A stem transaction must not enter the ordinary mining mempool merely
/// because this node saw it during the anonymity phase.
#[derive(Clone, Debug)]
pub struct StemEntry<T> {
    pub value: T,
    /// Session the stem arrived from. `None` means locally originated.
    pub source: Option<String>,
    pub route: String,
    pub embargo_at: u64,
    /// A failed stem gets at most one alternate route before the embargo
    /// becomes the final availability mechanism.
    pub repair_attempted: bool,
}

/// Hard memory bound for untrusted network input.
///
/// A remote peer must not be able to turn the privacy pool into an
/// unbounded allocation. Keeping the same cardinality ceiling as the
/// ordinary mempool makes the failure mode explicit and predictable.
pub const STEMPOOL_MAX_ENTRIES: usize = 10_000;

/// Remote traffic may consume at most 75% of the privacy pool.
///
/// This reservation is independent of peer identity, so rotating or
/// fabricating logical source identities cannot consume the local reserve.
/// Local origins may use the complete global pool.
pub const STEMPOOL_REMOTE_MAX_ENTRIES: usize = STEMPOOL_MAX_ENTRIES * 3 / 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StemInsert {
    Inserted,
    Duplicate,
    Full,
}

/// Separate Dandelion++ stempool.
///
/// This is deliberately generic so the state-machine tests do not need to
/// construct cryptographic transactions. Runtime integration will instantiate
/// it as `StemPool<Transaction>`.
#[derive(Clone, Debug)]
pub struct StemPool<T> {
    entries: HashMap<String, StemEntry<T>>,
    remote_total: usize,
}

impl<T> Default for StemPool<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            remote_total: 0,
        }
    }
}

impl<T> StemPool<T> {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn remote_len(&self) -> usize {
        self.remote_total
    }

    pub fn can_accept_remote(&self) -> bool {
        self.entries.len() < STEMPOOL_MAX_ENTRIES && self.remote_total < STEMPOOL_REMOTE_MAX_ENTRIES
    }

    fn account_remote_remove(&mut self, source: Option<&str>) {
        if source.is_none() {
            return;
        }

        debug_assert!(
            self.remote_total > 0,
            "remote stempool accounting underflow"
        );

        self.remote_total = self.remote_total.saturating_sub(1);
    }

    fn remove_entry(&mut self, txid: &str) -> Option<StemEntry<T>> {
        let entry = self.entries.remove(txid)?;

        self.account_remote_remove(entry.source.as_deref());

        Some(entry)
    }

    pub fn contains(&self, txid: &str) -> bool {
        self.entries.contains_key(txid)
    }

    pub fn get(&self, txid: &str) -> Option<&StemEntry<T>> {
        self.entries.get(txid)
    }

    /// Insert a previously unseen stem without overwriting an existing one.
    ///
    /// Duplicate detection matters for loop handling: silently replacing the
    /// previous entry would erase the evidence that a stem came back around.
    pub fn insert_new(
        &mut self,
        txid: String,
        value: T,
        route: String,
        embargo_at: u64,
    ) -> StemInsert {
        self.insert_new_from(txid, value, None, route, embargo_at)
    }
    pub fn insert_new_from(
        &mut self,
        txid: String,
        value: T,
        source: Option<String>,
        route: String,
        embargo_at: u64,
    ) -> StemInsert {
        if self.entries.contains_key(&txid) {
            return StemInsert::Duplicate;
        }

        if source.is_some() && !self.can_accept_remote() {
            return StemInsert::Full;
        }

        if self.entries.len() >= STEMPOOL_MAX_ENTRIES {
            return StemInsert::Full;
        }

        let is_remote = source.is_some();

        self.entries.insert(
            txid,
            StemEntry {
                value,
                source,
                route,
                embargo_at,
                repair_attempted: false,
            },
        );

        if is_remote {
            self.remote_total = self.remote_total.saturating_add(1);
        }

        StemInsert::Inserted
    }

    /// Admit a locally originated stem without allowing remote traffic to
    /// consume all anonymity capacity.
    ///
    /// The hard memory ceiling remains unchanged. When the pool is exactly
    /// full, a local origin may replace one remote stem (`source.is_some()`).
    /// A remote stem may never evict another entry, and a local stem may never
    /// evict another local stem.
    ///
    /// If several remote entries are eligible, evict the one whose local
    /// fail-safe embargo expires first. The txid is the deterministic
    /// tie-breaker. Its previous stem hop still owns its own embargo, so
    /// removing our local tracking does not remove the network-level
    /// availability failsafe.
    pub fn insert_local_with_remote_preemption(
        &mut self,
        txid: String,
        value: T,
        route: String,
        embargo_at: u64,
    ) -> StemInsert {
        if self.entries.contains_key(&txid) {
            return StemInsert::Duplicate;
        }

        if self.entries.len() < STEMPOOL_MAX_ENTRIES {
            return self.insert_new(txid, value, route, embargo_at);
        }

        // The invariant says the pool cannot exceed the hard cap. Do not
        // mutate unexpected over-cap state in an attempt to repair it here.
        if self.entries.len() > STEMPOOL_MAX_ENTRIES {
            return StemInsert::Full;
        }

        let victim = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.source.is_some())
            .min_by(|(txid_a, entry_a), (txid_b, entry_b)| {
                entry_a
                    .embargo_at
                    .cmp(&entry_b.embargo_at)
                    .then_with(|| txid_a.cmp(txid_b))
            })
            .map(|(txid, _)| txid.clone());

        let Some(victim) = victim else {
            // Every slot is local-origin. Remote traffic did not cause the
            // exhaustion, so local entries are never sacrificed.
            return StemInsert::Full;
        };

        let removed = self.remove(&victim);

        debug_assert!(
            removed.is_some(),
            "selected remote preemption victim disappeared"
        );

        self.insert_new(txid, value, route, embargo_at)
    }

    pub fn repair_route_once(
        &mut self,
        txid: &str,
        failed_route: &str,
        new_route: String,
    ) -> Option<T>
    where
        T: Clone,
    {
        let entry = self.entries.get_mut(txid)?;

        if entry.route != failed_route || entry.repair_attempted {
            return None;
        }

        entry.route = new_route;
        entry.repair_attempted = true;

        Some(entry.value.clone())
    }

    /// Remove entries whose payload satisfies `predicate`.
    ///
    /// Runtime uses this when an accepted block consumes an input or creates
    /// an output belonging to a stem transaction. Aggregation destroys the
    /// original txid on-chain, so cleanup must use transaction contents.
    pub fn drop_where<F>(&mut self, mut predicate: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let doomed = self
            .entries
            .iter()
            .filter(|(_, entry)| predicate(&entry.value))
            .map(|(txid, _)| txid.clone())
            .collect::<Vec<_>>();

        let removed = doomed.len();

        for txid in doomed {
            let _ = self.remove(&txid);
        }

        removed
    }

    /// Used when a normal/fluff copy is observed: the stem copy is cancelled
    /// instead of being allowed to fluff again after its old embargo expires.
    pub fn remove(&mut self, txid: &str) -> Option<StemEntry<T>> {
        self.remove_entry(txid)
    }

    /// Atomically remove every stem whose fail-safe embargo has expired.
    /// The caller promotes these entries into the public mempool and fluffs.
    pub fn take_due(&mut self, now: u64) -> Vec<StemEntry<T>> {
        let due = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.embargo_at <= now)
            .map(|(txid, _)| txid.clone())
            .collect::<Vec<_>>();

        due.into_iter()
            .filter_map(|txid| self.remove(&txid))
            .collect()
    }
}

/// Repair a genuinely failed stem write without destroying the privacy phase.
///
/// A transaction gets one alternate edge. The failed edge and the original
/// source are excluded. Further failures are left to the randomized embargo;
/// repeated rapid rerouting would itself create a useful timing fingerprint.
pub fn repair_failed_stem<T: Clone>(
    router: &mut DandelionRouter,
    pool: &mut StemPool<T>,
    txid: &str,
    failed_route: &str,
    now: u64,
    outbound: &[String],
) -> Option<(String, T)> {
    let entry = pool.get(txid)?;

    if entry.route != failed_route || entry.repair_attempted {
        return None;
    }

    let source = entry.source.clone();

    let available: Vec<String> = outbound
        .iter()
        .filter(|route| route.as_str() != failed_route)
        .cloned()
        .collect();

    if available.is_empty() {
        return None;
    }

    router.refresh(now, &available);

    let new_route = router.route_for(now, source.as_deref(), &available)?;

    if new_route == failed_route || source.as_deref() == Some(new_route.as_str()) {
        return None;
    }

    let value = pool.repair_route_once(txid, failed_route, new_route.clone())?;

    Some((new_route, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peers() -> Vec<String> {
        vec![
            "out:10.0.0.1:17891".into(),
            "out:10.0.0.2:17891".into(),
            "out:10.0.0.3:17891".into(),
            "out:10.0.0.4:17891".into(),
        ]
    }

    #[test]
    fn origin_route_is_stable_inside_an_epoch() {
        let mut router = DandelionRouter::default();
        let peers = peers();

        let first = router.route_for(1_000, None, &peers).unwrap();
        let end = router.epoch_ends_at();

        assert!(end >= 1_000 + DANDELION_EPOCH_MIN_SECS);
        assert_eq!(router.route_for(end - 1, None, &peers).unwrap(), first);
    }

    #[test]
    fn inbound_route_is_stable_and_never_returns_to_source() {
        let mut router = DandelionRouter::default();
        let peers = peers();
        let source = peers[0].as_str();

        let first = router.route_for(2_000, Some(source), &peers).unwrap();

        assert_ne!(first, source);
        assert_eq!(
            router.route_for(2_001, Some(source), &peers).unwrap(),
            first
        );
    }

    #[test]
    fn route_repairs_when_its_destination_disappears() {
        let mut router = DandelionRouter::default();
        let peers = peers();

        let first = router
            .route_for(3_000, Some("in:198.51.100.7:50000"), &peers)
            .unwrap();

        let remaining = peers
            .into_iter()
            .filter(|peer| peer != &first)
            .collect::<Vec<_>>();

        let repaired = router
            .route_for(3_001, Some("in:198.51.100.7:50000"), &remaining)
            .unwrap();

        assert_ne!(repaired, first);
        assert!(remaining.contains(&repaired));
    }

    #[test]
    fn epoch_rollover_rebuilds_privacy_graph_state() {
        let mut router = DandelionRouter::default();
        let peers = peers();

        let _ = router.route_for(4_000, None, &peers).unwrap();

        let epoch = router.epoch();
        let end = router.epoch_ends_at();

        router.refresh(end, &peers);

        assert_eq!(router.epoch(), epoch + 1);
        assert!(router.epoch_ends_at() > end);
    }

    #[test]
    fn at_most_two_dandelion_destinations_are_selected() {
        let mut router = DandelionRouter::default();
        let peers = peers();

        router.refresh(5_000, &peers);

        assert!(!router.destinations().is_empty());
        assert!(router.destinations().len() <= DANDELION_DESTINATIONS);

        let mut unique = router.destinations().to_vec();
        unique.sort();
        unique.dedup();

        assert_eq!(unique.len(), router.destinations().len());
    }

    #[test]
    fn stem_pool_releases_only_after_embargo() {
        let mut pool = StemPool::default();

        assert_eq!(
            pool.insert_new("tx-a".into(), 7u64, "out:a".into(), 100,),
            StemInsert::Inserted
        );

        assert!(pool.take_due(99).is_empty());

        let due = pool.take_due(100);

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].value, 7);
        assert!(pool.is_empty());
    }

    #[test]
    fn seeing_fluff_can_cancel_a_stem_entry() {
        let mut pool = StemPool::default();

        pool.insert_new("tx-b".into(), "payload", "out:b".into(), 200);

        let removed = pool.remove("tx-b").unwrap();

        assert_eq!(removed.value, "payload");
        assert!(!pool.contains("tx-b"));
    }

    #[test]
    fn duplicate_stem_does_not_replace_original_entry() {
        let mut pool = StemPool::default();

        assert_eq!(
            pool.insert_new("tx-dup".into(), 1u64, "out:first".into(), 100,),
            StemInsert::Inserted
        );

        assert_eq!(
            pool.insert_new("tx-dup".into(), 2u64, "out:second".into(), 200,),
            StemInsert::Duplicate
        );

        let kept = pool.get("tx-dup").unwrap();

        assert_eq!(kept.value, 1);
        assert_eq!(kept.route, "out:first");
        assert_eq!(kept.embargo_at, 100);
    }

    #[test]
    fn stempool_has_a_hard_memory_bound() {
        let mut pool = StemPool::default();

        for i in 0..STEMPOOL_MAX_ENTRIES {
            assert_eq!(
                pool.insert_new(format!("tx-{i}"), i, "out:a".into(), 100,),
                StemInsert::Inserted
            );
        }

        assert_eq!(pool.len(), STEMPOOL_MAX_ENTRIES);

        assert_eq!(
            pool.insert_new("one-too-many".into(), usize::MAX, "out:b".into(), 100,),
            StemInsert::Full
        );

        assert_eq!(pool.len(), STEMPOOL_MAX_ENTRIES);
    }

    #[test]
    fn drop_where_removes_only_matching_stems() {
        let mut pool = StemPool::default();

        assert_eq!(
            pool.insert_new("a".into(), 1u64, "out:a".into(), 100),
            StemInsert::Inserted
        );
        assert_eq!(
            pool.insert_new("b".into(), 2u64, "out:b".into(), 100),
            StemInsert::Inserted
        );
        assert_eq!(
            pool.insert_new("c".into(), 3u64, "out:c".into(), 100),
            StemInsert::Inserted
        );

        assert_eq!(pool.drop_where(|value| *value % 2 == 0), 1);

        assert!(pool.contains("a"));
        assert!(!pool.contains("b"));
        assert!(pool.contains("c"));
    }

    #[test]
    fn failed_stem_repairs_once_without_echoing_to_source() {
        let mut router = DandelionRouter::default();
        let mut pool = StemPool::default();

        assert_eq!(
            pool.insert_new_from(
                "repair-me".into(),
                42u64,
                Some("out:source".into()),
                "out:dead".into(),
                100,
            ),
            StemInsert::Inserted
        );

        let outbound = vec!["out:source".to_string(), "out:alternate".to_string()];

        let repaired = repair_failed_stem(
            &mut router,
            &mut pool,
            "repair-me",
            "out:dead",
            1,
            &outbound,
        )
        .expect("alternate route");

        assert_eq!(repaired.0, "out:alternate");
        assert_eq!(repaired.1, 42);

        let entry = pool.get("repair-me").unwrap();

        assert_eq!(entry.route, "out:alternate");
        assert!(entry.repair_attempted);
        assert_ne!(entry.source.as_deref(), Some(entry.route.as_str()));

        // One repair only. A second failed hop waits for embargo.
        assert!(repair_failed_stem(
            &mut router,
            &mut pool,
            "repair-me",
            "out:alternate",
            2,
            &["out:third".to_string()],
        )
        .is_none());
    }

    #[test]
    fn local_origin_preempts_remote_when_stempool_is_full() {
        let mut pool = StemPool::default();

        for i in 0..STEMPOOL_REMOTE_MAX_ENTRIES {
            assert_eq!(
                pool.insert_new_from(
                    format!("remote-{i:05}"),
                    i,
                    Some("peer:remote".into()),
                    "out:next-hop".into(),
                    10_000 + i as u64,
                ),
                StemInsert::Inserted
            );
        }

        let local_fill = STEMPOOL_MAX_ENTRIES - STEMPOOL_REMOTE_MAX_ENTRIES;

        for i in 0..local_fill {
            assert_eq!(
                pool.insert_new(
                    format!("local-fill-{i:05}"),
                    STEMPOOL_REMOTE_MAX_ENTRIES + i,
                    "out:local-hop".into(),
                    50_000 + i as u64,
                ),
                StemInsert::Inserted
            );
        }

        assert_eq!(pool.len(), STEMPOOL_MAX_ENTRIES);

        assert_eq!(pool.remote_len(), STEMPOOL_REMOTE_MAX_ENTRIES);

        assert_eq!(
            pool.insert_local_with_remote_preemption(
                "local-origin".into(),
                usize::MAX,
                "out:local-hop".into(),
                99_999,
            ),
            StemInsert::Inserted
        );

        assert_eq!(pool.len(), STEMPOOL_MAX_ENTRIES);

        assert!(pool.contains("local-origin"));

        assert!(
            !pool.contains("remote-00000"),
            "earliest-expiring remote victim was not preempted"
        );

        assert_eq!(pool.remote_len(), STEMPOOL_REMOTE_MAX_ENTRIES - 1);
    }

    #[test]
    fn remote_global_budget_reserves_local_capacity() {
        let mut pool = StemPool::default();

        for i in 0..STEMPOOL_REMOTE_MAX_ENTRIES {
            assert_eq!(
                pool.insert_new_from(
                    format!("remote-budget-{i:05}"),
                    i,
                    Some(format!("peer-{i}")),
                    "out:next".into(),
                    20_000 + i as u64,
                ),
                StemInsert::Inserted
            );
        }

        assert_eq!(pool.remote_len(), STEMPOOL_REMOTE_MAX_ENTRIES);

        assert_eq!(pool.len(), STEMPOOL_REMOTE_MAX_ENTRIES);

        assert_eq!(
            pool.insert_new_from(
                "remote-over-budget".into(),
                usize::MAX,
                Some("peer-new".into()),
                "out:next".into(),
                99_999,
            ),
            StemInsert::Full
        );

        assert_eq!(
            pool.insert_local_with_remote_preemption(
                "local-reserved".into(),
                usize::MAX - 1,
                "out:local".into(),
                99_999,
            ),
            StemInsert::Inserted
        );

        assert!(pool.contains("local-reserved"));
    }

    #[test]
    fn duplicate_remote_does_not_consume_budget_twice() {
        let mut pool = StemPool::default();

        assert_eq!(
            pool.insert_new_from(
                "duplicate".into(),
                1u64,
                Some("peer:a".into()),
                "out:a".into(),
                100,
            ),
            StemInsert::Inserted
        );

        assert_eq!(pool.remote_len(), 1);

        assert_eq!(
            pool.insert_new_from(
                "duplicate".into(),
                2u64,
                Some("peer:b".into()),
                "out:b".into(),
                200,
            ),
            StemInsert::Duplicate
        );

        assert_eq!(pool.remote_len(), 1);
    }

    #[test]
    fn remote_accounting_tracks_all_removal_paths() {
        let mut pool = StemPool::default();

        assert_eq!(
            pool.insert_new_from(
                "remove".into(),
                1u64,
                Some("peer:a".into()),
                "out:a".into(),
                100,
            ),
            StemInsert::Inserted
        );

        assert_eq!(
            pool.insert_new_from(
                "drop".into(),
                2u64,
                Some("peer:b".into()),
                "out:b".into(),
                200,
            ),
            StemInsert::Inserted
        );

        assert_eq!(
            pool.insert_new_from(
                "due".into(),
                3u64,
                Some("peer:c".into()),
                "out:c".into(),
                10,
            ),
            StemInsert::Inserted
        );

        assert_eq!(pool.remote_len(), 3);

        assert!(pool.remove("remove").is_some());
        assert_eq!(pool.remote_len(), 2);

        assert_eq!(pool.drop_where(|value| *value == 2), 1);

        assert_eq!(pool.remote_len(), 1);

        let due = pool.take_due(10);

        assert_eq!(due.len(), 1);
        assert_eq!(pool.remote_len(), 0);
    }

    #[test]
    fn full_local_stempool_cannot_be_displaced() {
        let mut pool = StemPool::default();

        for i in 0..STEMPOOL_MAX_ENTRIES {
            assert_eq!(
                pool.insert_new(
                    format!("local-{i:05}"),
                    i,
                    "out:local-hop".into(),
                    20_000 + i as u64,
                ),
                StemInsert::Inserted
            );
        }

        assert_eq!(pool.len(), STEMPOOL_MAX_ENTRIES);

        // Remote traffic may never evict local privacy state.
        assert_eq!(
            pool.insert_new_from(
                "remote-overflow".into(),
                usize::MAX - 1,
                Some("peer:remote".into()),
                "out:remote-hop".into(),
                30_000,
            ),
            StemInsert::Full
        );

        // Nor may a new local transaction silently exceed the hard cap by
        // sacrificing an existing local transaction.
        assert_eq!(
            pool.insert_local_with_remote_preemption(
                "local-overflow".into(),
                usize::MAX,
                "out:local-hop".into(),
                30_001,
            ),
            StemInsert::Full
        );

        assert_eq!(pool.len(), STEMPOOL_MAX_ENTRIES);

        assert!(pool.contains("local-00000"));

        assert!(!pool.contains("remote-overflow"));

        assert!(!pool.contains("local-overflow"));
    }
}
