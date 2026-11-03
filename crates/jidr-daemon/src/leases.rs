//! Pool leases (D42, Phase 13d): who holds which throwaway clone, and until
//! when. A lease is tied to the client connection that took it (MCP: the
//! agent's process; 2 minutes of grace after it disconnects) or to a TTL
//! (the CLI's default is 2 hours). The daemon's reaper releases expired ones.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

/// How long a connection lease outlives its connection.
pub const GRACE: Duration = Duration::from_secs(120);
/// How often the reaper looks for expired leases.
pub const REAP_EVERY: Duration = Duration::from_secs(30);

/// What keeps a lease alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Holder {
    /// The client connection that took it (until it closes, plus grace).
    Connection { id: u64 },
    /// A fixed lifetime, renewable.
    Ttl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Lease {
    pub id: Uuid,
    pub base: Uuid,
    pub device: Uuid,
    pub holder: Holder,
    /// When the reaper releases it; `None` while its connection is open.
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct Leases {
    by_id: HashMap<Uuid, Lease>,
}

impl Leases {
    pub fn insert(&mut self, lease: Lease) {
        self.by_id.insert(lease.id, lease);
    }

    pub fn get(&self, id: Uuid) -> Option<&Lease> {
        self.by_id.get(&id)
    }

    /// The lease on `device`, if it is a pool clone.
    pub fn for_device(&self, device: Uuid) -> Option<&Lease> {
        self.by_id.values().find(|l| l.device == device)
    }

    pub fn remove(&mut self, id: Uuid) -> Option<Lease> {
        self.by_id.remove(&id)
    }

    pub fn all(&self) -> Vec<Lease> {
        let mut v: Vec<Lease> = self.by_id.values().cloned().collect();
        v.sort_by_key(|l| l.created_at);
        v
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn of_base(&self, base: Uuid) -> usize {
        self.by_id.values().filter(|l| l.base == base).count()
    }

    /// Connection `conn` closed: its leases expire after the grace period.
    pub fn disconnected(&mut self, conn: u64, now: DateTime<Utc>) {
        for l in self.by_id.values_mut() {
            if l.holder == (Holder::Connection { id: conn }) && l.expires_at.is_none() {
                l.expires_at = Some(now + grace());
            }
        }
    }

    /// Extends a lease. A connection lease whose connection is open has no
    /// expiry to extend; a TTL lease runs `ttl` from now.
    pub fn renew(&mut self, id: Uuid, ttl: Duration, now: DateTime<Utc>) -> Option<Lease> {
        let l = self.by_id.get_mut(&id)?;
        if l.holder == Holder::Ttl || l.expires_at.is_some() {
            l.expires_at = Some(now + chrono::Duration::from_std(ttl).unwrap_or_default());
        }
        Some(l.clone())
    }

    /// Leases due for release at `now`.
    pub fn expired(&self, now: DateTime<Utc>) -> Vec<Lease> {
        self.by_id
            .values()
            .filter(|l| l.expires_at.is_some_and(|t| t <= now))
            .cloned()
            .collect()
    }
}

fn grace() -> chrono::Duration {
    chrono::Duration::from_std(GRACE).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(holder: Holder, expires_at: Option<DateTime<Utc>>) -> Lease {
        Lease {
            id: Uuid::new_v4(),
            base: Uuid::nil(),
            device: Uuid::new_v4(),
            holder,
            expires_at,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn connection_leases_expire_after_grace() {
        let now = Utc::now();
        let mut l = Leases::default();
        let a = lease(Holder::Connection { id: 1 }, None);
        let b = lease(Holder::Connection { id: 2 }, None);
        l.insert(a.clone());
        l.insert(b.clone());
        assert!(
            l.expired(now + chrono::Duration::days(1)).is_empty(),
            "open connections"
        );
        l.disconnected(1, now);
        assert_eq!(l.get(a.id).unwrap().expires_at, Some(now + grace()));
        assert!(
            l.expired(now + chrono::Duration::seconds(119)).is_empty(),
            "in grace"
        );
        let due = l.expired(now + chrono::Duration::seconds(120));
        assert_eq!(due.iter().map(|x| x.id).collect::<Vec<_>>(), vec![a.id]);
        // Renewing during grace extends it; an open connection has nothing to renew.
        l.renew(a.id, Duration::from_secs(600), now).unwrap();
        assert!(l.expired(now + chrono::Duration::seconds(300)).is_empty());
        assert_eq!(
            l.renew(b.id, Duration::from_secs(5), now)
                .unwrap()
                .expires_at,
            None
        );
    }

    #[test]
    fn ttl_leases_expire_and_renew() {
        let now = Utc::now();
        let mut l = Leases::default();
        let t = lease(Holder::Ttl, Some(now + chrono::Duration::seconds(60)));
        l.insert(t.clone());
        assert!(l.expired(now).is_empty());
        assert_eq!(l.expired(now + chrono::Duration::seconds(60)).len(), 1);
        l.renew(
            t.id,
            Duration::from_secs(3600),
            now + chrono::Duration::seconds(50),
        )
        .unwrap();
        assert!(l.expired(now + chrono::Duration::seconds(61)).is_empty());
        l.disconnected(7, now);
        assert_eq!(l.of_base(Uuid::nil()), 1);
        assert_eq!(l.for_device(t.device).unwrap().id, t.id);
        assert!(l.remove(t.id).is_some() && l.is_empty());
    }
}
