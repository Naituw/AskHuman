//! Monthly, single-writer traffic counters replicated through paired phones.

use serde::{Deserialize, Serialize};

use super::store::Traffic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficRecord {
    #[serde(default)]
    pub device_kind: DeviceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_from: Option<i64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub uncertain: bool,
    pub broker: String,
    pub mac_id: String,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_name: Option<String>,
    #[serde(flatten)]
    pub counters: Traffic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    #[default]
    Mac,
    Phone,
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn broker_id(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// A record represents one device's cumulative counters, including when relayed by another peer.
pub fn merge(
    records: &mut Vec<TrafficRecord>,
    incoming: &[TrafficRecord],
    broker: &str,
    month: &str,
) -> bool {
    let mut changed = false;
    for record in incoming {
        if record.broker != broker
            || record.counters.month != month
            || !super::crypto::is_valid_id(&record.mac_id)
            || record.updated_at < 0
            || record.recorded_from.is_some_and(|from| from < 0)
        {
            continue;
        }
        if let Some(saved) = records.iter_mut().find(|saved| {
            saved.broker == broker
                && saved.counters.month == month
                && saved.mac_id == record.mac_id
                && saved.device_kind == record.device_kind
        }) {
            let before = saved.clone();
            saved.counters.bytes_in = saved.counters.bytes_in.max(record.counters.bytes_in);
            saved.counters.bytes_out = saved.counters.bytes_out.max(record.counters.bytes_out);
            saved.counters.msgs_in = saved.counters.msgs_in.max(record.counters.msgs_in);
            saved.counters.msgs_out = saved.counters.msgs_out.max(record.counters.msgs_out);
            if record.updated_at >= saved.updated_at && record.mac_name.is_some() {
                saved.mac_name = record.mac_name.clone();
            }
            saved.updated_at = saved.updated_at.max(record.updated_at);
            saved.recorded_from = match (saved.recorded_from, record.recorded_from) {
                (Some(a), Some(b)) => Some(a.min(b)),
                _ => None,
            };
            saved.uncertain |= record.uncertain;
            changed |= *saved != before;
        } else {
            records.push(record.clone());
            changed = true;
        }
    }
    records.sort_by(|a, b| {
        (&a.broker, &a.counters.month, &a.device_kind, &a.mac_id).cmp(&(
            &b.broker,
            &b.counters.month,
            &b.device_kind,
            &b.mac_id,
        ))
    });
    changed
}

pub fn month_start(month: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(&format!("{month}-01T00:00:00Z"))
        .ok()
        .map(|date| date.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, received: u64, sent: u64) -> TrafficRecord {
        TrafficRecord {
            device_kind: DeviceKind::Mac,
            recorded_from: None,
            uncertain: false,
            broker: "broker.example".into(),
            mac_id: id.into(),
            updated_at: 1,
            mac_name: None,
            counters: Traffic {
                month: "2026-11".into(),
                bytes_in: received,
                bytes_out: sent,
                ..Traffic::default()
            },
        }
    }

    #[test]
    fn merges_components_without_recounting_replays_or_other_instances() {
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaa";
        let b = "bbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut records = vec![record(a, 120, 30)];
        let incoming = vec![record(a, 100, 40), record(b, 50, 60)];
        assert!(merge(&mut records, &incoming, "broker.example", "2026-11"));
        assert_eq!(
            (records[0].counters.bytes_in, records[0].counters.bytes_out),
            (120, 40)
        );
        assert_eq!(records.len(), 2);
        assert!(!merge(&mut records, &incoming, "broker.example", "2026-11"));
        assert!(!merge(
            &mut records,
            &incoming,
            "another.example",
            "2026-11"
        ));
        assert!(!merge(&mut records, &incoming, "broker.example", "2026-12"));
    }
}
