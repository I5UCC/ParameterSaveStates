//! Service-level tests that drive the modules against a real local socket.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use psstates_core::{ParamValue, Store};
use psstates_osc::discovery::DiscoveryEvent;
use psstates_osc::osc::IncomingMessage;

use crate::Service;
use psstates_core::model::FixedParam;
use std::net::Ipv4Addr;

fn service() -> (tokio::runtime::Runtime, Service, tempfile::TempDir) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    // No mDNS: a VRChat running on the developer machine must not interfere.
    let svc = Service::start_with(rt.handle().clone(), store, false).unwrap();
    (rt, svc, dir)
}

#[test]
fn avatar_change_applies_auto_sync_and_fixed() {
    let (rt, svc, _dir) = service();
    // Capture what we send.
    let sink = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    sink.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    svc.inner.sender.set_target(sink.local_addr().unwrap());

    svc.store()
        .update_settings(|s| {
            s.auto_sync_parameters = vec!["Hue".into()];
            s.fixed_parameters = vec![FixedParam::new("Glasses", ParamValue::Bool(true))];
            Ok(())
        })
        .unwrap();

    let inner = svc.inner.clone();
    rt.block_on(async move {
        inner.on_avatar_change("avtr_a".into(), false);
        inner.handle_osc(IncomingMessage {
            addr: "/avatar/parameters/Hue".into(),
            args: vec![rosc::OscType::Float(0.75)],
        });
        inner.on_avatar_change("avtr_b".into(), false);
    });

    let mut received = Vec::new();
    let mut buf = [0u8; 1024];
    // avtr_a: fixed; avtr_b: auto-sync Hue + fixed.
    for _ in 0..3 {
        let (n, _) = sink.recv_from(&mut buf).unwrap();
        match rosc::decoder::decode_udp(&buf[..n]).unwrap().1 {
            rosc::OscPacket::Message(m) => received.push(m.addr),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        received,
        [
            "/avatar/parameters/Glasses",
            "/avatar/parameters/Hue",
            "/avatar/parameters/Glasses"
        ]
    );
    let st = svc.status();
    assert_eq!(st.current_avatar.unwrap().id, "avtr_b");
    assert_eq!(st.previous_avatar.unwrap().id, "avtr_a");
    svc.shutdown();
}

/// VRChat announces itself via mDNS before its OSCQuery HTTP server answers.
/// The service must keep trying instead of giving up after the first failure.
#[test]
fn connects_when_vrchat_oscquery_comes_up_late() {
    use psstates_osc::oscquery::{OscQueryNode, ACCESS_READ_WRITE};
    use psstates_osc::server::{host_info, serve_oscquery};

    let (rt, svc, _dir) = service();

    // Reserve a port, then free it so the first connection attempts are refused.
    let port = {
        let l = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        l.local_addr().unwrap().port()
    };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let inner = svc.inner.clone();
    rt.block_on(async {
        inner.handle_discovery(DiscoveryEvent::Found {
            instance: "VRChat-Client-TEST".into(),
            addrs: vec![addr],
        });
    });
    // Wait until the first attempt has completely failed (Windows retransmits a
    // refused SYN for a while), so only the retry path can make this test pass.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while svc.inner.connecting.load(Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "first attempt never finished"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!svc.status().connected, "nothing is listening yet");

    // VRChat's OSCQuery server finally starts, with an avatar already loaded.
    let mut tree = OscQueryNode::container("/");
    tree.insert(
        "/avatar/change",
        OscQueryNode {
            full_path: "/avatar/change".into(),
            access: Some(ACCESS_READ_WRITE),
            osc_type: Some("s".into()),
            value: Some(vec![serde_json::json!("avtr_late")]),
            ..Default::default()
        },
    );
    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        serve_oscquery(listener, host_info("VRChat-Client-TEST", 9000), tree);
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    loop {
        let st = svc.status();
        if st.connected
            && st
                .current_avatar
                .as_ref()
                .is_some_and(|a| a.id == "avtr_late")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "did not connect in time: {st:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    svc.shutdown();
}
