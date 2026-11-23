//! `agentdrafter serve`: the HTTP API, a thin layer over the core's registry
//! of commands, as the command line is (ADR-0007). It holds no drawing
//! logic: it reads a request, hands the command and its input to the core,
//! and writes the core's reply.
//!
//! - `GET /commands` lists every command, with what it does and its input's
//!   JSON Schema.
//! - `POST /commands/<name>` runs one. The body is the command's input, a
//!   JSON object, sent as `Content-Type: application/json`; an empty body is
//!   `{}`. The reply is the one the command line prints, with status 200
//!   when it is ok, 400 when the request is wrong and 422 when the core
//!   reports an error, as the command line exits 0, 2 and 1.
//! - `GET /` is the viewer's page, and `GET /assets/...` its files
//!   (`page.rs`), which draw a drawing set's sheets in the browser from the
//!   display list the page asks `display-list` for.
//! - `POST /events` watches a drawing set for the viewer: the body names
//!   the set, `{"set": "..."}`, and the response is a stream of
//!   Server-Sent Events that stays open, each event the `journal`
//!   command's reply, sent when the stream opens and again each time the
//!   set's journal changes on disk, whichever process changed it, or the
//!   `proposals` command's, sent when the stream opens and again each time
//!   a proposal is made, kept or discarded, and the `guides` command's,
//!   sent when the stream opens and again each time a guide is given or
//!   the journal changes, which moves the guides' steps on. Every
//!   change set and undo, and every accept, adds to the journal, so a
//!   change set an agent sends from another process, through any
//!   interface, reaches an open viewer without a reload. It is a POST with
//!   a JSON body, and carries the token beyond the local machine as every
//!   command does, because a page reads the stream with `fetch`, which
//!   sends headers, where `EventSource` sends none.
//!
//! - `/assistant` and the routes under it are the drafter's assistant
//!   (#156, `assistant.rs`): `GET /assistant` lists the agents found on the
//!   PATH when `serve` started, `POST /assistant/find` looks again,
//!   `POST /assistant/messages` passes the drafter's message to their agent,
//!   `POST /assistant/agent` picks the agent, and `POST /assistant/events`
//!   streams the conversation on a set as it changes, as `/events` streams
//!   the journal.
//!
//! A command the viewer's page sends is recorded as the viewer's, not the
//! HTTP API's, where it records the interface, as an accept does: the
//! browser marks a request its page makes to its own server
//! `Sec-Fetch-Site: same-origin`, a header no script can set.
//!
//! It is an open port to guard (ADR-0007). It listens on the local machine
//! unless `--listen` names another address. Zach decided on 2026-11-11 that
//! a token is needed only when it listens beyond the local machine: then a
//! new token is made each time it starts and printed in the line it starts
//! with, every request carries it as `Authorization: Bearer <token>`, it is
//! compared in constant time, and a request without it is refused with a
//! JSON error. On the local machine no token is needed, but a request must
//! be addressed to the local machine, by its `Host` and any `Origin`, so a
//! web page in the reviewer's browser cannot reach it under another name,
//! and a body must be sent as JSON, which a page cannot send to another
//! site without asking first.
//!
//! The viewer's page and its files are served without the token, since a
//! browser cannot send one to open a page: they are the page's own built
//! files, embedded in the binary, and give nothing of the machine's. On
//! the local machine they are still served only to requests addressed to
//! it. The page takes the token from the address the start line prints,
//! whose fragment holds it, and sends it with every command.
//!
//! HTTP is read plainly: one request on each connection, its body sized by
//! `Content-Length` and no chunked body, so no request can be read as part
//! of another. A connection has 30 seconds to send its whole request and
//! 30 to take its reply, however slowly it sends, and a refused request is
//! closed unread, so no client can hold the server. Requests are taken one
//! at a time, so two change sets never run on a set at once. An event
//! stream is the one response that stays open: each is watched on a thread
//! of its own, which only reads, at most [`MOST_WATCHERS`] at once.

use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use agentdrafter_core::{
    commands, find_command, original_journal_file, Error, ErrorCode, Interface, Reply, Settings,
    GUIDES_FOLDER, JOURNAL_FILE, PROPOSALS_FOLDER,
};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;

use crate::assistant::{assistant, Choice, Message};
use crate::{page, send, Args};

/// The port `serve` listens on when `--listen` names none.
pub const DEFAULT_PORT: u16 = 7316;

pub const SERVE_USAGE: &str = "agentdrafter serve [--listen <address:port>]";

/// What `serve` does, for the command line's help.
pub fn serve_does() -> String {
    format!(
        "Serves every command as a local HTTP API, on 127.0.0.1:{DEFAULT_PORT} unless --listen \
         names another address and port. GET /commands lists the commands, each with what it does \
         and its input's JSON Schema; POST /commands/<name> runs one, its body the command's \
         input as JSON (Content-Type: application/json), and replies with the JSON the command \
         line prints. Paths are taken from the folder the server started in. GET / is the \
         viewer, which draws the drawing set in that folder in a browser. It prints one line \
         of JSON when it starts, saying where it listens and the viewer's address. On an \
         address beyond the local machine every command needs the token that line gives, as \
         Authorization: Bearer <token>; the viewer's address carries it. It runs until it is \
         stopped."
    )
}

/// The most a request's line and headers may hold.
const MOST_HEAD: usize = 64 * 1024;

/// The most a request's body may hold.
const MOST_BODY: usize = 64 * 1024 * 1024;

/// How long a connection has to send its whole request, and then to take
/// its whole reply. However slowly a client sends, it is cut off then.
const DEADLINE: Duration = Duration::from_secs(30);

/// The most event streams open at once; one more is refused with 503.
pub const MOST_WATCHERS: usize = 16;

/// How often an event stream looks at its set's journal.
const WATCH_EVERY: Duration = Duration::from_millis(200);

/// How long an event stream may say nothing before it sends a comment, so
/// a connection the page has let go of is found closed.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The event streams open now.
static WATCHERS: AtomicUsize = AtomicUsize::new(0);

/// The time by which a connection must have sent its request, or taken
/// its reply.
#[derive(Clone, Copy)]
struct Deadline {
    by: Instant,
    limit: Duration,
}

impl Deadline {
    fn after(limit: Duration) -> Self {
        Self {
            by: Instant::now() + limit,
            limit,
        }
    }

    /// How long is left, or an error once the time is up.
    fn left(&self) -> io::Result<Duration> {
        let left = self.by.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                ErrorKind::TimedOut,
                format!("a connection has {:?} for its request", self.limit),
            ));
        }
        Ok(left)
    }

    /// Reads what has come on `stream`, waiting no later than the deadline.
    fn read(&self, stream: &mut TcpStream, buffer: &mut [u8]) -> io::Result<usize> {
        stream.set_read_timeout(Some(self.left()?))?;
        stream.read(buffer)
    }

    /// Writes all of `bytes` on `stream`, waiting no later than the
    /// deadline.
    fn write(&self, stream: &mut TcpStream, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            stream.set_write_timeout(Some(self.left()?))?;
            match stream.write(bytes) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(written) => bytes = &bytes[written..],
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        stream.flush()
    }

    /// The response to a request that did not arrive in time.
    fn late(&self) -> Response {
        Response::usage(
            408,
            format!(
                "a connection has {:?} to send its request, and this one's did not arrive in time",
                self.limit
            ),
        )
    }
}

/// Runs `agentdrafter serve`: listens, prints the line it starts with, and
/// serves until it is stopped.
pub fn serve(args: Args) -> ExitCode {
    let (listener, guard) = match listen(args) {
        Ok(listening) => listening,
        Err(error) => return print(send(Reply::<()>::from(Err(error)))),
    };
    let address = match listener.local_addr() {
        Ok(address) => address,
        Err(e) => {
            let error = Error::new(ErrorCode::ListenFailed, format!("could not listen: {e}"));
            return print(send(Reply::<()>::from(Err(error))));
        }
    };
    let (line, _) = send(Reply::from(Ok(started(address, &guard))));
    println!("{line}");
    let _ = std::io::stdout().flush();
    let settings = Settings::from_env();
    // The agents are looked for on the PATH as the server starts.
    let _ = assistant();
    // A connection that failed to open is passed over.
    for stream in listener.incoming().flatten() {
        handle(stream, &guard, &settings, DEADLINE, None);
    }
    ExitCode::SUCCESS
}

/// An in-process local HTTP server for the desktop shell (`crates/agentdrafter-desktop`).
///
/// Binds `127.0.0.1:0`, guards every command, event stream and assistant route
/// with a per-session random bearer token plus loopback `Host`/`Origin` checks,
/// serves the embedded viewer page (`viewer/dist/`), and shuts down cleanly on
/// [`LocalServer::shutdown`] or [`Drop`].
pub struct LocalServer {
    address: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    done: Option<mpsc::Receiver<()>>,
    thread: Option<JoinHandle<()>>,
}

