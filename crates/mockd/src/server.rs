//! The socket server. Framing, handshake and error behaviour follow the real daemon so a
//! client cannot tell the difference; what differs is only where answers come from.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use swe_core::{Clock, Error, ErrorCode, Event, ModuleId, Result};
use swe_proto::{
    decode_client, encode, is_valid_pattern, ops, topic_matches, ClientFrame, ManifestData, ServerFrame,
    MAX_LINE_BYTES, PROTOCOL_VERSION,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::fixtures::{Fixtures, Lookup, TimedEvent};
use crate::options::Options;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

type Subs = Arc<Mutex<Vec<String>>>;

struct Shared {
    fixtures: Fixtures,
    started: Instant,
    clock: Clock,
    shutdown: CancellationToken,
    tracker: TaskTracker,
}

#[derive(Clone)]
pub struct ShutdownHandle(CancellationToken);

impl ShutdownHandle {
    pub fn shutdown(&self) {
        self.0.cancel();
    }
}

/// A running mock daemon. Serving happens on background tasks; [`wait`](Self::wait) blocks
/// until shutdown is requested, by [`shutdown`](Self::shutdown) or a `core.shutdown` request.
pub struct Mockd {
    shared: Arc<Shared>,
    socket: PathBuf,
}

impl Mockd {
    /// Load and validate the fixtures, then listen. Fails, naming the file, if a fixture is bad.
    pub async fn start(options: Options) -> Result<Self> {
        let fixtures = Fixtures::load(&options.fixtures)?;
        let listener = bind(&options.socket)?;
        let shared = Arc::new(Shared {
            fixtures,
            started: Instant::now(),
            clock: Clock::system(),
            shutdown: CancellationToken::new(),
            tracker: TaskTracker::new(),
        });
        shared.tracker.spawn(accept(shared.clone(), listener));
        tracing::info!(socket = %options.socket.display(), fixtures = %options.fixtures.display(), "mock daemon ready");
        Ok(Self { shared, socket: options.socket })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle(self.shared.shutdown.clone())
    }

    pub fn shutdown(&self) {
        self.shared.shutdown.cancel();
    }

    /// Resolves after shutdown is requested and connections have drained (or a short grace
    /// period expired). Removes the socket file.
    pub async fn wait(self) {
        self.shared.shutdown.cancelled().await;
        self.shared.tracker.close();
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, self.shared.tracker.wait()).await;
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Bind the socket, clearing a stale one left by a mock that died without cleanup, but never
/// one that is still being listened on (the real daemon's, say).
fn bind(path: &Path) -> Result<UnixListener> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty() && !p.exists()) {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(Error::conflict(format!("something is already listening on {}", path.display())));
        }
        std::fs::remove_file(path)?;
    }
    Ok(UnixListener::bind(path)?)
}

async fn accept(shared: Arc<Shared>, listener: UnixListener) {
    loop {
        tokio::select! {
            _ = shared.shutdown.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let shared = shared.clone();
                    shared.tracker.clone().spawn(async move {
                        if let Err(e) = connection(shared, stream).await {
                            tracing::debug!(error = %e, "connection ended with an I/O error");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "accept failed");
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }
}

enum Line {
    Eof,
    Text(Vec<u8>),
    TooLong,
}

/// `read_until` with a ceiling, so a peer that never sends a newline cannot eat memory.
async fn read_line<R: AsyncBufRead + Unpin>(r: &mut R) -> io::Result<Line> {
    let mut buf = Vec::new();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() { Line::Eof } else { Line::Text(buf) });
        }
        let newline = available.iter().position(|&b| b == b'\n');
        let take = newline.unwrap_or(available.len());
        buf.extend_from_slice(&available[..take]);
        r.consume(newline.map_or(take, |i| i + 1));
        if buf.len() > MAX_LINE_BYTES {
            return Ok(Line::TooLong);
        }
        if newline.is_some() {
            return Ok(Line::Text(buf));
        }
    }
}

async fn write_frame(w: &mut OwnedWriteHalf, frame: &ServerFrame) -> io::Result<()> {
    let line = encode(frame).map_err(io::Error::other)?;
    w.write_all(line.as_bytes()).await?;
    w.flush().await
}

