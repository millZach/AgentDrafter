//! The HTTP API and the MCP server over the same core as the command line:
//! every command of the registry is reachable through all three, and gives
//! the same reply through each; the same change set gives the same journal
//! entry, apart from its time; an accept records the interface it came
//! through; the HTTP API listens on the local machine unless told
//! otherwise, and streams a set's journal as it changes, whichever process
//! changed it; and the MCP handshake works.
//!
//! Every server a test starts listens on the local machine only and is
//! stopped by the test, when the value that holds it is dropped.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "../../agentdrafter-core/tests/support/demo_set.rs"]
mod demo_set;
#[path = "../../agentdrafter-core/tests/support/snapshot.rs"]
mod snapshot;

const BIN: &str = env!("CARGO_BIN_EXE_agentdrafter");

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("servers")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `agentdrafter` in `cwd` and returns its exit code and the JSON it
/// printed.
fn cli(args: &[&str], cwd: &Path) -> (i32, Value) {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.stderr.is_empty(),
        "nothing on standard error: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "one line of JSON: {stdout}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

/// `agentdrafter serve`, started in a folder; stopped when dropped.
struct Http {
    child: Child,
    address: SocketAddr,
    started: Value,
}

impl Http {
    fn start(cwd: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(BIN)
            .arg("serve")
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let started: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(started["ok"], true, "{started}");
        let address = started["result"]["address"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        Self {
            child,
            address,
            started,
        }
    }

    /// Sends one request and returns its status and JSON.
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut stream = TcpStream::connect(self.address).unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.address,
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(head.contains("Content-Type: application/json"), "{head}");
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    fn run(&self, command: &str, input: &Value) -> (u16, Value) {
        self.send("POST", &format!("/commands/{command}"), Some(input))
    }

    /// Runs a command as the viewer's page sends it: the browser marks a
    /// request its page makes to its own server `Sec-Fetch-Site:
    /// same-origin`.
    fn run_as_viewer(&self, command: &str, input: &Value) -> (u16, Value) {
        let body = input.to_string();
        let mut stream = TcpStream::connect(self.address).unwrap();
        write!(
            stream,
            "POST /commands/{command} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Origin: http://{}\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: cors\r\n\
             Content-Length: {}\r\n\r\n{body}",
            self.address,
            self.address,
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    /// Opens the event stream of the drawing set `set`.
    fn events(&self, set: &str) -> Events {
        let body = json!({ "set": set }).to_string();
        let mut stream = TcpStream::connect(self.address).unwrap();
        write!(
            stream,
            "POST /events HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            self.address,
            body.len()
        )
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut events = Events {
            stream,
            read: String::new(),
        };
        let head = events.until("\r\n\r\n", Duration::from_secs(30));
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(
            head.contains("Content-Type: text/event-stream\r\n"),
            "{head}"
        );
        events
    }
}

/// An open event stream of a drawing set.
struct Events {
    stream: TcpStream,
    read: String,
}

impl Events {
    /// What came up to and with `end`, waiting no longer than `within`.
    fn until(&mut self, end: &str, within: Duration) -> String {
        let began = Instant::now();
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(at) = self.read.find(end) {
                let taken: String = self.read.drain(..at + end.len()).collect();
                return taken;
            }
            assert!(
                began.elapsed() < within,
                "nothing ended with {end:?} within {within:?}: {}",
                self.read
            );
            match self.stream.read(&mut chunk) {
                Ok(0) => panic!("the stream ended: {}", self.read),
                Ok(n) => self
                    .read
                    .push_str(std::str::from_utf8(&chunk[..n]).unwrap()),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }

    /// The next journal the stream sends, as the `journal` command replies,
    /// passing over comments.
    fn next_journal(&mut self, within: Duration) -> Value {
        loop {
            let event = self.until("\n\n", within);
            if event.starts_with(':') {
                continue;
            }
            let mut lines = event.lines();
            assert_eq!(lines.next(), Some("event: journal"), "{event}");
            let data = lines.next().unwrap().strip_prefix("data: ").unwrap();
            return serde_json::from_str(data).unwrap();
        }
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `agentdrafter mcp`, started in a folder; stopped when dropped, or by
/// closing its input.
struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(cwd: &Path) -> Self {
        let mut child = Command::new(BIN)
            .arg("mcp")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            next_id: 0,
        }
    }

    fn write(&mut self, message: &Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{message}").unwrap();
        input.flush().unwrap();
    }

    /// Sends a request and returns its answer, which carries its ID.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let line = self.request_line(method, params);
        let answer: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(answer["jsonrpc"], "2.0");
        assert_eq!(answer["id"], self.next_id, "{answer}");
        answer
    }

    /// Sends a request and returns its answer as the line it came in.
    fn request_line(&mut self, method: &str, params: Value) -> String {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        line
    }

    fn notify(&mut self, method: &str) {
        self.write(&json!({"jsonrpc": "2.0", "method": method}));
    }

    /// The legacy handshake, as a 2025-11-25 client opens.
    fn initialize(&mut self) -> Value {
        let answer = self.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "servers-test", "version": "0"}}),
        );
        self.notify("notifications/initialized");
        answer
    }

    /// Calls a tool and returns its result, checking that its text and its
    /// structured content are the same reply.
    fn call(&mut self, tool: &str, arguments: &Value) -> Value {
        let answer = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &answer["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["content"][0]["type"], "text");
        let reply: Value = serde_json::from_str(text).unwrap();
        assert_eq!(reply, result["structuredContent"]);
        assert_eq!(result["isError"], reply["ok"] == false);
        result.clone()
    }

    /// Closes the server's input, which stops it, and waits for it.
    fn finish(mut self) -> ExitStatus {
        drop(self.input.take());
        self.child.wait().unwrap()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `value` with every journal time and the interface an accept came
/// through taken out, the only things the same command through two
/// interfaces may differ in. A test of its own holds each accept to its
/// interface.
fn without_times(value: &Value) -> Value {
    match value {
        Value::Object(map) => map
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "at" | "via" | "accepted_at"))
            .map(|(key, value)| (key.clone(), without_times(value)))
            .collect::<serde_json::Map<_, _>>()
            .into(),
        Value::Array(items) => items.iter().map(without_times).collect(),
        other => other.clone(),
    }
}

/// The calls the interfaces' tests make, shared with the Python library's
/// (`python/tests/test_interfaces.py`), so every interface is held to the
/// same ones: the change sets they apply, by file name, and one call of each
/// command, as the command line's arguments and as the input the other
/// interfaces take.
const CALLS: &str = include_str!("calls.json");

fn shared() -> Value {
    serde_json::from_str(CALLS).unwrap()
}

/// The shared change set of this file name.
fn change_set(file: &str) -> Value {
    let change_set = shared()["change_sets"][file].clone();
    assert!(change_set.is_object(), "no change set {file} in calls.json");
    change_set
}

/// A folder for one interface, holding a copy of the demo set as
/// `demo-set` and the change sets as files, so every interface is given
/// the same relative paths.
fn interface_folder(test: &str, interface: &str) -> PathBuf {
    let dir = scratch(&format!("{test}/{interface}"));
    demo_set::copy_demo_set(&dir.join("demo-set"));
    for (file, change_set) in shared()["change_sets"].as_object().unwrap() {
        fs::write(dir.join(file), change_set.to_string()).unwrap();
    }
    dir
}

/// One call of each command, as the command line's arguments and as the
/// input the servers take. Run in the registry's order on a fresh copy of
/// the demo set, each finds what the ones before it made.
fn calls() -> BTreeMap<String, (Vec<String>, Value)> {
    shared()["calls"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, call)| {
            let args = serde_json::from_value(call["args"].clone()).unwrap();
            (name.clone(), (args, call["input"].clone()))
        })
        .collect()
}

