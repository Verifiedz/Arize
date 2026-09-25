//! The client against a scripted server speaking `swe-proto`, for the paths a healthy daemon
//! never takes: stray events, out-of-order responses, a refused handshake, a stale socket.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use swe_cli::{Client, ConnectError};
use swe_core::{Error, ErrorCode, Event, ModuleId};
use swe_proto::{decode_client, encode, ClientFrame, ServerFrame};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

/// Accept one connection and hand each client frame to `reply`, which returns the frames to
/// send back. The server answers the handshake itself unless `reply` handles `Hello`.
async fn serve(sock: &Path, mut reply: impl FnMut(ClientFrame) -> Vec<ServerFrame> + Send + 'static) {
    let listener = UnixListener::bind(sock).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (rd, mut wr) = stream.into_split();
        let mut lines = BufReader::new(rd).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            for frame in reply(decode_client(&line).unwrap()) {
                wr.write_all(encode(&frame).unwrap().as_bytes()).await.unwrap();
            }
        }
    });
}

fn welcome() -> ServerFrame {
    ServerFrame::welcome("test", "s1")
}

fn stray_event() -> ServerFrame {
    ServerFrame::event(Event::new(ModuleId::new("records"), "records.item.created", json!({}), Default::default()))
}

fn sock(dir: &TempDir) -> PathBuf {
    dir.path().join("s.sock")
}

#[tokio::test]
async fn call_skips_events_and_other_ids() {
    let dir = TempDir::new().unwrap();
    serve(&sock(&dir), |f| match f {
        ClientFrame::Hello { .. } => vec![welcome()],
        ClientFrame::Request { id, params, .. } => {
            vec![stray_event(), ServerFrame::ok("someone-else", json!("wrong")), ServerFrame::ok(id, params)]
        }
        _ => vec![],
    })
    .await;
    let mut c = Client::connect(&sock(&dir)).await.unwrap();
    assert_eq!(c.call("x.echo", json!({"a": 1})).await.unwrap(), json!({"a": 1}));
    // Ids keep increasing on one connection.
    assert_eq!(c.call("x.echo", json!({"a": 2})).await.unwrap(), json!({"a": 2}));
}

#[tokio::test]
async fn daemon_errors_keep_their_code_and_detail() {
    let dir = TempDir::new().unwrap();
    serve(&sock(&dir), |f| match f {
        ClientFrame::Hello { .. } => vec![welcome()],
        ClientFrame::Request { id, .. } => {
            vec![ServerFrame::err(id, Error::not_found("no collection 'x'").with_detail(json!({"collection": "x"})))]
        }
        _ => vec![],
    })
    .await;
    let mut c = Client::connect(&sock(&dir)).await.unwrap();
    let e = c.call("records.list", Value::Null).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(e.detail, Some(json!({"collection": "x"})));
}

#[tokio::test]
async fn a_refused_handshake_is_reported_not_retried() {
    let dir = TempDir::new().unwrap();
    serve(&sock(&dir), |f| match f {
        ClientFrame::Hello { .. } => {
            vec![ServerFrame::error(Error::new(ErrorCode::UnsupportedVersion, "this daemon speaks v2"))]
        }
        _ => vec![],
    })
    .await;
    let e = Client::connect(&sock(&dir)).await.err().unwrap();
    assert!(!e.nobody_listening(), "a live daemon that refuses us must not trigger an auto-start");
    let e = e.into_error(&sock(&dir));
    assert_eq!(e.code, ErrorCode::UnsupportedVersion);
}

#[tokio::test]
async fn a_closed_connection_is_unavailable() {
    let dir = TempDir::new().unwrap();
    // Welcomes, then answers the request by hanging up.
    let listener = UnixListener::bind(sock(&dir)).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (rd, mut wr) = stream.into_split();
        let mut lines = BufReader::new(rd).lines();
        lines.next_line().await.unwrap();
        wr.write_all(encode(&welcome()).unwrap().as_bytes()).await.unwrap();
        lines.next_line().await.unwrap();
    });
    let mut c = Client::connect(&sock(&dir)).await.unwrap();
    let e = c.call("core.ping", Value::Null).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Unavailable);
}

#[tokio::test]
async fn missing_and_stale_sockets_mean_nobody_is_listening() {
    let dir = TempDir::new().unwrap();
    let missing = Client::connect(&sock(&dir)).await.err().unwrap();
    assert!(missing.nobody_listening());

    // A socket file left behind by a daemon that died.
    drop(std::os::unix::net::UnixListener::bind(sock(&dir)).unwrap());
    assert!(sock(&dir).exists());
    let stale = Client::connect(&sock(&dir)).await.err().unwrap();
    assert!(matches!(stale, ConnectError::Io(_)));
    assert!(stale.nobody_listening());
}
