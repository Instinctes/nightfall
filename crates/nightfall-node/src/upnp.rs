//! Optional UPnP port mapping for the P2P port.
//!
//! Most nodes sit behind a home router and accept no connections, so the
//! network leans on a few seeds. Asking the router to forward the P2P port
//! lets an ordinary node become one that others can dial. It is off unless
//! the operator switches it on, and it never runs when the node is set up to
//! keep its traffic behind a proxy: a node that hides its address must not
//! also ask the router to publish it.
//!
//! What it does, and nothing more: find the router on the local network,
//! map the P2P port to this machine for an hour, renew the mapping every
//! half hour, and remove it when the node stops.

use crate::runtime::{SharedState, Shutdown, Workers};
use igd_next::{PortMappingProtocol, SearchOptions};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

const LEASE_SECS: u32 = 3_600;
const RENEW_EVERY: Duration = Duration::from_secs(30 * 60);
const DESCRIPTION: &str = "NIGHTFALLCOIN node";

fn set_status(state: &SharedState, status: impl Into<String>) {
    if let Ok(mut g) = state.lock() {
        g.set_upnp_status(Some(status.into()));
    }
}

/// The address of this machine on the router's network.
fn local_ip_towards(gateway: SocketAddr) -> Option<IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(gateway).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Map `port` and keep it mapped until shutdown.
pub(crate) fn spawn(state: SharedState, port: u16, shutdown: Arc<Shutdown>, workers: Arc<Workers>) {
    set_status(&state, "looking for a UPnP router…");
    workers.spawn(move || {
        let mut options = SearchOptions::default();
        options.timeout = Some(Duration::from_secs(5));
        let gateway = match igd_next::search_gateway(options) {
            Ok(g) => g,
            Err(e) => {
                set_status(&state, format!("no UPnP router found ({e})"));
                return;
            }
        };
        let Some(ip) = local_ip_towards(gateway.addr) else {
            set_status(
                &state,
                "UPnP router found, but this machine's address is unknown",
            );
            return;
        };
        let local = SocketAddr::new(ip, port);
        let mut mapped = false;
        loop {
            if shutdown.is_requested() {
                break;
            }
            match gateway.add_port(
                PortMappingProtocol::TCP,
                port,
                local,
                LEASE_SECS,
                DESCRIPTION,
            ) {
                Ok(()) => {
                    mapped = true;
                    let external = gateway
                        .get_external_ip()
                        .map(|e| format!("{e}:{port}"))
                        .unwrap_or_else(|_| format!("port {port}"));
                    set_status(&state, format!("port open on the router ({external})"));
                }
                Err(e) => {
                    set_status(&state, format!("the router refused the port mapping ({e})"));
                }
            }
            if shutdown.wait_timeout(RENEW_EVERY) {
                break;
            }
        }
        if mapped {
            let _ = gateway.remove_port(PortMappingProtocol::TCP, port);
        }
    });
}
