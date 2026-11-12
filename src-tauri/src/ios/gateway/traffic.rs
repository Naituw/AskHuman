use super::*;
use crate::ios::traffic::{broker_id, merge, month_start, DeviceKind, TrafficRecord};

impl Actor {
    pub(super) fn current_traffic(&self) -> Traffic {
        if self.store.state.traffic.month == month_of(self.now()) {
            self.store.state.traffic.clone()
        } else {
            Traffic {
                month: month_of(self.now()),
                ..Traffic::default()
            }
        }
    }

    pub(super) fn traffic_records(&self) -> Vec<TrafficRecord> {
        let broker = broker_id(&self.cfg.host);
        let month = month_of(self.now());
        self.store
            .state
            .traffic_records
            .iter()
            .filter(|record| record.broker == broker && record.counters.month == month)
            .cloned()
            .collect()
    }

    pub(super) fn prepare_traffic(&mut self) {
        let broker = broker_id(&self.cfg.host);
        let month = month_of(self.now());
        let now = self.now();
        self.store
            .state
            .traffic_observed_since
            .entry(broker.clone())
            .or_insert(now);
        // Legacy counters have no broker identity. Their first upgrade binds them to this broker.
        if self
            .store
            .state
            .traffic_broker
            .as_ref()
            .is_some_and(|old| old != &broker)
        {
            self.store.state.traffic = self
                .store
                .state
                .traffic_records
                .iter()
                .find(|record| {
                    record.broker == broker
                        && record.counters.month == month
                        && record.mac_id == self.mac_id
                        && record.device_kind == DeviceKind::Mac
                })
                .map(|record| record.counters.clone())
                .unwrap_or_default();
        }
        let legacy_counters = self.current_traffic();
        let has_legacy_bytes = legacy_counters
            .bytes_in
            .saturating_add(legacy_counters.bytes_out)
            > 0;
        if self.store.state.traffic_broker.is_none() && has_legacy_bytes {
            let legacy = TrafficRecord {
                broker: broker.clone(),
                mac_id: self.mac_id.clone(),
                device_kind: DeviceKind::Mac,
                counters: self.current_traffic(),
                mac_name: Some(self.mac_name()),
                updated_at: self.now(),
                recorded_from: None,
                uncertain: false,
            };
            merge(
                &mut self.store.state.traffic_records,
                std::slice::from_ref(&legacy),
                &broker,
                &month,
            );
        }
        self.store.state.traffic_broker = Some(broker);
        for rt in self.rt.values_mut() {
            rt.traffic_pending = true;
        }
        self.update_traffic_record();
        self.persist_traffic();
    }

    fn update_traffic_record(&mut self) {
        let counters = self.current_traffic();
        self.store.state.traffic = counters.clone();
        let previous = self.store.state.traffic_records.iter().find(|record| {
            record.broker == broker_id(&self.cfg.host)
                && record.counters.month == counters.month
                && record.mac_id == self.mac_id
                && record.device_kind == DeviceKind::Mac
        });
        let recorded_from = previous
            .map(|record| record.recorded_from)
            .unwrap_or_else(|| {
                Some(
                    self.store
                        .state
                        .traffic_observed_since
                        .get(&broker_id(&self.cfg.host))
                        .copied()
                        .unwrap_or(self.traffic_measurement_started)
                        .max(month_start(&counters.month).unwrap_or(self.now())),
                )
            });
        let uncertain = previous.is_some_and(|record| record.uncertain);
        let record = TrafficRecord {
            device_kind: DeviceKind::Mac,
            recorded_from,
            uncertain,
            broker: broker_id(&self.cfg.host),
            mac_id: self.mac_id.clone(),
            updated_at: self.now(),
            mac_name: Some(self.mac_name()),
            counters,
        };
        merge(
            &mut self.store.state.traffic_records,
            std::slice::from_ref(&record),
            &record.broker,
            &record.counters.month,
        );
    }

    fn persist_traffic(&mut self) {
        self.traffic_storage_issue = self.store.save_state_checked().is_err();
        if self.traffic_storage_issue {
            eprintln!("askhuman: could not persist mobile traffic counters");
        }
    }