impl LocalServer {
    /// The loopback socket address the server is bound to.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The server's base HTTP URL, such as `http://127.0.0.1:54321`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The per-session random bearer token required on every API and stream request.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether the built viewer page (`viewer/dist/`) was embedded into this binary.
    pub fn viewer_built(&self) -> bool {
        page::VIEWER_BUILT
    }

    /// Stops accepting connections, signals open event streams to exit, and
    /// waits briefly for the server thread to finish without ever hanging UI
    /// teardown if an in-flight request is slow.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocking `accept()` call on the server thread with a short timeout.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(200));
        if let Some(done) = self.done.take() {
            let exited = done.recv_timeout(Duration::from_millis(500)).is_ok();
            if exited {
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
            } else {
                // Detach cleanly rather than stalling app exit behind a slow handler.
                let _ = self.thread.take();
            }
        } else if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Starts an in-process local server on `127.0.0.1:0` with a fresh per-session
/// random bearer token for the desktop shell.
pub fn start_local(settings: Settings) -> Result<LocalServer, Error> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| {
        Error::new(
            ErrorCode::ListenFailed,
            format!("could not start the local server on 127.0.0.1:0: {e}"),
        )
    })?;
    let address = listener.local_addr().map_err(|e| {
        Error::new(
            ErrorCode::ListenFailed,
            format!("could not read the local server's address: {e}"),
        )
    })?;
    let token = Guard::new_token()?;
    let guard = Guard::DesktopToken {
        token: token.clone(),
        host: address.to_string(),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (done_tx, done_rx) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("agentdrafter-local-server".into())
        .spawn(move || {
            let _ = assistant();
            for stream in listener.incoming().flatten() {
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                handle(
                    stream,
                    &guard,
                    &settings,
                    DEADLINE,
                    Some(Arc::clone(&thread_stop)),
                );
            }
            drop(listener);
            let _ = done_tx.send(());
        })
        .map_err(|e| {
            Error::new(
                ErrorCode::ListenFailed,
                format!("could not start the local server thread: {e}"),
            )
        })?;
    Ok(LocalServer {
        address,
        token,
        stop,
        done: Some(done_rx),
        thread: Some(thread),
    })
}

fn print((line, code): (String, ExitCode)) -> ExitCode {
    println!("{line}");
    code
}

/// Reads `serve`'s arguments and listens where they say, with the guard
/// that address needs.
fn listen(args: Args) -> Result<(TcpListener, Guard), Error> {
    let address = listen_address(args)?;
    let addresses: Vec<SocketAddr> = address
        .to_socket_addrs()
        .map_err(|e| {
            Error::usage(format!(
                "--listen takes an address and a port, such as 127.0.0.1:{DEFAULT_PORT}, not \
                 `{address}`: {e}; usage: {SERVE_USAGE}"
            ))
        })?
        .collect();
    let listener = TcpListener::bind(&addresses[..]).map_err(|e| {
        Error::new(
            ErrorCode::ListenFailed,
            format!("could not listen on {address}: {e}"),
        )
    })?;
    let ip = listener
        .local_addr()
        .map_err(|e| Error::new(ErrorCode::ListenFailed, format!("could not listen: {e}")))?
        .ip();
    Ok((listener, Guard::for_address(ip)?))
}

/// Where `serve` listens: the address and port `--listen` names, or the
/// local machine's at [`DEFAULT_PORT`].
fn listen_address(mut args: Args) -> Result<String, Error> {
    let mut address = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--listen") => {
                let given = args
                    .next()
                    .and_then(|a| a.into_string().ok())
                    .ok_or_else(|| {
                        Error::usage(format!(
                        "--listen needs an address and a port, such as 127.0.0.1:{DEFAULT_PORT}; \
                         usage: {SERVE_USAGE}"
                    ))
                    })?;
                if address.replace(given).is_some() {
                    return Err(Error::usage("--listen is given twice"));
                }
            }
            _ => {
                return Err(Error::usage(format!(
                    "serve takes only --listen, not `{}`; usage: {SERVE_USAGE}",
                    arg.to_string_lossy()
                )))
            }
        }
    }
    Ok(address.unwrap_or_else(|| format!("127.0.0.1:{DEFAULT_PORT}")))
}

/// What a server lets in.
#[derive(Debug, PartialEq, Eq)]
enum Guard {
    /// It listens on the local machine only: a request needs no token, but
    /// is addressed to the local machine.
    Local,
    /// It listens beyond the local machine: every request carries this
    /// token.
    Token(String),
    /// In-process local desktop server: listens on loopback only, requires
    /// an exact `Host: 127.0.0.1:<port>` (and exact `Origin` when sent) plus
    /// this per-session bearer token, and serves the page with
    /// [`page::DESKTOP_POLICY`].
    DesktopToken { token: String, host: String },
}

impl Guard {
    /// Generates a fresh 32-byte random hexadecimal token.
    fn new_token() -> Result<String, Error> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| {
            Error::new(
                ErrorCode::ListenFailed,
                format!("could not make a token to guard the server: {e}"),
            )
        })?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// The guard a server listening on `ip` needs: none beyond the local
    /// machine's own for a loopback address, and a new token for any other,
    /// the unspecified address that listens on every network among them.
    fn for_address(ip: IpAddr) -> Result<Self, Error> {
        if is_local(ip) {
            return Ok(Self::Local);
        }
        Ok(Self::Token(Self::new_token()?))
    }

    fn is_desktop(&self) -> bool {
        matches!(self, Self::DesktopToken { .. })
    }
}

