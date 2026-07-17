//! Layer B — network presence (macOS).
//!
//! For AI usage that leaves no local transcript (web chats, native apps), the
//! reliable content-free signal is the process → AI-endpoint TCP connection. We
//! read the process table with `libproc` (`proc_pidfdinfo` on each socket fd) —
//! no root, no entitlement, own-user processes only, exactly what `lsof` shows.
//! Each established connection is attributed by process identity or destination
//! range (see `attribution`); unknown traffic resolves to nothing, so Slack or a
//! browser to an unrelated site never registers.
//!
//! Observations are coalesced into presence INTERVALS per (provider, process,
//! surface): a provider seen every poll extends one interval; a gap of
//! `gap_ms` closes it and writes one row. This is a coarse "an AI tool was
//! active" span, distinct from the rich interactions Layer A records — a browser
//! ChatGPT tab shows up here even though its content is never read.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use libproc::file_info::{pidfdinfo, ListFDs, ProcFDType};
use libproc::net_info::{SocketFDInfo, SocketInfoKind};
use libproc::proc_pid::{listpidinfo, pidpath};
use libproc::processes::{pids_by_type, ProcFilter};

use ai_usage_monitor::attribution::{attribute_connection, ProcessInfo, Surface};
use ai_usage_monitor::store::{PresenceRow, Store};

/// Coalesces per-poll connection observations into presence intervals.
pub struct NetPresence {
    open: HashMap<Key, Interval>,
    gap_ms: i64,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    provider: &'static str,
    process: String,
    surface: &'static str,
}

struct Interval {
    started_ms: i64,
    last_ms: i64,
    observations: i64,
}

impl NetPresence {
    pub fn new(gap_ms: i64) -> Self {
        Self { open: HashMap::new(), gap_ms }
    }

    /// Poll the process table once, extend/open intervals for what is active,
    /// and close+persist any interval whose provider has gone quiet past the
    /// gap. Returns how many distinct AI providers are active right now (for the
    /// status line).
    pub fn poll(&mut self, store: &Store, now_ms: i64) -> usize {
        let mut active: HashSet<Key> = HashSet::new();
        for (proc, ip) in observe_connections() {
            if let Some(attr) = attribute_connection(&proc, ip) {
                active.insert(Key {
                    provider: attr.provider,
                    process: proc.name,
                    surface: attr.surface.as_str(),
                });
            }
        }
        self.apply(store, active, now_ms)
    }

    /// Pure coalescing step over an already-observed active set (split out so it
    /// is testable without touching the real process table).
    fn apply(&mut self, store: &Store, active: HashSet<Key>, now_ms: i64) -> usize {
        for key in &active {
            let iv = self
                .open
                .entry(key.clone())
                .or_insert(Interval { started_ms: now_ms, last_ms: now_ms, observations: 0 });
            iv.last_ms = now_ms;
            iv.observations += 1;
        }

        let stale: Vec<Key> = self
            .open
            .iter()
            .filter(|(k, iv)| !active.contains(*k) && now_ms - iv.last_ms > self.gap_ms)
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale {
            if let Some(iv) = self.open.remove(&key) {
                write_interval(store, &key, &iv);
            }
        }

        // Distinct providers (not process/surface rows) currently active.
        active.iter().map(|k| k.provider).collect::<HashSet<_>>().len()
    }

    /// Persist every still-open interval (on shutdown) so nothing is lost.
    pub fn flush_open(&mut self, store: &Store, now_ms: i64) {
        for (key, mut iv) in self.open.drain().collect::<Vec<_>>() {
            iv.last_ms = iv.last_ms.max(now_ms);
            write_interval(store, &key, &iv);
        }
    }
}

fn write_interval(store: &Store, key: &Key, iv: &Interval) {
    let row = PresenceRow {
        provider: key.provider.to_string(),
        process: key.process.clone(),
        surface: key.surface.to_string(),
        started_at_ms: iv.started_ms,
        ended_at_ms: iv.last_ms,
        observations: iv.observations,
    };
    if let Err(e) = store.insert_presence(&row) {
        log::warn!("presence insert failed: {e}");
    }
}

/// One-shot snapshot of AI-attributed live connections, for `--diagnose`:
/// `(process, remote-ip, provider, surface)`. Same observation path as `poll`,
/// without the interval bookkeeping.
pub fn snapshot() -> Vec<(String, IpAddr, &'static str, Surface)> {
    observe_connections()
        .into_iter()
        .filter_map(|(proc, ip)| {
            attribute_connection(&proc, ip).map(|a| (proc.name, ip, a.provider, a.surface))
        })
        .collect()
}

