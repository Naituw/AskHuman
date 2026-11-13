use super::testkit::*;
use crate::ios::{crypto, store::Traffic, traffic::TrafficRecord, wire};

#[tokio::test(start_paused = true)]
async fn a_capable_phone_merges_owners_and_keeps_the_ledger_after_revocation_and_restart() {
    let fx = Fixture::new().await;
    let mut phone = fx.pair_phone().await;
    phone.set_presence(true, true).await;
    phone
        .send(&wire::Hello {
            traffic_usage: true,
            traffic_sync: false,
            nonce: "traffic-test".into(),
            proto: wire::PROTO,
            app_ver: "1".into(),
            device_name: "phone".into(),
            lifecycle_results: false,
            lean_watch: false,
        })
        .await;
    let ack: wire::HelloAck = phone.recv_body(wire::t::HELLO_ACK).await;
    assert!(ack.caps.traffic_usage);
    let first: wire::TrafficSync = phone.recv_body("traffic.usage").await;
    let own = first.records.first().unwrap();
    let remote_id = crypto::new_id();
    let remote = TrafficRecord {
        device_kind: crate::ios::traffic::DeviceKind::Mac,
        recorded_from: None,
        uncertain: false,
        broker: own.broker.clone(),
        mac_id: remote_id.clone(),
        mac_name: Some("Other Mac".into()),
        updated_at: own.updated_at,
        counters: Traffic {
            month: own.counters.month.clone(),
            bytes_out: 500,
            ..Traffic::default()
        },
    };
    phone
        .send(&wire::TrafficSync {
            records: vec![remote.clone(), remote.clone()],
        })
        .await;
    fx.settle().await;
    let state = fx.gateway.state().await.unwrap();
    assert_eq!(state.traffic_records.len(), 2);
    assert_eq!(
        state
            .traffic_records
            .iter()
            .find(|r| r.mac_id == remote_id)
            .unwrap()
            .counters
            .bytes_out,
        500
    );
    assert!(fx.gateway.revoke(&phone.phone_id).await);
    let fx = fx.restart().await;
    let state = fx.gateway.state().await.unwrap();
    assert_eq!(
        state
            .traffic_records
            .iter()
            .find(|r| r.mac_id == remote_id)
            .unwrap(),
        &remote
    );
    fx.gateway.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn legacy_hello_does_not_receive_traffic_messages() {
    let fx = Fixture::new().await;
    let mut phone = fx.live_phone().await;
    assert!(
        !phone
            .gets("traffic.usage", std::time::Duration::from_millis(500))
            .await
    );
    fx.gateway.shutdown().await;
}