/// Whether `ip` is the local machine's own, such as 127.0.0.1 or ::1.
fn is_local(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

/// The line `serve` starts with: where it listens, the viewer's address,
/// and the token when it needs one. The viewer's address carries the token
/// in its fragment, which a browser never sends: this line is the only
/// place the server writes it.
fn started(address: SocketAddr, guard: &Guard) -> Value {
    let mut started = json!({
        "listening": format!("http://{address}"),
        "address": address.to_string(),
        "local_only": *guard == Guard::Local,
        "token_required": *guard != Guard::Local,
        "viewer": format!("http://{address}/"),
    });
    if let Guard::Token(token) = guard {
        started["viewer"] = json!(format!("http://{address}/#token={token}"));
        started["token"] = json!(token);
    }
    started
}

/// Takes one request on `stream`, answers it, and closes it. The request
/// must arrive within `limit`, and the reply be taken within it.
fn handle(
    mut stream: TcpStream,
    guard: &Guard,
    settings: &Settings,
    limit: Duration,
    stop: Option<Arc<AtomicBool>>,
) {
    let arrive_by = Deadline::after(limit);
    match respond(&mut stream, guard, settings, arrive_by) {
        Some(Answer::Reply(response)) => {
            let _ = Deadline::after(limit).write(&mut stream, &response.bytes());
        }
        Some(Answer::Watch(set)) => return watch(stream, set, settings.clone(), limit, stop),
        Some(Answer::Converse(set)) => return converse(stream, set, limit, stop),
        None => {}
    }
    // A refused request's body is not read: the connection is closed with
    // whatever the client is still sending.
    let _ = stream.shutdown(Shutdown::Write);
}

/// How a request is answered: with one response, or with an event stream
/// that watches a drawing set.
enum Answer {
    Reply(Response),
    Watch(PathBuf),
    /// A stream of the assistant's conversation on a set.
    Converse(PathBuf),
}

/// The answer to the request on `stream`, or none when the connection
/// closed, or fell silent, before a request came.
fn respond(
    stream: &mut TcpStream,
    guard: &Guard,
    settings: &Settings,
    arrive_by: Deadline,
) -> Option<Answer> {
    let head = match read_head(stream, arrive_by) {
        Ok(Some(head)) => head,
        Ok(None) => return None,
        Err(response) => return Some(Answer::Reply(response)),
    };
    // The viewer's page needs no token, but on the local machine is served
    // only to a request addressed to it.
    if let Some(file) = page::file(head.route()) {
        match guard {
            Guard::Local => {
                if let Err(response) = addressed_locally(&head) {
                    return Some(Answer::Reply(response));
                }
            }
            Guard::DesktopToken { host, .. } => {
                if let Err(response) = addressed_to_desktop(&head, host) {
                    return Some(Answer::Reply(response));
                }
            }
            Guard::Token(_) => {}
        }
        return Some(Answer::Reply(serve_page(&head, file, guard.is_desktop())));
    }
    if let Err(response) = admit(&head, guard) {
        return Some(Answer::Reply(response));
    }
    if head.route() == "/events" {
        return Some(match events(stream, head, arrive_by) {
            Ok(set) => Answer::Watch(set),
            Err(response) => Answer::Reply(response),
        });
    }
    if head.route() == "/assistant" || head.route().starts_with("/assistant/") {
        return Some(
            converse_route(stream, head, settings, arrive_by).unwrap_or_else(Answer::Reply),
        );
    }
    Some(Answer::Reply(
        route(stream, head, settings, arrive_by).unwrap_or_else(|response| response),
    ))
}

/// The drawing set a request for an event stream names, or why it is
/// refused.
fn events(stream: &mut TcpStream, head: Head, arrive_by: Deadline) -> Result<PathBuf, Response> {
    if head.method != "POST" {
        return Err(Response::usage(
            405,
            "POST /events watches the drawing set its body names, {\"set\": \"<folder>\"}",
        )
        .allow("POST"));
    }
    let body = read_body(stream, head, arrive_by)?;
    let named = serde_json::from_slice::<serde_json::Map<String, Value>>(&body)
        .ok()
        .filter(|body| body.len() == 1)
        .and_then(|body| body.get("set").and_then(Value::as_str).map(PathBuf::from));
    let Some(set) = named else {
        return Err(Response::usage(
            400,
            "the body of POST /events names the drawing set to watch, {\"set\": \"<folder>\"}, \
             and nothing else",
        ));
    };
    if WATCHERS.load(Ordering::SeqCst) >= MOST_WATCHERS {
        return Err(Response::usage(
            503,
            format!("at most {MOST_WATCHERS} event streams are open at once; close one first"),
        ));
    }
    Ok(set)
}

/// Answers a request to the drafter's assistant.
fn converse_route(
    stream: &mut TcpStream,
    head: Head,
    settings: &Settings,
    arrive_by: Deadline,
) -> Result<Answer, Response> {
    let route = head.route().to_owned();
    let post = |head: &Head| -> Result<(), Response> {
        if head.method != "POST" {
            return Err(
                Response::usage(405, format!("POST {route} with a JSON body")).allow("POST"),
            );
        }
        Ok(())
    };
    let ok = |result: Value| Response::reply(200, &Reply::from(Ok(result)));
    let refused =
        |(status, error): (u16, Error)| Response::reply(status, &Reply::<()>::from(Err(error)));
    let assistant = assistant();
    match route.as_str() {
        "/assistant" => {
            if head.method != "GET" {
                return Err(Response::usage(405, "GET /assistant lists the agents").allow("GET"));
            }
            Ok(Answer::Reply(ok(assistant.agents())))
        }
        "/assistant/find" => {
            post(&head)?;
            read_body(stream, head, arrive_by)?;
            Ok(Answer::Reply(ok(assistant.find_again())))
        }
        "/assistant/events" => {
            post(&head)?;
            let body = read_body(stream, head, arrive_by)?;
            let named = serde_json::from_slice::<serde_json::Map<String, Value>>(&body)
                .ok()
                .filter(|body| body.len() == 1)
                .and_then(|body| body.get("set").and_then(Value::as_str).map(PathBuf::from));
            let Some(set) = named else {
                return Err(Response::usage(
                    400,
                    "the body of POST /assistant/events names the drawing set, {\"set\": \"<folder>\"}, \
                     and nothing else",
                ));
            };
            if WATCHERS.load(Ordering::SeqCst) >= MOST_WATCHERS {
                return Err(Response::usage(
                    503,
                    format!(
                        "at most {MOST_WATCHERS} event streams are open at once; close one first"
                    ),
                ));
            }
            // A drawing set's alone, kept by its canonical path.
            match assistant.open(&set) {
                Ok(key) => Ok(Answer::Converse(key)),
                Err(refusal) => Ok(Answer::Reply(refused(refusal))),
            }
        }
        "/assistant/messages" => {
            post(&head)?;
            let body = read_body(stream, head, arrive_by)?;
            let message: Message = serde_json::from_slice(&body).map_err(|e| {
                Response::usage(
                    400,
                    format!(
                        "the body of POST /assistant/messages is {{\"set\", \"text\", \"via\", \
                         \"looking_at\"}}: {e}"
                    ),
                )
            })?;
            Ok(Answer::Reply(match assistant.send(message, settings) {
                Ok(result) => ok(result),
                Err(refusal) => refused(refusal),
            }))
        }
        "/assistant/agent" => {
            post(&head)?;
            let body = read_body(stream, head, arrive_by)?;
            let choice: Choice = serde_json::from_slice(&body).map_err(|e| {
                Response::usage(
                    400,
                    format!("the body of POST /assistant/agent is {{\"set\", \"agent\"}}: {e}"),
                )
            })?;
            Ok(Answer::Reply(match assistant.choose(&choice) {
                Ok(result) => ok(result),
                Err(refusal) => refused(refusal),
            }))
        }
        other => Err(Response::usage(
            404,
            format!(
                "no route `{other}`: GET /assistant lists the agents, and POST /assistant/find, \
                 /assistant/messages, /assistant/agent and /assistant/events reach the assistant"
            ),
        )),
    }
}

/// Streams the assistant's conversation on `set` on `stream`, on a thread
/// of its own: all of it when the stream opens, and again each time it
/// changes, until the page lets the stream go.
fn converse(mut stream: TcpStream, set: PathBuf, limit: Duration, stop: Option<Arc<AtomicBool>>) {
    WATCHERS.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new()
        .name("agentdrafter-assistant-events".into())
        .spawn(move || {
            let _ = stream_conversation(&mut stream, &set, limit, stop.as_deref());
            let _ = stream.shutdown(Shutdown::Both);
            WATCHERS.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        WATCHERS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn stream_conversation(
    stream: &mut TcpStream,
    set: &Path,
    limit: Duration,
    stop: Option<&AtomicBool>,
) -> io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
                X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n";
    Deadline::after(limit).write(stream, head.as_bytes())?;
    let assistant = assistant();
    let mut sent: Option<u64> = None;
    let mut said = Instant::now();
    loop {
        if stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
            return Ok(());
        }
        let (version, data) = assistant.snapshot(set);
        if sent != Some(version) {
            let event = format!("event: assistant\ndata: {data}\n\n");
            Deadline::after(limit).write(stream, event.as_bytes())?;
            sent = Some(version);
            said = Instant::now();
        } else if said.elapsed() >= KEEP_ALIVE {
            Deadline::after(limit).write(stream, b": still watching\n\n")?;
            said = Instant::now();
        }
        if closed(stream) || stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
            return Ok(());
        }
        assistant.wait(set, version, WATCH_EVERY * 5);
    }
}

/// Streams the journal of the drawing set or original DWG in `set` on
/// `stream`, on a thread of its own: the `journal` command's reply as an
/// event when the stream opens, and again each time the journal changes on
/// disk, until the page lets the stream go.
fn watch(
    mut stream: TcpStream,
    set: PathBuf,
    settings: Settings,
    limit: Duration,
    stop: Option<Arc<AtomicBool>>,
) {
    WATCHERS.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new()
        .name("agentdrafter-events".into())
        .spawn(move || {
            let _ = stream_journal(&mut stream, &set, &settings, limit, stop.as_deref());
            let _ = stream.shutdown(Shutdown::Both);
            WATCHERS.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        WATCHERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What `POST /events` is watching on disk.
enum WatchKind {
    /// A drawing set folder: watches `<set>/.agentdrafter/journal.jsonl`,
    /// proposals and guides.
    Set(PathBuf),
    /// An original `.dwg` file: watches its per-original `journal.jsonl`
    /// resolved by the core's own [`original_journal_file`] helper.
    OriginalDwg(PathBuf),
    /// Anything else (such as a single `.yaml` sheet description or a
    /// missing path): sends the initial `journal` reply once and idles
    /// cleanly without error spam.
    Other,
}

fn watch_kind(set: &Path) -> WatchKind {
    if set.is_dir() {
        WatchKind::Set(set.join(JOURNAL_FILE))
    } else if let Some(journal_path) = original_journal_file(set) {
        WatchKind::OriginalDwg(journal_path)
    } else {
        WatchKind::Other
    }
}

fn stream_journal(
    stream: &mut TcpStream,
    set: &Path,
    settings: &Settings,
    limit: Duration,
    stop: Option<&AtomicBool>,
) -> io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n\
                X-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n";
    Deadline::after(limit).write(stream, head.as_bytes())?;
    let journal = find_command("journal").expect("the registry holds `journal`");
    let proposals = find_command("proposals").expect("the registry holds `proposals`");
    let guides = find_command("guides").expect("the registry holds `guides`");
    let kind = watch_kind(set);
    type Seen = (Option<(u64, SystemTime)>, Vec<String>, Vec<String>);
    let mut seen: Option<Seen> = None;
    let (mut sent, mut sent_proposals): (Option<String>, Option<String>) = (None, None);
    let mut sent_guides: Option<String> = None;
    let mut said = Instant::now();
    loop {
        if stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
            return Ok(());
        }
        // A proposal adds a file beside the journal; a keep or a discard
        // adds to the journal, which ends it.
        let now = match &kind {
            WatchKind::Set(journal_file) => (
                file_state(journal_file),
                listed_state(set, PROPOSALS_FOLDER),
                listed_state(set, GUIDES_FOLDER),
            ),
            WatchKind::OriginalDwg(journal_file) => {
                (file_state(journal_file), Vec::new(), Vec::new())
            }
            WatchKind::Other => (None, Vec::new(), Vec::new()),
        };
        if seen.as_ref() != Some(&now) {
            let journal_moved = seen.as_ref().is_none_or(|s| s.0 != now.0);
            let guides_moved = journal_moved || seen.as_ref().is_none_or(|s| s.2 != now.2);
            seen = Some(now);
            let input = serde_json::json!({ "set": set });
            if journal_moved {
                let reply = journal.run(input.clone(), settings, Interface::Http);
                // A reply holds only strings, numbers, lists and
                // string-keyed maps, which always serialize, and JSON
                // escapes every line break, so it is one line of data.
                let data = serde_json::to_string(&reply).expect("a reply serializes to JSON");
                // A change set's entry is written pending and then marked
                // done: a look between the two waits for the commit, and its
                // reply already holds the entry done, so the second says
                // nothing new.
                if sent.as_ref() != Some(&data) {
                    let event = format!("event: journal\ndata: {data}\n\n");
                    Deadline::after(limit).write(stream, event.as_bytes())?;
                    sent = Some(data);
                    said = Instant::now();
                }
            }
            if matches!(kind, WatchKind::Set(_)) {
                let reply = proposals.run(input.clone(), settings, Interface::Http);
                let data = serde_json::to_string(&reply).expect("a reply serializes to JSON");
                if sent_proposals.as_ref() != Some(&data) {
                    let event = format!("event: proposals\ndata: {data}\n\n");
                    Deadline::after(limit).write(stream, event.as_bytes())?;
                    sent_proposals = Some(data);
                    said = Instant::now();
                }
                // A guide's steps are worked out on the set as it stands, so a
                // change set moves them on as a new guide does.
                if guides_moved {
                    let reply = guides.run(input, settings, Interface::Http);
                    let data = serde_json::to_string(&reply).expect("a reply serializes to JSON");
                    if sent_guides.as_ref() != Some(&data) {
                        let event = format!("event: guides\ndata: {data}\n\n");
                        Deadline::after(limit).write(stream, event.as_bytes())?;
                        sent_guides = Some(data);
                        said = Instant::now();
                    }
                }
            }
        } else if said.elapsed() >= KEEP_ALIVE {
            Deadline::after(limit).write(stream, b": still watching\n\n")?;
            said = Instant::now();
        }
        if closed(stream) || stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
            return Ok(());
        }
        std::thread::sleep(WATCH_EVERY);
    }
}

/// The files of the set's proposals, or its guides, in `folder`, which
/// change when one is proposed or given; none when it has none.
fn listed_state(set: &Path, folder: &str) -> Vec<String> {
    let Ok(listing) = fs::read_dir(set.join(folder)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = listing
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    names
}

/// A journal file's length and when it was last written, which change when an
/// entry is added; none when there is no journal file yet.
fn file_state(path: &Path) -> Option<(u64, SystemTime)> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// Whether the client has closed the connection.
fn closed(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let mut byte = [0u8; 1];
    let gone = match stream.peek(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != ErrorKind::WouldBlock,
    };
    let _ = stream.set_nonblocking(false);
    gone
}

/// Which interface a request let in came through: the viewer, when the
/// browser says its page sent it to its own server, which no script can
/// say for it, or else the HTTP API.
fn interface_of(head: &Head) -> Interface {
    match head.header("sec-fetch-site") {
        Some(site) if site.trim().eq_ignore_ascii_case("same-origin") => Interface::Viewer,
        _ => Interface::Http,
    }
}

/// One of the viewer's page's files, to a GET, or its headers alone to a
/// HEAD.
fn serve_page(head: &Head, file: page::File, desktop: bool) -> Response {
    if !matches!(head.method.as_str(), "GET" | "HEAD") {
        return Response::usage(405, "the viewer's page is had with GET").allow("GET, HEAD");
    }
    Response {
        status: 200,
        body: file.bytes.to_vec(),
        content_type: file.content_type,
        cache: if file.fingerprinted {
            "public, max-age=31536000, immutable"
        } else {
            "no-store"
        },
        page: file.content_type.starts_with("text/html"),
        desktop,
        headers_only: head.method == "HEAD",
        allow: None,
    }
}

/// A request's line and headers, and what was read of its body with them.
struct Head {
    method: String,
    path: String,
    /// Each header, its name in lower case.
    headers: Vec<(String, String)>,
    body_start: Vec<u8>,
}

impl Head {
    /// The request's path without its query.
    fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or_default()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Reads a request's line and headers, or nothing when the connection
/// closes, or sends nothing by the deadline.
fn read_head(stream: &mut TcpStream, arrive_by: Deadline) -> Result<Option<Head>, Response> {
    let mut read = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&read) {
            Ok(httparse::Status::Complete(length)) => {
                let headers: Vec<(String, String)> = request
                    .headers
                    .iter()
                    .map(|h| {
                        (
                            h.name.to_ascii_lowercase(),
                            String::from_utf8_lossy(h.value).into_owned(),
                        )
                    })
                    .collect();
                for once in ["host", "authorization", "content-length", "content-type"] {
                    if headers.iter().filter(|(n, _)| n == once).count() > 1 {
                        return Err(Response::usage(
                            400,
                            format!("a request gives its {once} header once"),
                        ));
                    }
                }
                return Ok(Some(Head {
                    method: request.method.unwrap_or_default().to_owned(),
                    path: request.path.unwrap_or_default().to_owned(),
                    headers,
                    body_start: read[length..].to_vec(),
                }));
            }
            Ok(httparse::Status::Partial) => {}
            Err(httparse::Error::TooManyHeaders) => {
                return Err(Response::usage(431, "a request has at most 64 headers"))
            }
            Err(e) => {
                return Err(Response::usage(
                    400,
                    format!("the request is not HTTP: {e}"),
                ))
            }
        }
        if read.len() >= MOST_HEAD {
            return Err(Response::usage(
                431,
                format!("a request's line and headers hold at most {MOST_HEAD} bytes"),
            ));
        }
        match arrive_by.read(stream, &mut chunk) {
            Ok(0) if read.is_empty() => return Ok(None),
            Ok(0) => {
                return Err(Response::usage(
                    400,
                    "the request ended before its headers did",
                ))
            }
            Ok(n) => read.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) if read.is_empty() => return Ok(None),
            Err(_) => return Err(arrive_by.late()),
        }
    }
}

