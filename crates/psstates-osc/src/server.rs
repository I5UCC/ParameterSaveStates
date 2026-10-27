//! Minimal OSCQuery HTTP server advertising which OSC addresses we want to
//! receive. VRChat reads this tree to decide which messages to send us.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tokio::net::TcpListener;

use crate::oscquery::{HostInfo, OscQueryNode, ACCESS_WRITE};

struct ServerState {
    host_info: HostInfo,
    root: OscQueryNode,
}

/// Builds the namespace tree we expose.
pub fn build_tree() -> OscQueryNode {
    let mut root = OscQueryNode::container("/");
    root.insert(
        "/avatar/change",
        OscQueryNode {
            full_path: "/avatar/change".into(),
            access: Some(ACCESS_WRITE),
            osc_type: Some("s".into()),
            description: Some("Avatar change notifications".into()),
            ..Default::default()
        },
    );
    root
}

fn json(status: StatusCode, body: String) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

async fn handle(state: Arc<ServerState>, uri: Uri) -> Response {
    if uri.query().is_some_and(|q| q.contains("HOST_INFO")) {
        return json(
            StatusCode::OK,
            serde_json::to_string(&state.host_info).unwrap_or_default(),
        );
    }
    match state.root.find(uri.path()) {
        Some(node) => json(
            StatusCode::OK,
            serde_json::to_string(node).unwrap_or_default(),
        ),
        None => (StatusCode::NOT_FOUND, "OSC Path not found").into_response(),
    }
}

/// The `HOST_INFO` we advertise.
pub fn host_info(name: &str, osc_port: u16) -> HostInfo {
    HostInfo {
        name: name.to_string(),
        extensions: [
            ("ACCESS", true),
            ("CLIPMODE", false),
            ("RANGE", true),
            ("TYPE", true),
            ("VALUE", true),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
        osc_ip: Ipv4Addr::LOCALHOST.to_string(),
        osc_port,
        osc_transport: "UDP".into(),
    }
}

/// Serves an OSCQuery namespace on an already bound listener.
pub fn serve_oscquery(listener: TcpListener, host_info: HostInfo, root: OscQueryNode) {
    let state = Arc::new(ServerState { host_info, root });
    let app = Router::new().fallback(move |uri: Uri| handle(state.clone(), uri));
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("OSCQuery server stopped: {e}");
        }
    });
}

/// Starts the server on all interfaces with a random port and returns the port.
pub async fn start_oscquery_server(name: &str, osc_port: u16) -> std::io::Result<u16> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))).await?;
    let port = listener.local_addr()?.port();
    serve_oscquery(listener, host_info(name, osc_port), build_tree());
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oscquery::OscQueryClient;

    #[tokio::test]
    async fn serves_tree_and_host_info() {
        let port = start_oscquery_server("Test", 1234).await.unwrap();
        let client = OscQueryClient::new(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
        let info = client.host_info().await.unwrap();
        assert_eq!(info.name, "Test");
        assert_eq!(info.osc_port, 1234);
        let node = client.node("/avatar/change").await.unwrap().unwrap();
        assert_eq!(node.osc_type.as_deref(), Some("s"));
        assert!(client.node("/nope").await.unwrap().is_none());
    }
}