/// Enumerate every own-user process's established TCP connections as
/// `(process, remote-ip)`. Processes we can't read (system/other-user) are
/// skipped silently — same visibility ceiling as `lsof` without sudo.
fn observe_connections() -> Vec<(ProcessInfo, IpAddr)> {
    let mut out = Vec::new();
    let pids = pids_by_type(ProcFilter::All).unwrap_or_default();
    for pid in pids {
        if pid == 0 {
            continue;
        }
        let pid = pid as i32;
        let Ok(path) = pidpath(pid) else { continue };
        let (name, in_app_bundle) = identity(&path);
        let Ok(fds) = listpidinfo::<ListFDs>(pid, 4096) else { continue };
        for fd in fds {
            if !matches!(ProcFDType::from(fd.proc_fdtype), ProcFDType::Socket) {
                continue;
            }
            let Ok(sock) = pidfdinfo::<SocketFDInfo>(pid, fd.proc_fd) else { continue };
            if let Some(ip) = tcp_foreign_ip(&sock) {
                out.push((ProcessInfo { name: name.clone(), in_app_bundle }, ip));
            }
        }
    }
    out
}

/// The foreign (remote) IP of a connected TCP socket, or `None` for anything
/// that isn't an outbound TCP connection with a peer.
fn tcp_foreign_ip(sock: &SocketFDInfo) -> Option<IpAddr> {
    let si = &sock.psi;
    if si.soi_kind != SocketInfoKind::Tcp as i32 {
        return None;
    }
    // SAFETY: soi_kind == Tcp selects the pri_tcp arm of the proto union; the
    // address arm of insi_faddr is then selected by insi_vflag below.
    let ini = unsafe { si.soi_proto.pri_tcp }.tcpsi_ini;
    if ini.insi_fport == 0 {
        return None; // listening / unconnected
    }
    const INI_IPV4: u8 = 0x1;
    if ini.insi_vflag & INI_IPV4 != 0 {
        let raw = unsafe { ini.insi_faddr.ina_46.i46a_addr4.s_addr };
        let ip = Ipv4Addr::from(u32::from_be(raw));
        (!ip.is_unspecified()).then_some(IpAddr::V4(ip))
    } else {
        let bytes = unsafe { ini.insi_faddr.ina_6.s6_addr };
        let ip = Ipv6Addr::from(bytes);
        (!ip.is_unspecified()).then_some(IpAddr::V6(ip))
    }
}

/// Derive a stable identity for a process from its executable path. For a
/// bundled app we use the `.app` bundle name (so a helper process doing the
/// networking still reads as the app); otherwise the executable's basename.
fn identity(path: &str) -> (String, bool) {
    if let Some(app) = bundle_name(path) {
        (app, true)
    } else {
        let leaf = path.rsplit('/').next().unwrap_or(path).to_string();
        (leaf, false)
    }
}

/// `.../<Name>.app/...` → `Name`.
fn bundle_name(path: &str) -> Option<String> {
    let idx = path.find(".app/")?;
    let name = path[..idx].rsplit('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_reads_bundle_and_cli() {
        assert_eq!(
            identity("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome Helper"),
            ("Google Chrome".to_string(), true)
        );
        assert_eq!(identity("/Users/me/.bun/bin/claude"), ("claude".to_string(), false));
        assert_eq!(
            identity("/Applications/Claude.app/Contents/MacOS/Claude"),
            ("Claude".to_string(), true)
        );
    }

    #[test]
    fn interval_coalesces_and_closes_after_gap() {
        let store = Store::open_in_memory().unwrap();
        let mut np = NetPresence::new(60_000);
        let key = Key { provider: "openai", process: "codex".into(), surface: "cli" };
        // An open interval, then applied ticks with NO active connections (the
        // pure step, so it doesn't observe the real machine).
        np.open.insert(key.clone(), Interval { started_ms: 1000, last_ms: 1000, observations: 1 });
        // Within the gap → stays open.
        assert_eq!(np.apply(&store, HashSet::new(), 20_000), 0);
        assert_eq!(store.pending_presence().unwrap().len(), 0, "still open within gap");
        // Past the gap → closes and writes.
        assert_eq!(np.apply(&store, HashSet::new(), 120_000), 0);
        let rows = store.pending_presence().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.provider, "openai");
        assert_eq!(rows[0].row.started_at_ms, 1000);
    }
}
