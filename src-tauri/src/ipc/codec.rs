//! NDJSON 编解码：一行一个 JSON 消息，便于流式读写与调试。

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{Error, ErrorKind};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// 序列化 `msg` 为一行 JSON（追加换行）并写出、flush。
pub async fn write_msg<W, T>(w: &mut W, msg: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut line = serde_json::to_vec(msg).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    w.write_all(&line).await?;
    w.flush().await
}

/// 读取下一行并解析为 `T`。返回 `Ok(None)` 表示 EOF（对端关闭）。
pub async fn read_msg<R, T>(r: &mut R) -> std::io::Result<Option<T>>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    let mut line = String::new();
    let n = r.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None); // EOF
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        // 空行：跳过（不视为消息也不视为 EOF）。
        return Ok(None);
    }
    let msg = serde_json::from_str(trimmed).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::lifecycle::Fingerprint;
    use crate::ipc::{ClientHello, ClientMsg};
    use tokio::io::BufReader;

    #[tokio::test]
    async fn ndjson_round_trip() {
        let (mut tx, rx) = tokio::io::duplex(1024);
        let mut reader = BufReader::new(rx);
        let sent = ClientMsg::Hello(ClientHello {
            protocol_version: 1,
            client_version: "0.0.0".into(),
            binary_path: "/tmp/AskHuman".into(),
            fingerprint: Fingerprint { size: 7, hash: 42 },
            pid: 123,
        });
        write_msg(&mut tx, &sent).await.unwrap();
        let got: Option<ClientMsg> = read_msg(&mut reader).await.unwrap();
        match got {
            Some(ClientMsg::Hello(h)) => {
                assert_eq!(h.pid, 123);
                assert_eq!(h.fingerprint.size, 7);
                assert_eq!(h.protocol_version, 1);
            }
            other => panic!("unexpected message: {:?}", other),
        }
    }

    #[tokio::test]
    async fn eof_returns_none() {
        let (tx, rx) = tokio::io::duplex(16);
        drop(tx); // 关闭写端 → 读端 EOF
        let mut reader = BufReader::new(rx);
        let got: Option<ClientMsg> = read_msg(&mut reader).await.unwrap();
        assert!(got.is_none());
    }
}