    pub(super) fn count_traffic(&mut self, inbound: bool, bytes: usize, announce: bool) {
        self.store
            .state
            .count_traffic(&month_of(self.now()), inbound, bytes);
        if announce {
            self.announce_traffic();
        }
        self.persist_traffic();
    }

    pub(super) fn announce_traffic(&mut self) {
        self.update_traffic_record();
        for rt in self.rt.values_mut() {
            rt.traffic_pending = true;
        }
    }

    pub(super) fn uncertain_traffic(&mut self) {
        self.announce_traffic();
        let broker = broker_id(&self.cfg.host);
        let month = month_of(self.now());
        if let Some(record) = self.store.state.traffic_records.iter_mut().find(|record| {
            record.broker == broker
                && record.counters.month == month
                && record.mac_id == self.mac_id
                && record.device_kind == DeviceKind::Mac
        }) {
            record.uncertain = true;
        }
        self.persist_traffic();
    }

    pub(super) fn on_traffic_sync(&mut self, phone_id: &str, sync: wire::TrafficSync) {
        if !self
            .rt
            .get(phone_id)
            .is_some_and(|rt| rt.traffic_sync && rt.hello_at.is_some())
        {
            return;
        }
        let broker = broker_id(&self.cfg.host);
        let month = month_of(self.now());
        for record in &sync.records {
            if record.broker == broker
                && record.counters.month == month
                && crypto::is_valid_id(&record.mac_id)
            {
                self.store.state.traffic_synced_at.insert(
                    format!(
                        "{}|{}|{}|{}",
                        broker,
                        month,
                        if record.device_kind == DeviceKind::Mac {
                            "mac"
                        } else {
                            "phone"
                        },
                        record.mac_id
                    ),
                    self.now(),
                );
            }
        }
        // The owner is the only writer. A restored peer may contribute a higher saved watermark.
        if merge(
            &mut self.store.state.traffic_records,
            &sync.records,
            &broker,
            &month,
        ) {
            if let Some(own) = self.store.state.traffic_records.iter().find(|record| {
                record.broker == broker
                    && record.counters.month == month
                    && record.mac_id == self.mac_id
                    && record.device_kind == DeviceKind::Mac
            }) {
                let current = &mut self.store.state.traffic;
                current.bytes_in = current.bytes_in.max(own.counters.bytes_in);
                current.bytes_out = current.bytes_out.max(own.counters.bytes_out);
                current.msgs_in = current.msgs_in.max(own.counters.msgs_in);
                current.msgs_out = current.msgs_out.max(own.counters.msgs_out);
            }
            for (id, rt) in &mut self.rt {
                if id != phone_id {
                    rt.traffic_pending = true;
                }
            }
        }
        self.persist_traffic();
    }

    pub(super) fn send_traffic(&mut self, phone_id: &str) {
        if !self
            .rt
            .get(phone_id)
            .is_some_and(|rt| rt.traffic_sync && rt.hello_at.is_some())
        {
            return;
        }
        let records = self.traffic_records();
        for batch in records.chunks(128) {
            self.send(
                phone_id,
                &wire::TrafficSync {
                    records: batch.to_vec(),
                },
            );
        }
        let now = self.now();
        if let Some(rt) = self.rt.get_mut(phone_id) {
            rt.traffic_sent_at = Some(now);
            rt.traffic_pending = false;
        }
    }

    pub(super) fn tick_traffic(&mut self, now: i64) {
        if self.store.state.traffic.month != month_of(now) {
            self.prepare_traffic();
        }
        let phones: Vec<_> = self
            .live_phones()
            .into_iter()
            .filter(|id| {
                self.rt.get(id).is_some_and(|rt| {
                    rt.traffic_sync
                        && rt.traffic_pending
                        && rt.traffic_sent_at.is_none_or(|at| now - at >= 60_000)
                })
            })
            .collect();
        // Keep pending for peers that have not reached their next update slot yet.
        for phone in phones {
            self.send_traffic(&phone);
        }
    }
}