#[test]
fn every_command_of_the_registry_is_reachable_through_every_interface_with_the_same_reply() {
    let (cli_dir, http_dir, mcp_dir) = (
        interface_folder("every-command", "cli"),
        interface_folder("every-command", "http"),
        interface_folder("every-command", "mcp"),
    );
    let http = Http::start(&http_dir, &["--listen", "127.0.0.1:0"]);
    let mut mcp = Mcp::start(&mcp_dir);
    mcp.initialize();

    // Each server lists every command of the registry, with its schema.
    let registry = agentdrafter_core::commands();
    let (status, listed) = http.send("GET", "/commands", None);
    assert_eq!(status, 200, "{listed}");
    let listed = listed["result"]["commands"].as_array().unwrap().clone();
    let tools = mcp.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(listed.len(), registry.len());
    assert_eq!(tools.len(), registry.len());
    for ((command, listed), tool) in registry.iter().zip(&listed).zip(&tools) {
        assert_eq!(listed["name"], command.name());
        assert_eq!(listed["does"], command.does());
        assert_eq!(listed["input_schema"], command.input_schema());
        assert_eq!(tool["name"], command.name());
        assert_eq!(tool["description"], command.does());
        assert_eq!(tool["inputSchema"], command.input_schema());
    }

    // Each command, run through each interface, gives the same reply.
    let calls = calls();
    for command in registry {
        let name = command.name();
        let Some((args, input)) = calls.get(name) else {
            panic!("`{name}` is in the registry: add a call of it to tests/calls.json");
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, by_cli) = cli(&args, &cli_dir);
        let (status, by_http) = http.run(name, input);
        let by_mcp = mcp.call(name, input)["structuredContent"].clone();
        assert_eq!(by_cli["ok"], true, "{name}: {by_cli}");
        assert_eq!((code, status), (0, 200), "{name}");
        assert_eq!(
            without_times(&by_http),
            without_times(&by_cli),
            "`{name}` through the HTTP API and the command line"
        );
        assert_eq!(
            without_times(&by_mcp),
            without_times(&by_cli),
            "`{name}` through the MCP server and the command line"
        );
    }
    assert!(
        mcp.finish().success(),
        "the MCP server stops when its input closes"
    );
    for dir in [&http_dir, &mcp_dir] {
        assert_eq!(
            snapshot::snapshot(&dir.join("demo-set"))
                .into_iter()
                .map(|(path, bytes)| (path.strip_prefix(dir).unwrap().to_owned(), bytes))
                .collect::<BTreeMap<_, _>>(),
            snapshot::snapshot(&cli_dir.join("demo-set"))
                .into_iter()
                .map(|(path, bytes)| (path.strip_prefix(&cli_dir).unwrap().to_owned(), bytes))
                .collect::<BTreeMap<_, _>>(),
            "every file of the set is the same after the same commands"
        );
    }
}