/// Lets the request through `guard`, or refuses it.
fn admit(head: &Head, guard: &Guard) -> Result<(), Response> {
    match guard {
        Guard::Token(token) => {
            let given = head.header("authorization").and_then(bearer);
            let right =
                given.is_some_and(|given| bool::from(given.as_bytes().ct_eq(token.as_bytes())));
            if !right {
                return Err(Response::refused(
                    401,
                    "this server listens beyond the local machine, so every request carries the \
                     token it printed when it started, as Authorization: Bearer <token>",
                ));
            }
        }
        Guard::DesktopToken { token, host } => {
            addressed_to_desktop(head, host)?;
            let given = head.header("authorization").and_then(bearer);
            let right =
                given.is_some_and(|given| bool::from(given.as_bytes().ct_eq(token.as_bytes())));
            if !right {
                return Err(Response::refused(
                    401,
                    "this local desktop server requires its session token as Authorization: \
                     Bearer <token>",
                ));
            }
        }
        Guard::Local => addressed_locally(head)?,
    }
    Ok(())
}

/// Requires an exact `Host: 127.0.0.1:<port>` header (never accepting a missing
/// `Host` or any other port/host) and, when `Origin` is present, an exact
/// `http://127.0.0.1:<port>` origin.
fn addressed_to_desktop(head: &Head, expected_host: &str) -> Result<(), Response> {
    let Some(host) = head.header("host").map(str::trim) else {
        return Err(Response::refused(
            403,
            format!("this local desktop server requires Host: {expected_host}"),
        ));
    };
    if !host.eq_ignore_ascii_case(expected_host) {
        return Err(Response::refused(
            403,
            format!(
                "this local desktop server takes requests addressed to `{expected_host}` only, \
                 not `{host}`"
            ),
        ));
    }
    if let Some(origin) = head.header("origin").map(str::trim) {
        let expected_origin = format!("http://{expected_host}");
        if !origin.eq_ignore_ascii_case(&expected_origin) {
            return Err(Response::refused(
                403,
                format!(
                    "this local desktop server takes requests from `{expected_origin}` only, not \
                     from `{origin}`"
                ),
            ));
        }
    }
    Ok(())
}

