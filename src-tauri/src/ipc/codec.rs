//! NDJSON 编解码：一行一个 JSON 消息，便于流式读写与调试。

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{Error, ErrorKind};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, Lines};

/// Retain partial NDJSON frames when a select branch or timeout cancels a read.
pub struct MessageReader<R> {
    lines: Lines<R>,
}

impl<R: AsyncBufRead + Unpin> MessageReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            lines: reader.lines(),
        }
    }

    pub async fn read<T: DeserializeOwned>(&mut self) -> std::io::Result<Option<T>> {
        match self.lines.next_line().await? {
            Some(line) => parse_line(&line),
            None => Ok(None),
        }
    }
}

fn parse_line<T: DeserializeOwned>(line: &str) -> std::io::Result<Option<T>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

/// Describe transport failures without formatting payloads embedded in serde/IO errors.
pub fn error_summary(error: &Error) -> String {
    if let Some(json) = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<serde_json::Error>())
    {
        format!(
            "json_{:?} line={} column={}",
            json.classify(),
            json.line(),
            json.column()
        )
    } else {
        format!("io_{:?} os={:?}", error.kind(), error.raw_os_error())
    }
}

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
/// This one-shot reader is not cancellation safe; reuse MessageReader in select/timeout loops.
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
    parse_line(&line)
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

    #[tokio::test(start_paused = true)]
    async fn persistent_reader_keeps_utf8_bytes_across_repeated_cancellation() {
        let (mut writer, reader) = tokio::io::duplex(128);
        let mut reader = MessageReader::new(BufReader::new(reader));
        let bytes = "{\"text\":\"中文\"}\n".as_bytes();
        let split = bytes.iter().position(|byte| *byte == 0xe4).unwrap() + 1;
        writer.write_all(&bytes[..split]).await.unwrap();
        for _ in 0..3 {
            assert!(tokio::time::timeout(
                std::time::Duration::from_millis(10),
                reader.read::<serde_json::Value>()
            )
            .await
            .is_err());
        }
        writer.write_all(&bytes[split..]).await.unwrap();
        assert_eq!(
            reader.read::<serde_json::Value>().await.unwrap().unwrap()["text"],
            "中文"
        );
    }

    #[tokio::test]
    async fn persistent_reader_preserves_frame_and_eof_semantics() {
        let mut reader = MessageReader::new(BufReader::new(&b" {\"n\":1}\r\n{\"n\":2}\n\n"[..]));
        assert_eq!(
            reader.read::<serde_json::Value>().await.unwrap().unwrap()["n"],
            1
        );
        assert_eq!(
            reader.read::<serde_json::Value>().await.unwrap().unwrap()["n"],
            2
        );
        assert!(reader.read::<serde_json::Value>().await.unwrap().is_none());
        assert!(reader.read::<serde_json::Value>().await.unwrap().is_none());
    }

    #[test]
    fn failure_summaries_never_format_user_values_or_io_messages() {
        #[derive(serde::Deserialize)]
        struct Expected {
            _number: u32,
        }
        let json = serde_json::from_str::<Expected>(r#"{"_number":"private answer"}"#)
            .err()
            .unwrap();
        let summary = error_summary(&Error::new(ErrorKind::InvalidData, json));
        assert!(summary.contains("json_Data"));
        assert!(!summary.contains("private answer"));
        let summary = error_summary(&Error::new(ErrorKind::BrokenPipe, "private credential"));
        assert!(summary.contains("io_BrokenPipe"));
        assert!(!summary.contains("private credential"));
    }
}