#[test]
fn the_same_change_set_gives_the_same_reply_and_journal_entry_through_every_interface() {
    let (cli_dir, http_dir, mcp_dir) = (
        interface_folder("same-change-set", "cli"),
        interface_folder("same-change-set", "http"),
        interface_folder("same-change-set", "mcp"),
    );
    for dir in [&cli_dir, &http_dir, &mcp_dir] {
        let (code, json) = cli(&["build", "demo-set"], dir);
        assert_eq!(code, 0, "{json}");
    }
    let http = Http::start(&http_dir, &["--listen", "127.0.0.1:0"]);
    let mut mcp = Mcp::start(&mcp_dir);
    mcp.initialize();

    // A template edit, which lands on every sheet, then a change set that
    // lands on one sheet and fails on another.
    for (file, ok) in [("move.json", true), ("partial.json", false)] {
        let input = json!({"set": "demo-set", "change_set": change_set(file)});
        let (code, by_cli) = cli(&["change", "demo-set", file], &cli_dir);
        let (status, by_http) = http.run("change", &input);
        let result = mcp.call("change", &input);
        assert_eq!(result["isError"], !ok);
        assert_eq!(by_cli["ok"], ok, "{by_cli}");
        assert_eq!((code, status), if ok { (0, 200) } else { (1, 422) });
        assert_eq!(
            by_http, by_cli,
            "the HTTP API's reply is the command line's"
        );
        assert_eq!(
            result["structuredContent"], by_cli,
            "the MCP server's reply is the command line's"
        );
    }
    assert!(mcp.finish().success());
    drop(http);

    let journal = |dir: &Path| -> Vec<Value> {
        fs::read_to_string(dir.join("demo-set").join(agentdrafter_core::JOURNAL_FILE))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let by_cli = journal(&cli_dir);
    // Two entries, and the lines that mark a change set's done.
    let entries = by_cli.iter().filter(|line| line.get("kind").is_some());
    assert_eq!(entries.count(), 2);
    for entry in &by_cli {
        assert!(entry["at"].as_str().unwrap().ends_with('Z'), "{entry}");
    }
    for dir in [&http_dir, &mcp_dir] {
        let entries = journal(dir);
        assert_eq!(
            entries.iter().map(without_times).collect::<Vec<_>>(),
            by_cli.iter().map(without_times).collect::<Vec<_>>(),
            "the same journal entries, apart from their times"
        );
    }
}

#[test]
fn an_accept_records_the_interface_it_came_through_and_issued_change_sets_cannot_be_undone() {
    for via in ["cli", "http", "viewer", "mcp"] {
        let dir = interface_folder("accept-via", via);
        let (code, built) = cli(&["build", "demo-set"], &dir);
        assert_eq!(code, 0, "{built}");
        let (code, landed) = cli(&["change", "demo-set", "move.json"], &dir);
        assert_eq!(code, 0, "{landed}");
        let input = json!({"set": "demo-set", "newest_seen": 1});
        let (accepted, refused) = match via {
            "cli" => (
                cli(&["accept", "demo-set", "--newest-seen", "1"], &dir).1,
                cli(&["undo", "demo-set", "1"], &dir).1,
            ),
            "http" | "viewer" => {
                let http = Http::start(&dir, &["--listen", "127.0.0.1:0"]);
                let accepted = if via == "viewer" {
                    http.run_as_viewer("accept", &input).1
                } else {
                    http.run("accept", &input).1
                };
                let (status, refused) =
                    http.run("undo", &json!({"set": "demo-set", "change_set": 1}));
                assert_eq!(status, 422, "{refused}");
                (accepted, refused)
            }
            _ => {
                let mut mcp = Mcp::start(&dir);
                mcp.initialize();
                let accepted = mcp.call("accept", &input)["structuredContent"].clone();
                let refused = mcp.call("undo", &json!({"set": "demo-set", "change_set": 1}))
                    ["structuredContent"]
                    .clone();
                (accepted, refused)
            }
        };
        assert_eq!(accepted["ok"], true, "{via}: {accepted}");
        assert_eq!(accepted["result"]["via"], via);
        assert_eq!(accepted["result"]["issued"], json!([1]));
        assert_eq!(accepted["result"]["journal_entry"], 2);
        assert_eq!(refused["ok"], false, "{via}: {refused}");
        assert_eq!(refused["error"]["code"], "issued", "{via}: {refused}");

        let entries: Vec<Value> =
            fs::read_to_string(dir.join("demo-set").join(agentdrafter_core::JOURNAL_FILE))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .filter(|line| line.get("kind").is_some())
                .collect();
        let accept = &entries[1];
        assert_eq!(accept["kind"], "accept");
        assert_eq!(
            accept["via"], via,
            "the journal records the interface: {accept}"
        );
        assert_eq!(accept["issued"], json!([1]));
        assert_eq!(entries.len(), 2, "a refused undo is not recorded");
    }
}

#[test]
fn an_open_event_stream_sends_the_journal_when_another_process_changes_the_set() {
    let dir = interface_folder("events", "http");
    let (code, built) = cli(&["build", "demo-set"], &dir);
    assert_eq!(code, 0, "{built}");
    let http = Http::start(&dir, &["--listen", "127.0.0.1:0"]);
    let mut events = http.events("demo-set");
    let first = events.next_journal(Duration::from_secs(30));
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(
        first["result"]["entries"],
        json!([]),
        "an empty journal at first"
    );

    // A change set sent by the command line, another process.
    let (code, landed) = cli(&["change", "demo-set", "move.json"], &dir);
    assert_eq!(code, 0, "{landed}");
    let next = events.next_journal(Duration::from_secs(10));
    let entries = next["result"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{next}");
    assert_eq!(entries[0]["kind"], "change_set");
    assert_eq!(
        entries[0]["landed_on"],
        json!(["DEMO-001", "DEMO-002", "DEMO-003"])
    );
    assert!(
        entries[0]["objects"]
            .as_array()
            .unwrap()
            .contains(&json!({"id": "DEMO-002/border", "how": "changed"})),
        "the objects it changed, on each sheet: {next}"
    );

    // Commands go on being answered while the stream is open, and an undo
    // through the HTTP API reaches the stream too.
    let (status, undone) = http.run("undo", &json!({"set": "demo-set", "change_set": 1}));
    assert_eq!(status, 200, "{undone}");
    let next = events.next_journal(Duration::from_secs(10));
    assert_eq!(next["result"]["entries"][1]["kind"], "undo", "{next}");

    // The stream asks for its set in a JSON body, as a command does.
    let (status, refused) = http.send("GET", "/events", None);
    assert_eq!(status, 405, "{refused}");
    let (status, refused) = http.send("POST", "/events", Some(&json!({"folder": "demo-set"})));
    assert_eq!(status, 400, "{refused}");
    assert_eq!(refused["error"]["code"], "usage");
}

/// Starts `agentdrafter change demo-set move.json` in `dir`, waits for the
/// core's test-only seam to stop its commit with the template and one DWG
/// in place and the others not, and kills it there.
fn killed_mid_commit(dir: &Path) {
    let mut child = Command::new(BIN)
        .args(["change", "demo-set", "move.json"])
        .current_dir(dir)
        .env("AGENTDRAFTER_TEST_STOP_COMMIT_AT", "16")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut said = String::new();
    BufReader::new(child.stderr.take().unwrap())
        .read_line(&mut said)
        .unwrap();
    assert!(said.contains("stopped the commit at step 16"), "{said}");
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn a_change_killed_mid_commit_is_rolled_back_by_the_next_command_through_any_server() {
    let dir = interface_folder("killed", "servers");
    let (code, built) = cli(&["build", "demo-set"], &dir);
    assert_eq!(code, 0, "{built}");
    let set = dir.join("demo-set");
    let start = snapshot::snapshot(&set);
    let http = Http::start(&dir, &["--listen", "127.0.0.1:0"]);
    let mut mcp = Mcp::start(&dir);
    mcp.initialize();

    // The HTTP API's next command rolls back the change set first.
    killed_mid_commit(&dir);
    assert!(snapshot::snapshot(&set) != start, "killed half-way");
    let (status, queried) = http.run(
        "query",
        &json!({"source": "demo-set", "query": {"kind": "blocks"}}),
    );
    assert_eq!(status, 200, "{queried}");
    assert_eq!(queried["rolled_back"]["entry"], 1, "{queried}");
    assert_eq!(queried["rolled_back"]["kind"], "change_set");
    assert!(snapshot::snapshot(&set) == start, "every file as it was");

    // So does the MCP server's.
    killed_mid_commit(&dir);
    let checked = mcp.call("check", &json!({"source": "demo-set"}));
    assert_eq!(checked["structuredContent"]["rolled_back"]["entry"], 2);
    assert!(snapshot::snapshot(&set) == start);

    // And an event stream the viewer opens, whose first journal says so.
    killed_mid_commit(&dir);
    let mut events = http.events("demo-set");
    let first = events.next_journal(Duration::from_secs(30));
    assert_eq!(first["rolled_back"]["entry"], 3, "{first}");
    let states: Vec<&Value> = first["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["state"])
        .collect();
    assert_eq!(states, [&json!("rolled_back"); 3]);
    assert!(snapshot::snapshot(&set) == start);

    // A further change set lands, through the HTTP API.
    let (status, landed) = http.run(
        "change",
        &json!({"set": "demo-set", "change_set": change_set("move.json")}),
    );
    assert_eq!(status, 200, "{landed}");
    assert!(landed.get("rolled_back").is_none());
    assert_eq!(landed["result"]["journal_entry"], 4);
    assert!(mcp.finish().success());
}

#[test]
fn serve_listens_on_the_local_machine_and_needs_no_token_there() {
    let dir = scratch("local");
    for listen in ["127.0.0.1:0", "localhost:0"] {
        let http = Http::start(&dir, &["--listen", listen]);
        let started = &http.started["result"];
        assert!(http.address.ip().is_loopback(), "{listen}: {started}");
        assert_eq!(started["local_only"], true);
        assert_eq!(started["token_required"], false);
        assert!(started.get("token").is_none());
        assert_eq!(started["listening"], format!("http://{}", http.address));
        assert_eq!(started["viewer"], format!("http://{}/", http.address));
        // The viewer's page, which the binary carries, built or not.
        let mut stream = TcpStream::connect(http.address).unwrap();
        write!(stream, "GET / HTTP/1.1\r\nHost: {}\r\n\r\n", http.address).unwrap();
        let mut page = String::new();
        stream.read_to_string(&mut page).unwrap();
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
        assert!(page.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(page.contains("Content-Security-Policy: "));
        let (status, json) = http.send("GET", "/commands", None);
        assert_eq!(
            status, 200,
            "no token is needed on the local machine: {json}"
        );
        let (status, json) = http.run("schema", &json!({"kind": "set"}));
        assert_eq!(status, 200, "{json}");
        assert_eq!(json["result"]["title"], "AgentDrafter drawing set settings");
        // A key the schema does not name is refused, as the command line
        // refuses it.
        let (status, json) = http.run(
            "query",
            &json!({"source": ".", "query": {"kind": "extents", "typo": 1}}),
        );
        assert_eq!(status, 400, "{json}");
        assert_eq!(json["error"]["code"], "usage");
    }

    // A wrong address, or a port another program holds, is a JSON error.
    let (code, json) = cli(&["serve", "--listen", "not an address"], &dir);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let (code, json) = cli(&["serve", "--listen", &format!("127.0.0.1:{port}")], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "listen_failed");
    let (code, json) = cli(&["mcp", "extra"], &dir);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");

    let (_, help) = cli(&["help"], &dir);
    assert!(help["result"]["commands"]["serve"]["usage"]
        .as_str()
        .unwrap()
        .starts_with("agentdrafter serve"));
    assert_eq!(
        help["result"]["commands"]["mcp"]["usage"],
        "agentdrafter mcp"
    );
}

#[test]
fn the_mcp_handshake_works_and_tools_are_listed_and_called() {
    let dir = scratch("mcp");
    demo_set::copy_demo_set(&dir.join("demo-set"));
    let mut mcp = Mcp::start(&dir);

    let answer = mcp.initialize();
    let result = &answer["result"];
    assert_eq!(result["protocolVersion"], "2025-11-25");
    assert_eq!(result["serverInfo"]["name"], "agentdrafter");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["instructions"]
        .as_str()
        .unwrap()
        .contains("agentdrafter"));
    assert_eq!(mcp.request("ping", json!({}))["result"], json!({}));

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    let registry: Vec<&str> = agentdrafter_core::commands()
        .iter()
        .map(|command| command.name())
        .collect();
    assert_eq!(names, registry);
    for tool in tools["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
        assert!(tool["description"].as_str().unwrap().len() > 40);
    }

    let result = mcp.call("build", &json!({"source": "demo-set"}));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["result"]["built"], 3);
    assert!(dir.join("demo-set/dwg/DEMO-003.dwg").is_file());
    let result = mcp.call(
        "query",
        &json!({"source": "demo-set", "query": {"kind": "blocks"}, "sheet": "DEMO-002"}),
    );
    assert_eq!(
        result["structuredContent"]["result"]["sheets"],
        json!(["DEMO-002"])
    );
    // A key the query's kind does not take is refused, not passed over.
    let result = mcp.call(
        "query",
        &json!({"source": "demo-set", "query": {"kind": "blocks", "name": "x"}}),
    );
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["error"]["code"], "usage");

    // A command's own error is the tool's, for the agent to read and fix.
    let result = mcp.call("journal", &json!({"set": "demo-set", "folder": "x"}));
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["error"]["code"], "usage");
    let result = mcp.call("build", &json!({"source": "demo-set"}));
    assert_eq!(
        result["isError"], true,
        "the DWGs are there and no overwrite is asked"
    );
    let answer = mcp.request("tools/call", json!({"name": "draw", "arguments": {}}));
    assert_eq!(answer["error"]["code"], -32602);

    // A client of the newer revision, which sends no handshake, is served
    // on the same process.
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28"});
    let answer = mcp.request("server/discover", json!({"_meta": meta}));
    assert_eq!(answer["result"]["supportedVersions"], json!(["2026-07-28"]));
    let answer = mcp.request(
        "tools/call",
        json!({"_meta": meta, "name": "operations", "arguments": {}}),
    );
    assert_eq!(answer["result"]["resultType"], "complete");
    assert_eq!(answer["result"]["isError"], false);
    assert!(answer["result"]["structuredContent"]["result"]["operations"].is_array());

    assert!(
        mcp.finish().success(),
        "the MCP server stops when its input closes"
    );
}

#[test]
fn the_mcp_server_s_structured_content_keeps_every_digit_of_its_text() {
    /// A reply's result, each field as the bytes it was sent in.
    #[derive(serde::Deserialize)]
    struct RawReply {
        result: BTreeMap<String, Box<serde_json::value::RawValue>>,
    }
    #[derive(serde::Deserialize)]
    struct Answer {
        result: ToolResult,
    }
    #[derive(serde::Deserialize)]
    struct ToolResult {
        content: Vec<Text>,
        #[serde(rename = "structuredContent")]
        structured: RawReply,
    }
    #[derive(serde::Deserialize)]
    struct Text {
        text: String,
    }

    let dir = scratch("mcp-digits");
    demo_set::copy_demo_set(&dir.join("demo-set"));
    let mut mcp = Mcp::start(&dir);
    mcp.initialize();
    // A number the core writes, the checker's tolerance of 0.01 mm in
    // inches, and one it is given with every digit an f64 holds.
    let calls = [
        (
            "check",
            json!({"source": "demo-set"}),
            "tolerance",
            "0.00039370078740157485",
        ),
        (
            "measure-text",
            json!({"contents": "P", "font": "romans.shx", "height": 100.12345678901235}),
            "top",
            "100.12345678901235",
        ),
    ];
    for (tool, arguments, field, digits) in calls {
        let line = mcp.request_line("tools/call", json!({"name": tool, "arguments": arguments}));
        let answer: Answer = serde_json::from_str(&line).unwrap();
        let text: RawReply = serde_json::from_str(&answer.result.content[0].text).unwrap();
        assert_eq!(text.result[field].get(), digits, "{tool}: {line}");
        // Every number, text and flag of the result is the same bytes in
        // the structured content as in the text. (A list or object's keys
        // may come in another order.)
        let mut compared = 0;
        for (key, sent) in &text.result {
            if sent.get().starts_with(['{', '[']) {
                continue;
            }
            assert_eq!(
                answer.result.structured.result[key].get(),
                sent.get(),
                "{tool}: `{key}` in the structured content and in the text"
            );
            compared += 1;
        }
        assert!(compared > 2, "{tool}: {line}");
    }
    assert!(mcp.finish().success());
}