/// Lets a request addressed to the local machine through, by its `Host` and
/// any `Origin`, or refuses it.
fn addressed_locally(head: &Head) -> Result<(), Response> {
    if let Some(host) = head.header("host") {
        if !is_local_name(host_name(host)) {
            return Err(Response::refused(
                403,
                format!(
                    "this server listens on the local machine only and takes requests addressed \
                     to it, as localhost, 127.0.0.1 or [::1], not `{host}`"
                ),
            ));
        }
    }
    if let Some(origin) = head.header("origin") {
        let local = ["http://", "https://"].iter().any(|scheme| {
            origin
                .strip_prefix(scheme)
                .is_some_and(|rest| is_local_name(host_name(rest)))
        });
        if !local {
            return Err(Response::refused(
                403,
                format!(
                    "this server listens on the local machine only and takes requests from pages \
                     of the local machine only, not from `{origin}`"
                ),
            ));
        }
    }
    Ok(())
}

/// The token an `Authorization: Bearer <token>` header carries.
fn bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.trim().split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

/// The name in a `Host` header, or an origin's after its scheme, without
/// its port: `localhost` of `localhost:7316`, `::1` of `[::1]:7316`.
fn host_name(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    }
}

/// Whether `name` is the local machine: `localhost` or a loopback address.
fn is_local_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok_and(is_local)
}

/// Answers a request that was let in.
fn route(
    stream: &mut TcpStream,
    head: Head,
    settings: &Settings,
    arrive_by: Deadline,
) -> Result<Response, Response> {
    let path = head.path.split('?').next().unwrap_or_default();
    if path == "/commands" {
        if head.method != "GET" {
            return Err(Response::usage(405, "GET /commands lists the commands").allow("GET"));
        }
        let listed: Vec<_> = commands().iter().map(|command| command.info()).collect();
        return Ok(Response::reply(
            200,
            &Reply::from(Ok(json!({ "commands": listed }))),
        ));
    }
    let Some(name) = path.strip_prefix("/commands/") else {
        return Err(Response::usage(
            404,
            format!(
                "no route `{path}`: GET /commands lists the commands, POST /commands/<name> \
                 runs one, POST /events watches a drawing set's journal, GET /assistant and the \
                 routes under it reach the drafter's assistant, and GET / is the viewer"
            ),
        ));
    };
    let Some(command) = find_command(name) else {
        let names: Vec<&str> = commands().iter().map(|command| command.name()).collect();
        return Err(Response::usage(
            404,
            format!("no command `{name}`; the commands are {}", names.join(", ")),
        ));
    };
    if head.method != "POST" {
        return Err(Response::usage(
            405,
            format!("POST /commands/{name} runs `{name}`, its body the command's input"),
        )
        .allow("POST"));
    }
    let via = interface_of(&head);
    let body = read_body(stream, head, arrive_by)?;
    let input = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| Response::usage(400, format!("the body is not JSON: {e}")))?
    };
    let reply = command.run(input, settings, via);
    let status = match &reply.error {
        _ if reply.ok => 200,
        Some(error) if error.code == ErrorCode::Usage => 400,
        _ => 422,
    };
    Ok(Response::reply(status, &reply))
}

/// Reads a command's body, which is JSON and sized by `Content-Length`.
fn read_body(stream: &mut TcpStream, head: Head, arrive_by: Deadline) -> Result<Vec<u8>, Response> {
    if head.header("transfer-encoding").is_some() {
        return Err(Response::usage(
            411,
            "a body is sized by Content-Length; a chunked body is not taken",
        ));
    }
    let length = match head.header("content-length") {
        None => 0,
        Some(length) => length
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|_| length.trim().bytes().all(|b| b.is_ascii_digit()))
            .ok_or_else(|| {
                Response::usage(
                    400,
                    format!("Content-Length is a number of bytes, not `{length}`"),
                )
            })?,
    };
    if length > 0 {
        let json = head.header("content-type").is_some_and(|kind| {
            kind.split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
        });
        if !json {
            return Err(Response::usage(
                415,
                "a command's input is sent as JSON, with Content-Type: application/json",
            ));
        }
    }
    if length > MOST_BODY {
        return Err(Response::usage(
            413,
            format!("a command's input holds at most {MOST_BODY} bytes"),
        ));
    }
    let mut body = head.body_start;
    body.truncate(length);
    let mut chunk = vec![0u8; 64 * 1024];
    while body.len() < length {
        let wanted = (length - body.len()).min(chunk.len());
        match arrive_by.read(stream, &mut chunk[..wanted]) {
            Ok(0) => {
                return Err(Response::usage(
                    400,
                    format!("the body ended before its Content-Length of {length} bytes"),
                ))
            }
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Err(arrive_by.late()),
        }
    }
    Ok(body)
}

/// A response: its status and what it carries, a JSON reply or one of the
/// viewer's page's files.
struct Response {
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
    /// Its Cache-Control.
    cache: &'static str,
    /// Whether it is the viewer's page, which is sent with what the page
    /// may do.
    page: bool,
    /// Whether this response comes from the in-process desktop shell server.
    desktop: bool,
    /// Whether it answers a HEAD: its headers, without the body they size.
    headers_only: bool,
    allow: Option<&'static str>,
}

impl Response {
    fn reply<T: serde::Serialize>(status: u16, reply: &Reply<T>) -> Self {
        Self {
            status,
            // A reply holds only strings, numbers, lists and string-keyed
            // maps, which always serialize.
            body: serde_json::to_vec(reply).expect("a reply serializes to JSON"),
            content_type: "application/json",
            cache: "no-store",
            page: false,
            desktop: false,
            headers_only: false,
            allow: None,
        }
    }

    /// A request that is wrong, refused before it reached a command.
    fn usage(status: u16, message: impl Into<String>) -> Self {
        Self::reply(status, &Reply::<()>::from(Err(Error::usage(message))))
    }

    /// A request the guard does not let in.
    fn refused(status: u16, message: impl Into<String>) -> Self {
        let error = Error::new(ErrorCode::Unauthorized, message);
        Self::reply(status, &Reply::<()>::from(Err(error)))
    }

    fn allow(mut self, methods: &'static str) -> Self {
        self.allow = Some(methods);
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            409 => "Conflict",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            408 => "Request Timeout",
            411 => "Length Required",
            413 => "Content Too Large",
            415 => "Unsupported Media Type",
            422 => "Unprocessable Content",
            431 => "Request Header Fields Too Large",
            503 => "Service Unavailable",
            _ => "Error",
        };
        let mut head = format!(
            "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
             Cache-Control: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
            self.status,
            self.content_type,
            self.body.len(),
            self.cache,
        );
        if let Some(methods) = self.allow {
            head.push_str(&format!("Allow: {methods}\r\n"));
        }
        if self.status == 401 {
            head.push_str("WWW-Authenticate: Bearer\r\n");
        }
        if self.page {
            let policy = if self.desktop {
                page::DESKTOP_POLICY
            } else {
                page::POLICY
            };
            head.push_str(&format!(
                "Content-Security-Policy: {policy}\r\nReferrer-Policy: no-referrer\r\n\
                 X-Frame-Options: DENY\r\n",
            ));
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        if !self.headers_only {
            bytes.extend_from_slice(&self.body);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::thread;

    use super::*;

    #[test]
    fn serve_listens_on_the_local_machine_at_the_default_port_unless_told_otherwise() {
        let address = listen_address(Vec::new().into_iter()).unwrap();
        assert_eq!(address, format!("127.0.0.1:{DEFAULT_PORT}"));
        let ip: SocketAddr = address.parse().unwrap();
        assert_eq!(Guard::for_address(ip.ip()).unwrap(), Guard::Local);

        let told = listen_address(vec!["--listen".into(), "0.0.0.0:8080".into()].into_iter());
        assert_eq!(told.unwrap(), "0.0.0.0:8080");
        for wrong in [
            &["--listen"][..],
            &["--listen", "a:1", "--listen", "b:2"],
            &["--port", "1"],
        ] {
            let args: Vec<_> = wrong.iter().map(Into::into).collect();
            let error = listen_address(args.into_iter()).unwrap_err();
            assert_eq!(error.code, ErrorCode::Usage, "{wrong:?}");
        }
    }

    #[test]
    fn only_a_loopback_address_listens_without_a_token() {
        for local in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            "::ffff:127.0.0.1".parse().unwrap(),
        ] {
            assert_eq!(Guard::for_address(local).unwrap(), Guard::Local, "{local}");
        }
        let mut tokens = Vec::new();
        for beyond in [
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 7)),
            "fd00::7".parse().unwrap(),
        ] {
            let Guard::Token(token) = Guard::for_address(beyond).unwrap() else {
                panic!("{beyond} needs a token");
            };
            assert_eq!(token.len(), 64, "32 random bytes in hexadecimal");
            assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
            tokens.push(token);
        }
        tokens.sort();
        tokens.dedup();
        assert_eq!(tokens.len(), 5, "a new token each time");

