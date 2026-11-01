//! Let modern stdio clients probe before falling back to the SDK's legacy handshake.

use std::future::Future;

use rmcp::{
    model::{ClientJsonRpcMessage, ClientRequest, ErrorCode, ErrorData, ServerJsonRpcMessage},
    transport::Transport,
    RoleServer,
};

pub(super) struct LegacyDiscoveryTransport<T> {
    inner: T,
    awaiting_initialize: bool,
}

impl<T> LegacyDiscoveryTransport<T> {
    pub(super) fn new(inner: T) -> Self {
        Self {
            inner,
            awaiting_initialize: true,
        }
    }
}

impl<T: Transport<RoleServer>> Transport<RoleServer> for LegacyDiscoveryTransport<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            let message = self.inner.receive().await?;
            if self.awaiting_initialize {
                if let ClientJsonRpcMessage::Request(request) = &message {
                    if let ClientRequest::CustomRequest(custom) = &request.request {
                        if custom.method == "server/discover" {
                            // MCP 2026-07-28 stdio probing permits an error to trigger the
                            // legacy initialize flow. Do not advertise modern semantics.
                            let error = ErrorData::new(
                                ErrorCode::METHOD_NOT_FOUND,
                                "server/discover is unsupported; use initialize",
                                None,
                            );
                            self.inner
                                .send(ServerJsonRpcMessage::error(error, Some(request.id.clone())))
                                .await
                                .ok()?;
                            continue;
                        }
                    }
                    if matches!(request.request, ClientRequest::InitializeRequest(_)) {
                        self.awaiting_initialize = false;
                    }
                }
            }
            return Some(message);
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.inner.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{transport::IntoTransport, ServiceExt};
    use serde_json::{json, Value};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn handshake_and_list(probe: bool) {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(async move {
            super::super::ask::AskServer::new()
                .serve(LegacyDiscoveryTransport::new(server_io.into_transport()))
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let (read, mut write) = tokio::io::split(client_io);
        let mut reader = BufReader::new(read);
        if probe {
            // Retry probes and both JSON-RPC ID forms must preserve the same connection.
            for id in [json!(1), json!("retry")] {
                let request = json!({"jsonrpc":"2.0","id":id,"method":"server/discover",
                    "params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}});
                write
                    .write_all(format!("{request}\n").as_bytes())
                    .await
                    .unwrap();
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let reply: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(reply["id"], id);
                assert_eq!(reply["error"]["code"], -32601);
                assert!(reply.get("result").is_none());
            }
        }
        let request = json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},
            "clientInfo":{"name":"discovery-test","version":"0"}}});
        write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
        write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/list\"}\n").await.unwrap();
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 6);
        assert!(!tools.iter().any(|tool| tool["name"] == "dev_ask"));
        assert!(tools.iter().any(|tool| tool["name"] == "ask"));
        drop(write);
        drop(reader);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn discovery_probe_falls_back_to_real_sdk_handshake() {
        tokio::time::timeout(std::time::Duration::from_secs(3), handshake_and_list(true))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn legacy_client_handshake_remains_supported() {
        tokio::time::timeout(std::time::Duration::from_secs(3), handshake_and_list(false))
            .await
            .unwrap();
    }
}