fn decode(bytes: &[u8]) -> Result<ClientFrame> {
    std::str::from_utf8(bytes).map_err(|_| Error::bad_request("frame is not UTF-8")).and_then(decode_client)
}

async fn connection(shared: Arc<Shared>, stream: UnixStream) -> io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);

    // Handshake: `hello` first, before anything else.
    let first = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_line(&mut rd))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no hello"))??;
    let hello = match first {
        Line::Eof => return Ok(()),
        Line::TooLong => return write_frame(&mut wr, &ServerFrame::error(Error::bad_request("line too long"))).await,
        Line::Text(bytes) => decode(&bytes),
    };
    match hello {
        Ok(ClientFrame::Hello { v, .. }) if v == PROTOCOL_VERSION => {}
        Ok(ClientFrame::Hello { v, .. }) => {
            let e = Error::new(
                ErrorCode::UnsupportedVersion,
                format!("protocol v{v} not supported; this daemon speaks v{PROTOCOL_VERSION}"),
            )
            .with_detail(json!({"supported": [PROTOCOL_VERSION]}));
            return write_frame(&mut wr, &ServerFrame::error(e)).await;
        }
        Ok(_) => return write_frame(&mut wr, &ServerFrame::error(Error::bad_request("expected hello"))).await,
        Err(e) => return write_frame(&mut wr, &ServerFrame::error(e)).await,
    }
    write_frame(&mut wr, &ServerFrame::welcome(env!("CARGO_PKG_VERSION"), ulid::Ulid::new().to_string())).await?;

    let subs: Subs = Arc::default();
    let (out, out_rx) = mpsc::channel::<ServerFrame>(256);
    let writer = shared.tracker.spawn(writer(wr, out_rx, shared.shutdown.clone()));
    let mut timeline_started = false;

    loop {
        let line = tokio::select! {
            _ = shared.shutdown.cancelled() => break,
            l = read_line(&mut rd) => l?,
        };
        let bytes = match line {
            Line::Eof => break,
            Line::TooLong => {
                let _ = out.send(ServerFrame::error(Error::bad_request("line too long"))).await;
                break;
            }
            Line::Text(b) if b.iter().all(u8::is_ascii_whitespace) => continue,
            Line::Text(b) => b,
        };
        match decode(&bytes) {
            Err(e) => {
                let _ = out.send(ServerFrame::error(e)).await;
            }
            Ok(ClientFrame::Hello { .. }) => {
                let _ = out.send(ServerFrame::error(Error::bad_request("already greeted"))).await;
            }
            Ok(ClientFrame::Request { id, op, params, queue, .. }) => {
                let control = serde_json::to_value(queue.unwrap_or_default()).unwrap_or_default();
                shared.tracker.spawn(request(shared.clone(), out.clone(), subs.clone(), id, op, params, control));
            }
            Ok(ClientFrame::Subscribe { id, topics, .. }) => {
                let reply = match topics.iter().find(|t| !is_valid_pattern(t)) {
                    Some(bad) => ServerFrame::err(id, Error::invalid_params(format!("invalid topic pattern '{bad}'"))),
                    None => {
                        let total = update_subs(&subs, |s| {
                            for t in topics {
                                if !s.contains(&t) {
                                    s.push(t);
                                }
                            }
                        });
                        ServerFrame::ok(id, json!({"subscribed": total}))
                    }
                };
                let ok = matches!(reply, ServerFrame::Response { ok: true, .. });
                let _ = out.send(reply).await;
                // The scripted timeline starts at a connection's first successful subscribe,
                // so a client that subscribes late does not miss the opening of it.
                if ok && !timeline_started && !shared.fixtures.timeline().is_empty() {
                    timeline_started = true;
                    let events = shared.fixtures.timeline().to_vec();
                    shared.tracker.spawn(replay(shared.clone(), events, out.clone(), subs.clone()));
                }
            }
            Ok(ClientFrame::Unsubscribe { id, topics, .. }) => {
                let total = update_subs(&subs, |s| s.retain(|p| !topics.contains(p)));
                let _ = out.send(ServerFrame::ok(id, json!({"subscribed": total}))).await;
            }
        }
    }
    // Dropping our sender lets the writer finish once in-flight handlers have replied.
    drop(out);
    let _ = writer.await;
    Ok(())
}