        let address: SocketAddr = "0.0.0.0:7316".parse().unwrap();
        let guard = Guard::Token("t".repeat(64));
        let line = started(address, &guard);
        assert_eq!(line["token"], "t".repeat(64));
        assert_eq!(line["token_required"], true);
        assert_eq!(line["local_only"], false);
        // The viewer's address carries the token in its fragment, which a
        // browser never sends.
        assert_eq!(
            line["viewer"],
            format!("http://0.0.0.0:7316/#token={}", "t".repeat(64))
        );
        let line = started("127.0.0.1:7316".parse().unwrap(), &Guard::Local);
        assert_eq!(line["listening"], "http://127.0.0.1:7316");
        assert_eq!(line["viewer"], "http://127.0.0.1:7316/");
        assert_eq!(line["token_required"], false);
        assert!(line.get("token").is_none());
    }

    /// Serves `requests`, each raw bytes on its own connection, with
    /// `guard`, on a loopback port of its own, and returns each response's
    /// status, head and body as they came. The server stops when the last
    /// is answered.
    fn exchange_raw(guard: Guard, requests: Vec<Vec<u8>>) -> Vec<(u16, String, Vec<u8>)> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        assert!(is_local(address.ip()));
        let count = requests.len();
        let server = thread::spawn(move || {
            let settings = Settings::default();
            for _ in 0..count {
                let (stream, _) = listener.accept().unwrap();
                handle(stream, &guard, &settings, DEADLINE, None);
            }
        });
        let responses = requests
            .into_iter()
            .map(|request| {
                let mut stream = TcpStream::connect(address).unwrap();
                stream.write_all(&request).unwrap();
                let mut response = Vec::new();
                stream.read_to_end(&mut response).unwrap();
                let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
                let head = String::from_utf8(response[..split].to_vec()).unwrap();
                let status = head.split(' ').nth(1).unwrap().parse().unwrap();
                (status, head, response[split + 4..].to_vec())
            })
            .collect();
        server.join().unwrap();
        responses
    }

    fn get(method: &str, path: &str, host: &str, more: &str) -> Vec<u8> {
        format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n{more}\r\n").into_bytes()
    }

    #[test]
    fn beyond_the_local_machine_the_viewers_page_needs_no_token_but_commands_do() {
        let token = "c".repeat(64);
        let host = "100.64.0.7:7316";
        let responses = exchange_raw(
            Guard::Token(token.clone()),
            vec![
                get("GET", "/", host, ""),
                get("GET", "/index.html?source=demo-plant", "forge.example", ""),
                get("HEAD", "/", host, ""),
                get("POST", "/", host, "Content-Length: 0\r\n"),
                get("GET", "/commands", host, ""),
                get("GET", "/nowhere", host, ""),
                get("GET", "/../Cargo.toml", host, ""),
                get(
                    "GET",
                    "/commands",
                    host,
                    &format!("Authorization: Bearer {token}\r\n"),
                ),
            ],
        );
        let statuses: Vec<u16> = responses.iter().map(|(s, _, _)| *s).collect();
        assert_eq!(statuses, [200, 200, 200, 405, 401, 401, 401, 200]);
        let (_, head, body) = &responses[0];
        assert!(
            head.contains("Content-Type: text/html; charset=utf-8"),
            "{head}"
        );
        assert!(head.contains("Cache-Control: no-store"), "{head}");
        assert!(head.contains(&format!("Content-Security-Policy: {}", page::POLICY)));
        assert!(head.contains("Referrer-Policy: no-referrer"));
        assert_eq!(body.as_slice(), page::file("/").unwrap().bytes);
        assert_eq!(responses[1].2, responses[0].2, "the page has its query");
        // A HEAD gives the headers, sized as a GET's body, and no body.
        let (_, head, body) = &responses[2];
        assert!(head.contains(&format!("Content-Length: {}", responses[0].2.len())));
        assert!(body.is_empty());
        assert!(responses[3].1.contains("Allow: GET, HEAD"));
        for (_, _, body) in &responses[4..7] {
            let json: Value = serde_json::from_slice(body).unwrap();
            assert_eq!(json["error"]["code"], "unauthorized");
        }
    }

    #[test]
    fn each_of_the_pages_files_is_served_with_its_type() {
        let requests = page::VIEWER_FILES
            .iter()
            .map(|(route, _, _)| get("GET", route, "127.0.0.1", ""))
            .collect();
        let responses = exchange_raw(Guard::Token("d".repeat(64)), requests);
        for ((route, content_type, bytes), (status, head, body)) in
            page::VIEWER_FILES.iter().zip(&responses)
        {
            assert_eq!(*status, 200, "{route}");
            assert!(
                head.contains(&format!("Content-Type: {content_type}\r\n")),
                "{head}"
            );
            assert_eq!(body.as_slice(), *bytes, "{route}");
            let fingerprinted = page::VIEWER_BUILT && route.starts_with("/assets/");
            assert_eq!(
                head.contains("Cache-Control: public, max-age=31536000, immutable"),
                fingerprinted,
                "{route}"
            );
        }
    }

    #[test]
    fn on_the_local_machine_the_page_is_served_only_to_requests_addressed_to_it() {
        let responses = exchange_raw(
            Guard::Local,
            vec![
                get("GET", "/", "localhost:7316", ""),
                get("GET", "/", "[::1]:7316", ""),
                get("GET", "/", "rebound.example:7316", ""),
                get(
                    "GET",
                    "/",
                    "127.0.0.1",
                    "Origin: https://elsewhere.example\r\n",
                ),
            ],
        );
        let statuses: Vec<u16> = responses.iter().map(|(s, _, _)| *s).collect();
        assert_eq!(statuses, [200, 200, 403, 403]);
    }

    /// Serves `requests` as [`exchange_raw`] does, and returns each
    /// response's status, head and JSON.
    fn exchange(guard: Guard, requests: Vec<Vec<u8>>) -> Vec<(u16, String, Value)> {
        exchange_raw(guard, requests)
            .into_iter()
            .map(|(status, head, body)| (status, head, serde_json::from_slice(&body).unwrap()))
            .collect()
    }

    fn post(name: &str, headers: &str, body: &str) -> Vec<u8> {
        format!(
            "POST /commands/{name} HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    const JSON: &str = "Content-Type: application/json\r\n";

    #[test]
    fn beyond_the_local_machine_a_request_without_the_token_is_refused() {
        let token = "0123456789abcdef".repeat(4);
        let right = format!("{JSON}Authorization: Bearer {token}\r\n");
        let wrong = format!("{JSON}Authorization: Bearer {}\r\n", "f".repeat(64));
        let short = format!("{JSON}Authorization: Bearer {}\r\n", &token[..63]);
        let basic = format!("{JSON}Authorization: Basic {token}\r\n");
        let responses = exchange(
            Guard::Token(token.clone()),
            vec![
                post("operations", JSON, "{}"),
                post("operations", &wrong, "{}"),
                post("operations", &short, "{}"),
                post("operations", &basic, "{}"),
                b"GET /commands HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                post("operations", &right, "{}"),
                format!("GET /commands HTTP/1.1\r\nHost: drafting.example\r\nAuthorization: bearer {token}\r\n\r\n").into_bytes(),
            ],
        );
        for (status, head, json) in &responses[..5] {
            assert_eq!(*status, 401, "{json}");
            assert!(head.contains("WWW-Authenticate: Bearer"), "{head}");
            assert_eq!(json["ok"], false);
            assert_eq!(json["error"]["code"], "unauthorized");
            assert!(json["error"]["message"].as_str().unwrap().contains("token"));
        }
        let (status, _, json) = &responses[5];
        assert_eq!(*status, 200, "{json}");
        assert_eq!(json["ok"], true);
        assert!(json["result"]["operations"].is_array());
        let (status, _, json) = &responses[6];
        assert_eq!(*status, 200, "with the token, any name reaches it: {json}");
        assert!(json["result"]["commands"].is_array());
    }

    #[test]
    fn on_the_local_machine_a_request_needs_no_token_but_is_addressed_to_it() {
        let responses = exchange(
            Guard::Local,
            vec![
                post("operations", JSON, "{}"),
                post("operations", "", ""),
                b"GET /commands HTTP/1.1\r\nHost: localhost:7316\r\n\r\n".to_vec(),
                b"GET /commands HTTP/1.1\r\nHost: [::1]:7316\r\n\r\n".to_vec(),
                b"GET /commands HTTP/1.1\r\nHost: rebound.example:7316\r\n\r\n".to_vec(),
                post(
                    "operations",
                    &format!("{JSON}Origin: https://elsewhere.example\r\n"),
                    "{}",
                ),
                post(
                    "operations",
                    &format!("{JSON}Origin: http://localhost:5173\r\n"),
                    "{}",
                ),
                post("operations", "Content-Type: text/plain\r\n", "{}"),
            ],
        );
        let statuses: Vec<u16> = responses.iter().map(|(s, _, _)| *s).collect();
        assert_eq!(statuses, [200, 200, 200, 200, 403, 403, 200, 415]);
        for (_, _, json) in &responses[4..6] {
            assert_eq!(json["error"]["code"], "unauthorized");
        }
        assert_eq!(responses[7].2["error"]["code"], "usage");
    }

    #[test]
    fn a_request_is_read_plainly_and_a_wrong_one_gets_a_json_error() {
        let responses = exchange(
            Guard::Local,
            vec![
                b"GET /nowhere HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                post("draw", JSON, "{}"),
                b"GET /commands/operations HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                b"DELETE /commands HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                b"POST /commands/operations HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n".to_vec(),
                post("operations", JSON, "{not json"),
                post("operations", JSON, "[]"),
                post("operations", JSON, r#"{"extra": 1}"#),
                post("journal", JSON, r#"{"set": "no-such-set"}"#),
                b"POST /commands/operations HTTP/1.1\r\nHost: 127.0.0.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec(),
                b"POST /commands/operations HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 99999999999\r\n\r\n".to_vec(),
                b"not http at all\r\n\r\n".to_vec(),
            ],
        );
        let statuses: Vec<u16> = responses.iter().map(|(s, _, _)| *s).collect();
        assert_eq!(
            statuses,
            [404, 404, 405, 405, 411, 400, 400, 400, 422, 400, 413, 400]
        );
        assert!(responses[2].1.contains("Allow: POST"));
        assert!(responses[3].1.contains("Allow: GET"));
        for (status, head, json) in &responses {
            assert!(head.contains("Content-Type: application/json"), "{head}");
            assert_eq!(json["ok"], false, "{status}: {json}");
            let code = json["error"]["code"].as_str().unwrap();
            assert_eq!(code == "usage", *status != 422, "{status}: {json}");
        }
        assert!(responses[1].2["error"]["message"]
            .as_str()
            .unwrap()
            .contains("the commands are build, schema"));
    }

    /// Sends `start` on a new connection, then a byte every 50 ms for up to
    /// 10 seconds, and returns how long the server held the connection
    /// open and what it answered, if its answer was not lost to the close.
    fn trickle(address: SocketAddr, start: &[u8]) -> (Duration, Vec<u8>) {
        let mut stream = TcpStream::connect(address).unwrap();
        let began = Instant::now();
        stream.write_all(start).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let sender = thread::spawn(move || {
            for _ in 0..200 {
                thread::sleep(Duration::from_millis(50));
                if writer.write_all(b"a").is_err() {
                    break;
                }
            }
        });
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer);
        let held = began.elapsed();
        let _ = stream.shutdown(Shutdown::Both);
        sender.join().unwrap();
        (held, answer)
    }

    #[test]
    fn a_slow_client_is_cut_off_at_the_deadline_and_the_next_request_is_served() {
        let limit = Duration::from_secs(1);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let token = "a".repeat(64);
        let guard = Guard::Token(token.clone());
        let server = thread::spawn(move || {
            let settings = Settings::default();
            for _ in 0..4 {
                let (stream, _) = listener.accept().unwrap();
                handle(stream, &guard, &settings, limit, None);
            }
        });
        let head = "POST /commands/operations HTTP/1.1\r\nHost: 127.0.0.1\r\n";
        let auth = format!("Authorization: Bearer {token}\r\n");
        for (start, status, most) in [
            // Headers that never end, and a body that never does.
            (
                format!("{head}{auth}X-Slow: "),
                "408",
                limit + Duration::from_millis(1500),
            ),
            (
                format!("{head}{auth}{JSON}Content-Length: 100000\r\n\r\n"),
                "408",
                limit + Duration::from_millis(1500),
            ),
            // A refused request is closed at once, its body unread.
            (
                format!("{head}{JSON}Content-Length: 100000000\r\n\r\n"),
                "401",
                limit / 2,
            ),
        ] {
            let (held, answer) = trickle(address, start.as_bytes());
            assert!(held < most, "held {held:?} by {start:?}");
            let answer = String::from_utf8_lossy(&answer);
            assert!(
                answer.is_empty() || answer.starts_with(&format!("HTTP/1.1 {status} ")),
                "{answer}"
            );
        }
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .write_all(&post("operations", &format!("{JSON}{auth}"), "{}"))
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        assert!(
            answer.starts_with("HTTP/1.1 200 "),
            "the next request is served: {answer}"
        );
        server.join().unwrap();
    }

    /// A request for the event stream, with `headers`.
    fn events_request(headers: &str, body: &str) -> Vec<u8> {
        format!(
            "POST /events HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn beyond_the_local_machine_the_event_stream_needs_the_token_as_every_command_does() {
        let token = "e".repeat(64);
        let right = format!("{JSON}Authorization: Bearer {token}\r\n");
        let responses = exchange(
            Guard::Token(token.clone()),
            vec![
                events_request(JSON, r#"{"set": "demo-set"}"#),
                events_request(
                    &format!("{JSON}Authorization: Bearer {}\r\n", "f".repeat(64)),
                    r#"{"set": "demo-set"}"#,
                ),
                events_request(&right, r#"{"folder": "demo-set"}"#),
                events_request(&right, r#"{"set": "demo-set", "more": 1}"#),
                events_request("Content-Type: text/plain\r\n", r#"{"set": "demo-set"}"#),
                format!("GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n")
                    .into_bytes(),
            ],
        );
        for (status, _, json) in &responses[..2] {
            assert_eq!(*status, 401, "{json}");
            assert_eq!(json["error"]["code"], "unauthorized");
        }
        for (status, _, json) in &responses[2..4] {
            assert_eq!(*status, 400, "{json}");
            assert!(json["error"]["message"]
                .as_str()
                .unwrap()
                .contains("{\"set\""));
        }
        assert_eq!(responses[4].0, 401, "no token, whatever the body");
        assert_eq!(responses[5].0, 405, "the stream is asked for with POST");

        // With the token, the stream opens and its first event is the
        // journal command's reply on the set named.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let guard = Guard::Token(token);
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle(stream, &guard, &Settings::default(), DEADLINE, None);
        });
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .write_all(&events_request(&right, r#"{"set": "no-such-set"}"#))
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let mut read = String::new();
        let mut chunk = [0u8; 4096];
        while !read.contains("data: ") || !read.ends_with("\n\n") {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0, "the stream stays open: {read}");
            read.push_str(std::str::from_utf8(&chunk[..n]).unwrap());
        }
        assert!(read.starts_with("HTTP/1.1 200 OK\r\n"), "{read}");
        assert!(
            read.contains("Content-Type: text/event-stream\r\n"),
            "{read}"
        );
        let data = read
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap();
        let reply: Value = serde_json::from_str(data).unwrap();
        assert_eq!(reply["ok"], false, "no set there: {reply}");
        assert_eq!(reply["error"]["code"], "read_failed");
        drop(stream);
        server.join().unwrap();
    }

    #[test]
    fn a_request_the_viewers_page_sends_is_the_viewers_and_any_other_the_http_apis() {
        let head = |headers: &[(&str, &str)]| Head {
            method: "POST".into(),
            path: "/commands/accept".into(),
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            body_start: Vec::new(),
        };
        assert_eq!(
            interface_of(&head(&[("sec-fetch-site", "same-origin")])),
            Interface::Viewer
        );
        for other in [
            &[][..],
            &[("sec-fetch-site", "cross-site")],
            &[("sec-fetch-site", "none")],
            &[("origin", "http://127.0.0.1:7316")],
        ] {
            assert_eq!(interface_of(&head(other)), Interface::Http, "{other:?}");
        }
    }

    #[test]
    fn the_server_reaches_the_core_only_through_its_registry() {
        // No drawing logic lives in a server (ADR-0007): all it takes from
        // the core is the registry, the reply and its error, the settings
        // and the interface it hands on, and, for the HTTP API's event
        // stream, where a set keeps its journal and its proposals, which it
        // watches; and, for the MCP server the drafter's assistant is given,
        // the commands the core gives the assistant and the name its
        // proposals are made under.
        // The crate's name, written so that this test's own lines do not
        // hold it.
        let core = concat!("agentdrafter", "_core");
        for source in [
            include_str!("http.rs"),
            include_str!("mcp.rs"),
            include_str!("assistant/mod.rs"),
        ] {
            let uses: Vec<&str> = source
                .lines()
                .filter(|line| line.contains(core))
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect();
            assert_eq!(uses.len(), 1, "one use of the core: {uses:?}");
            let start = source.find(uses[0]).unwrap();
            let named = source[start..]
                .split_once("};")
                .map(|(used, _)| used)
                .and_then(|used| used.split_once('{'))
                .map(|(_, named)| named)
                .unwrap();
            for item in named.split(',').map(str::trim).filter(|i| !i.is_empty()) {
                assert!(
                    [
                        "commands",
                        "find_command",
                        "Error",
                        "ErrorCode",
                        "Interface",
                        "Reply",
                        "Settings",
                        "JOURNAL_FILE",
                        "PROPOSALS_FOLDER",
                        "GUIDES_FOLDER",
                        "original_journal_file",
                        "looking_at_left_off",
                        "assistant_commands",
                        "Access",
                        "Command",
                        "ASSISTANT"
                    ]
                    .contains(&item),
                    "a server uses `{item}` from the core"
                );
            }
        }
    }

    fn send_one(address: SocketAddr, request: &[u8]) -> (u16, String, Vec<u8>) {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.write_all(request).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8(response[..split].to_vec()).unwrap();
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, head, response[split + 4..].to_vec())
    }

    fn read_sse_event(stream: &mut TcpStream, buf: &mut String) -> (String, Value) {
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(end) = buf.find("\n\n") {
                let block = buf[..end].to_owned();
                buf.drain(..end + 2);
                let mut event = String::new();
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(ev) = line.strip_prefix("event: ") {
                        event = ev.to_owned();
                    } else if let Some(d) = line.strip_prefix("data: ") {
                        data.push_str(d);
                    }
                }
                if !data.is_empty() {
                    let parsed: Value = serde_json::from_str(&data).unwrap();
                    return (event, parsed);
                }
            }
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0, "stream closed before next event: {buf}");
            buf.push_str(std::str::from_utf8(&chunk[..n]).unwrap());
        }
    }

    fn post_host(host: &str, command: &str, headers: &str, body: &str) -> Vec<u8> {
        format!(
            "POST /commands/{command} HTTP/1.1\r\nHost: {host}\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn events_host(host: &str, headers: &str, body: &str) -> Vec<u8> {
        format!(
            "POST /events HTTP/1.1\r\nHost: {host}\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn start_local_guards_commands_with_token_serves_desktop_csp_and_shuts_down() {
        let server = start_local(Settings::default()).unwrap();
        let host = server.address().to_string();
        assert!(server.address().ip().is_loopback());
        assert_eq!(server.token().len(), 64);
        assert_eq!(server.base_url(), format!("http://{host}"));

        // Page served without token on exact loopback Host:port with desktop CSP.
        let (status, head, body) = send_one(
            server.address(),
            &get("GET", "/?desktop=1&start=1", &host, ""),
        );
        assert_eq!(status, 200);
        assert!(head.contains(&format!(
            "Content-Security-Policy: {}",
            page::DESKTOP_POLICY
        )));
        assert_eq!(body.as_slice(), page::file("/").unwrap().bytes);

        // Non-local Host, missing Host, wrong port Host, and wrong Origin all refused (403).
        let (status, _, _) = send_one(server.address(), &get("GET", "/", "rebound.example", ""));
        assert_eq!(status, 403);
        let (status, _, _) = send_one(server.address(), b"GET / HTTP/1.1\r\n\r\n");
        assert_eq!(status, 403, "missing Host must be refused in desktop mode");
        let (status, _, _) = send_one(server.address(), &get("GET", "/", "127.0.0.1:9999", ""));
        assert_eq!(
            status, 403,
            "wrong port Host must be refused in desktop mode"
        );
        let (status, _, _) = send_one(
            server.address(),
            &get("GET", "/", &host, "Origin: http://127.0.0.1:9999\r\n"),
        );
        assert_eq!(
            status, 403,
            "wrong port Origin must be refused in desktop mode"
        );

        // Command without token refused (401) when Host matches.
        let (status, _, _) = send_one(
            server.address(),
            &post_host(&host, "operations", JSON, "{}"),
        );
        assert_eq!(status, 401);

        // Command with token but missing Host refused (403).
        let raw_no_host = format!(
            "POST /commands/operations HTTP/1.1\r\n{JSON}Authorization: Bearer {}\r\nContent-Length: 2\r\n\r\n{{}}",
            server.token()
        );
        let (status, _, _) = send_one(server.address(), raw_no_host.as_bytes());
        assert_eq!(
            status, 403,
            "missing Host must be refused even with valid token"
        );

        // Command with exact Host, Origin, token and absolute path works.
        let auth = format!(
            "{JSON}Origin: http://{host}\r\nAuthorization: Bearer {}\r\n",
            server.token()
        );
        let abs_yaml = std::fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("examples/geometry.yaml"),
        )
        .unwrap();
        let body = serde_json::to_string(&json!({ "source": abs_yaml })).unwrap();
        let (status, _, raw) = send_one(
            server.address(),
            &post_host(&host, "display-list", &auth, &body),
        );
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["sheet"], "geometry");

        // Drop shuts down the server cleanly.
        let addr = server.address();
        drop(server);
        assert!(
            TcpStream::connect(addr).is_err(),
            "local server stops accepting connections after drop"
        );
    }

    #[test]
    fn events_stream_watches_original_dwg_journal_and_handles_single_yaml_cleanly() {
        let tmp =
            std::env::temp_dir().join(format!("agentdrafter-http-events-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let server = start_local(Settings::default()).unwrap();
        let host = server.address().to_string();
        let auth = format!("{JSON}Authorization: Bearer {}\r\n", server.token());

        // Build a made-up clean DWG from examples/geometry.yaml into tmp.
        let abs_yaml = std::fs::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("examples/geometry.yaml"),
        )
        .unwrap();
        let dwg_path = tmp.join("sample-orig.dwg");
        let build_body = serde_json::to_string(&json!({
            "source": abs_yaml,
            "output": dwg_path,
            "overwrite": true
        }))
        .unwrap();
        let (status, _, raw) = send_one(
            server.address(),
            &post_host(&host, "build", &auth, &build_body),
        );
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&raw));

        // Open POST /events watching the absolute original DWG path.
        let mut sse = TcpStream::connect(server.address()).unwrap();
        sse.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let watch_body = serde_json::to_string(&json!({ "set": dwg_path })).unwrap();
        sse.write_all(&events_host(&host, &auth, &watch_body))
            .unwrap();
        let mut buf = String::new();
        let (ev1, r1) = read_sse_event(&mut sse, &mut buf);
        assert_eq!(ev1, "journal");
        assert_eq!(r1["ok"], true, "{r1}");
        assert_eq!(r1["result"]["entries"].as_array().unwrap().len(), 0);

        // Apply a change set to the original DWG -> live journal event arrives!
        let change_body = serde_json::to_string(&json!({
            "set": dwg_path,
            "no_pdf": true,
            "change_set": {
                "operations": [{
                    "op": "add_line",
                    "layer": "0",
                    "from": [1, 1, 0],
                    "to": [2, 1, 0]
                }]
            }
        }))
        .unwrap();
        let (status, _, raw) = send_one(
            server.address(),
            &post_host(&host, "change", &auth, &change_body),
        );
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&raw));

        let (ev2, r2) = read_sse_event(&mut sse, &mut buf);
        assert_eq!(ev2, "journal");
        assert_eq!(r2["ok"], true, "{r2}");
        assert_eq!(r2["result"]["entries"].as_array().unwrap().len(), 1);
        assert_eq!(r2["result"]["can_undo"], json!([1]));
        drop(sse);

        // Single YAML sheet on POST /events responds cleanly without panic/crash.
        let mut yaml_sse = TcpStream::connect(server.address()).unwrap();
        yaml_sse
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let yaml_watch = serde_json::to_string(&json!({ "set": abs_yaml })).unwrap();
        yaml_sse
            .write_all(&events_host(&host, &auth, &yaml_watch))
            .unwrap();
        let mut ybuf = String::new();
        let (ev_y, r_y) = read_sse_event(&mut yaml_sse, &mut ybuf);
        assert_eq!(ev_y, "journal");
        assert_eq!(r_y["ok"], false, "single yaml has no set journal: {r_y}");
        drop(yaml_sse);

        drop(server);
        let _ = fs::remove_dir_all(&tmp);
    }
}