/// Synchronous on purpose: the lock must never be held across an `.await`.
fn update_subs(subs: &Mutex<Vec<String>>, f: impl FnOnce(&mut Vec<String>)) -> usize {
    let mut s = subs.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut s);
    s.len()
}

/// The only task that touches the socket's write half, so frames never interleave.
async fn writer(mut wr: OwnedWriteHalf, mut out: mpsc::Receiver<ServerFrame>, shutdown: CancellationToken) {
    loop {
        tokio::select! {
            biased;
            f = out.recv() => match f {
                Some(f) => if write_frame(&mut wr, &f).await.is_err() { return },
                None => break,
            },
            _ = shutdown.cancelled() => {
                while let Ok(f) = out.try_recv() {
                    if write_frame(&mut wr, &f).await.is_err() { return }
                }
                break;
            }
        }
    }
    let _ = wr.shutdown().await;
}

/// Answer one request. Runs on its own task so a `delay_ms` never holds up the connection,
/// and responses may arrive out of order, as they may from the real daemon.
async fn request(
    shared: Arc<Shared>,
    out: mpsc::Sender<ServerFrame>,
    subs: Subs,
    id: String,
    op: String,
    params: Value,
    control: Value,
) {
    let (result, delay, emit) = match shared.fixtures.lookup(&op, &params, &control) {
        Lookup::Hit(rule) => {
            let result = match (&rule.data, &rule.error) {
                (_, Some(e)) => Err(e.clone()),
                (Some(d), None) => Ok(d.clone()),
                (None, None) => Err(Error::internal("fixture has neither data nor error")),
            };
            (result, Duration::from_millis(rule.delay_ms), rule.emit.clone())
        }
        Lookup::NoMatch => (
            Err(Error::invalid_params(format!(
                "mockd: fixtures exist for '{op}' but none match these params and queue control"
            ))),
            Duration::ZERO,
            Vec::new(),
        ),
        Lookup::UnknownOp => (builtin(&shared, &op), Duration::ZERO, Vec::new()),
    };
    if !delay.is_zero() {
        sleep(delay).await;
    }
    let is_shutdown = op == ops::CORE_SHUTDOWN && result.is_ok();
    let frame = match result {
        Ok(data) => ServerFrame::ok(id, data),
        Err(e) => ServerFrame::err(id, e),
    };
    if out.send(frame).await.is_err() {
        return;
    }
    if is_shutdown {
        // Only after the response is queued; the writer drains it first.
        shared.shutdown.cancel();
        return;
    }
    replay(shared, emit, out, subs).await;
}

/// What a fixture set need not spell out. Any of these can be overridden by a fixture.
fn builtin(shared: &Shared, op: &str) -> Result<Value> {
    match op {
        ops::CORE_PING => Ok(json!({"pong": true, "uptime_s": shared.started.elapsed().as_secs()})),
        ops::CORE_MANIFEST => Ok(serde_json::to_value(ManifestData {
            protocol: PROTOCOL_VERSION,
            lanes: vec![swe_core::LaneConfig::new("default", 4)],
            modules: Vec::new(),
        })
        .unwrap_or_default()),
        ops::CORE_SHUTDOWN => Ok(json!({"ok": true})),
        _ => Err(Error::unknown_op(format!("mockd: no fixture for op '{op}'"))),
    }
}

/// Push `events` in `after_ms` order, each only if this connection subscribed to its topic.
async fn replay(shared: Arc<Shared>, mut events: Vec<TimedEvent>, out: mpsc::Sender<ServerFrame>, subs: Subs) {
    events.sort_by_key(|e| e.after_ms);
    let start = tokio::time::Instant::now();
    for e in events {
        tokio::select! {
            _ = shared.shutdown.cancelled() => return,
            _ = tokio::time::sleep_until(start + Duration::from_millis(e.after_ms)) => {}
        }
        let wanted = subs.lock().unwrap_or_else(|p| p.into_inner()).iter().any(|p| topic_matches(p, &e.topic));
        if !wanted {
            continue;
        }
        let event = Event::new(ModuleId::new(e.source()), e.topic.clone(), e.payload.clone(), shared.clock.now());
        if out.send(ServerFrame::event(event)).await.is_err() {
            return;
        }
    }
}
