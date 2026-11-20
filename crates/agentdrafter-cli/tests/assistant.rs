//! The assistant (#156): `agentdrafter serve` finds the drafter's own agent
//! on the PATH, Claude Code or Codex, passes each chat message to it pointed
//! at `agentdrafter mcp --assistant` on the open set, and streams its words
//! and the commands it calls. Its edits are always proposals: the server it
//! is given withholds every command that writes.
//!
//! No real agent runs: `tests/fake-agent/fake_agent.py` stands in for both,
//! behind a shim named `claude` or `codex` first on the server's PATH. It
//! speaks each agent's own JSON lines and calls the real MCP server.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "../../agentdrafter-core/tests/support/demo_set.rs"]
mod demo_set;

const BIN: &str = env!("CARGO_BIN_EXE_agentdrafter");

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("assistant")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The fake agent's script.
fn fake_agent() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fake-agent")
        .join("fake_agent.py")
}

/// A folder holding a shim for each of `agents`, which runs the fake agent
/// as that agent.
fn shims(dir: &Path, agents: &[&str]) -> PathBuf {
    let folder = dir.join("path");
    fs::create_dir_all(&folder).unwrap();
    let script = fake_agent();
    for agent in agents {
        if cfg!(windows) {
            fs::write(
                folder.join(format!("{agent}.cmd")),
                format!("@python \"{}\" {agent} %*\r\n", script.display()),
            )
            .unwrap();
        } else {
            let shim = folder.join(agent);
            fs::write(
                &shim,
                format!(
                    "#!/bin/sh\nexec python3 \"{}\" {agent} \"$@\"\n",
                    script.display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
    }
    folder
}

/// The PATH for a server: `first`, then the system's, without any folder
/// that holds a real `claude` or `codex`, so only the shims are found.
fn path_with(first: &Path) -> std::ffi::OsString {
    let system = std::env::var_os("PATH").unwrap_or_default();
    let real = |folder: &Path| {
        [
            "claude",
            "codex",
            "claude.cmd",
            "codex.cmd",
            "claude.exe",
            "codex.exe",
        ]
        .iter()
        .any(|name| folder.join(name).is_file())
    };
    let folders = std::iter::once(first.to_owned())
        .chain(std::env::split_paths(&system).filter(|f| !real(f)));
    std::env::join_paths(folders).unwrap()
}

/// A built copy of the demo set as `demo-set` in `dir`.
fn demo_set(dir: &Path) {
    demo_set::copy_demo_set(&dir.join("demo-set"));
    let out = Command::new(BIN)
        .args(["build", "demo-set"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `agentdrafter serve` with a PATH of its own; stopped when dropped.
struct Serve {
    child: Child,
    address: SocketAddr,
}

impl Serve {
    fn start(cwd: &Path, path: &std::ffi::OsString, envs: &[(&str, String)]) -> Self {
        let mut command = Command::new(BIN);
        command
            .args(["serve", "--listen", "127.0.0.1:0"])
            .current_dir(cwd)
            .env("PATH", path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let started: Value = serde_json::from_str(&line).unwrap();
        let address = started["result"]["address"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        Self { child, address }
    }

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
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.send("POST", path, Some(&body))
    }

    /// Opens the stream of the assistant's conversation on `set`.
    fn conversation(&self, set: &str) -> Conversation {
        let body = json!({ "set": set }).to_string();
        let mut stream = TcpStream::connect(self.address).unwrap();
        write!(
            stream,
            "POST /assistant/events HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            self.address,
            body.len()
        )
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut conversation = Conversation {
            stream,
            read: String::new(),
        };
        let head = conversation.until("\r\n\r\n", Duration::from_secs(30));
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(head.contains("text/event-stream"), "{head}");
        conversation
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An open stream of the assistant's conversation.
struct Conversation {
    stream: TcpStream,
    read: String,
}

impl Conversation {
    fn until(&mut self, end: &str, within: Duration) -> String {
        let began = Instant::now();
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(at) = self.read.find(end) {
                return self.read.drain(..at + end.len()).collect();
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

    /// The next state of the conversation the stream sends.
    fn next(&mut self, within: Duration) -> Value {
        loop {
            let event = self.until("\n\n", within);
            if event.starts_with(':') {
                continue;
            }
            let mut lines = event.lines();
            assert_eq!(lines.next(), Some("event: assistant"), "{event}");
            let data = lines.next().unwrap().strip_prefix("data: ").unwrap();
            let reply: Value = serde_json::from_str(data).unwrap();
            assert_eq!(reply["ok"], true, "{reply}");
            return reply["result"].clone();
        }
    }

    /// The conversation once it is not busy and holds `items` items.
    fn settled(&mut self, items: usize) -> Value {
        let began = Instant::now();
        loop {
            let state = self.next(Duration::from_secs(60));
            if state["busy"] == false && state["items"].as_array().unwrap().len() == items {
                return state;
            }
            assert!(
                began.elapsed() < Duration::from_secs(90),
                "the conversation never settled: {state}"
            );
        }
    }
}

/// The commands an assistant item's parts call, in order.
fn commands_of(item: &Value) -> Vec<String> {
    item["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["kind"] == "command")
        .map(|part| part["command"].as_str().unwrap().to_owned())
        .collect()
}

/// The words of an assistant item.
fn words_of(item: &Value) -> String {
    item["parts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|part| part["kind"] == "words")
        .map(|part| part["text"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Each run the fake agent logged: its arguments and prompt.
fn runs(log: &Path) -> Vec<Value> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn cli(args: &[&str], cwd: &Path) -> Value {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn with_no_agent_on_the_path_the_assistant_says_how_to_install_one() {
    let dir = scratch("no-agent");
    demo_set(&dir);
    let empty = dir.join("empty");
    fs::create_dir_all(&empty).unwrap();
    let serve = Serve::start(&dir, &std::ffi::OsString::from(&empty), &[]);
    let (status, reply) = serve.send("GET", "/assistant", None);
    assert_eq!(status, 200, "{reply}");
    let agents = reply["result"]["agents"].as_array().unwrap();
    assert_eq!(
        agents
            .iter()
            .map(|a| (a["agent"].as_str().unwrap(), a["found"].as_bool().unwrap()))
            .collect::<Vec<_>>(),
        [("claude", false), ("codex", false)]
    );
    let install = reply["result"]["install"].as_array().unwrap();
    assert!(install[0]["line"].as_str().unwrap().contains("claude-code"));
    assert!(install[1]["line"].as_str().unwrap().contains("codex"));

    let mut conversation = serve.conversation("demo-set");
    let state = conversation.next(Duration::from_secs(10));
    assert_eq!(state["agent"], Value::Null);
    assert_eq!(state["items"], json!([]));
    let (status, reply) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "hello", "looking_at": null}),
    );
    assert_eq!(status, 422, "{reply}");
    assert_eq!(reply["error"]["code"], "no_agent");
    let (status, reply) = serve.post(
        "/assistant/agent",
        json!({"set": "demo-set", "agent": "codex"}),
    );
    assert_eq!(status, 422, "{reply}");
}

#[test]
fn each_agent_proposes_through_the_assistants_server_and_changes_nothing() {
    for agent in ["claude", "codex"] {
        let dir = scratch(&format!("proposes-{agent}"));
        demo_set(&dir);
        let shims = shims(&dir, &[agent]);
        let log = dir.join("runs.jsonl");
        let serve = Serve::start(
            &dir,
            &path_with(&shims),
            &[("FAKE_AGENT_LOG", log.display().to_string())],
        );
        let (_, reply) = serve.send("GET", "/assistant", None);
        let found: Vec<&str> = reply["result"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["found"] == true)
            .map(|a| a["agent"].as_str().unwrap())
            .collect();
        assert_eq!(found, [agent], "only the shim is found");

        let mut conversation = serve.conversation("demo-set");
        let state = conversation.next(Duration::from_secs(10));
        assert_eq!(state["agent"], agent);
        assert_eq!(state["items"][0]["kind"], "note");
        assert!(
            state["items"][0]["text"]
                .as_str()
                .unwrap()
                .contains("agentdrafter mcp, on demo-set"),
            "{state}"
        );

        // A message with what the drafter is looking at.
        let (status, reply) = serve.post(
            "/assistant/messages",
            json!({
                "set": "demo-set",
                "text": "move this 0.1 to the right",
                "via": "chat",
                "looking_at": {"sheet": "demo-001", "layout": null, "selected": ["DEMO-001/border"], "names": ["border"]},
            }),
        );
        assert_eq!(status, 200, "{agent}: {reply}");
        assert_eq!(reply["result"]["item"], 1);
        let state = conversation.settled(3);
        let drafter = &state["items"][1];
        assert_eq!(drafter["kind"], "drafter");
        assert_eq!(
            drafter["looking_at"]["sheet"], "DEMO-001",
            "the core's name for it"
        );
        assert_eq!(drafter["looking_at"]["names"], json!(["border"]));
        let answer = &state["items"][2];
        assert_eq!(answer["kind"], "assistant");
        assert_eq!(answer["done"], true);
        assert_eq!(answer["error"], Value::Null, "{agent}: {answer}");
        assert_eq!(
            commands_of(answer),
            ["looking-at", "query", "propose"],
            "{answer}"
        );
        let parts = answer["parts"].as_array().unwrap();
        assert!(
            parts
                .iter()
                .filter(|p| p["kind"] == "command")
                .all(|p| p["ok"] == true),
            "{answer}"
        );
        assert_eq!(
            parts[0]["reply"]["result"]["selected"],
            json!(["DEMO-001/border"])
        );
        let proposed = &parts[2]["reply"]["result"];
        assert_eq!(proposed["proposal"], 1, "{answer}");
        assert!(words_of(answer).contains("proposal 1"), "{answer}");

        // What the drafter looks at is the core's, through any interface;
        // the proposal waits, the assistant's; nothing landed.
        let looking = cli(&["looking-at", "demo-set"], &dir);
        assert_eq!(looking["result"]["selected"], json!(["DEMO-001/border"]));
        let waiting = cli(&["proposals", "demo-set"], &dir);
        let listed = waiting["result"]["proposals"].as_array().unwrap();
        assert_eq!(listed.len(), 1, "{waiting}");
        assert_eq!(listed[0]["by"], "Assistant");
        let journal = cli(&["journal", "demo-set"], &dir);
        assert_eq!(journal["result"]["entries"], json!([]), "nothing landed");

        // The server it is given withholds `change`.
        let (status, _) = serve.post(
            "/assistant/messages",
            json!({"set": "demo-set", "text": "change it directly", "looking_at": null}),
        );
        assert_eq!(status, 200);
        let state = conversation.settled(5);
        let answer = &state["items"][4];
        assert_eq!(state["items"][3]["looking_at"], Value::Null);
        assert_eq!(commands_of(answer), ["change"]);
        assert_eq!(answer["parts"][0]["ok"], false, "{answer}");
        assert!(
            words_of(answer).contains("change listed: false"),
            "{answer}"
        );

        // The first run is told what it is; the next resumes its session
        // and is told the drafter left out what they look at.
        let runs = runs(&log);
        assert_eq!(runs.len(), 2);
        let first = runs[0]["prompt"].as_str().unwrap();
        assert!(first.contains("call `propose`"), "{first}");
        assert!(
            first.contains("looking at sheet DEMO-001 of `demo-set`"),
            "{first}"
        );
        let second = runs[1]["prompt"].as_str().unwrap();
        assert!(!second.contains("call `propose`"), "{second}");
        assert!(
            second.contains("left out what they are looking at"),
            "{second}"
        );
        let args: Vec<&str> = runs[1]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        let resumes = if agent == "claude" {
            "--resume"
        } else {
            "resume"
        };
        assert!(args.contains(&resumes), "{agent} resumes: {args:?}");
    }
}

#[test]
fn one_message_at_a_time_and_the_picker_starts_a_new_conversation() {
    let dir = scratch("busy-and-picker");
    demo_set(&dir);
    let shims = shims(&dir, &["claude", "codex"]);
    let serve = Serve::start(
        &dir,
        &path_with(&shims),
        &[("FAKE_AGENT_DELAY", "1".to_owned())],
    );
    let mut conversation = serve.conversation("demo-set");
    let state = conversation.next(Duration::from_secs(10));
    assert_eq!(state["agent"], "claude", "the first found");
    let (status, _) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "hello there", "via": "command_bar"}),
    );
    assert_eq!(status, 200);
    let (status, reply) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "and again"}),
    );
    assert_eq!(status, 409, "{reply}");
    assert_eq!(reply["error"]["code"], "assistant_busy");
    let (status, _) = serve.post(
        "/assistant/agent",
        json!({"set": "demo-set", "agent": "codex"}),
    );
    assert_eq!(status, 409, "no other agent while one works");
    let state = conversation.settled(3);
    assert_eq!(state["items"][1]["via"], "command_bar");
    assert!(words_of(&state["items"][2]).contains("You said: hello there"));

    let (status, reply) = serve.post(
        "/assistant/agent",
        json!({"set": "demo-set", "agent": "codex"}),
    );
    assert_eq!(status, 200, "{reply}");
    let state = conversation.settled(4);
    assert_eq!(state["agent"], "codex");
    let note = state["items"][3]["text"].as_str().unwrap();
    assert!(note.starts_with("Now on Codex"), "{note}");
    assert!(note.ends_with("A new conversation."), "{note}");
    let (status, reply) = serve.post(
        "/assistant/agent",
        json!({"set": "demo-set", "agent": "gemini"}),
    );
    assert_eq!(status, 400, "{reply}");
    let (status, reply) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "   "}),
    );
    assert_eq!(status, 400, "{reply}");
}

#[test]
fn the_assistants_server_answers_for_the_message_and_for_its_set_alone() {
    let dir = scratch("bound");
    demo_set(&dir);
    // Another set beside it, which the assistant's server must not reach.
    fs::create_dir_all(dir.join("other")).unwrap();
    demo_set::copy_demo_set(&dir.join("other").join("demo-set"));
    let shims = shims(&dir, &["claude"]);
    let serve = Serve::start(
        &dir,
        &path_with(&shims),
        &[("FAKE_AGENT_DELAY", "1".to_owned())],
    );
    let mut conversation = serve.conversation("demo-set");
    conversation.next(Duration::from_secs(10));

    // What the message carried, though the viewer records another sheet
    // before the agent asks.
    let (status, _) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "what do I see", "looking_at": {"sheet": "DEMO-001", "selected": ["DEMO-001/border"]}}),
    );
    assert_eq!(status, 200);
    let moved = cli(&["look-at", "demo-set", "DEMO-002"], &dir);
    assert_eq!(moved["ok"], true, "{moved}");
    let state = conversation.settled(3);
    let said = words_of(&state["items"][2]);
    assert!(
        said.contains("DEMO-001") && said.contains("DEMO-001/border"),
        "{said}: {}",
        state["items"][2]
    );

    // Left off: the agent is told so, whatever the viewer records.
    let (status, _) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "what do I see", "looking_at": null}),
    );
    assert_eq!(status, 200);
    let state = conversation.settled(5);
    let said = words_of(&state["items"][4]);
    assert!(said.contains("You were looking at None with []"), "{said}");
    assert!(said.contains("left what they are looking at off"), "{said}");
    let live = cli(&["looking-at", "demo-set"], &dir);
    assert_eq!(
        live["result"]["sheet"], "DEMO-002",
        "the live record is the viewer's"
    );

    // Another set is refused by the server bound to this one.
    let (status, _) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "look at other/demo-set", "looking_at": null}),
    );
    assert_eq!(status, 200);
    let state = conversation.settled(7);
    let answer = &state["items"][6];
    assert_eq!(answer["parts"][0]["command"], "looking-at");
    assert_eq!(answer["parts"][0]["ok"], false, "{answer}");
    assert!(words_of(answer).contains("Refused: true"), "{answer}");
}

#[test]
fn each_route_takes_a_drawing_set_alone_and_one_set_has_one_conversation() {
    let dir = scratch("sets");
    demo_set(&dir);
    let shims = shims(&dir, &["claude"]);
    let serve = Serve::start(&dir, &path_with(&shims), &[]);
    for set in ["nowhere", "demo-set/templates"] {
        let (status, reply) = serve.post(
            "/assistant/messages",
            json!({"set": set, "text": "hello", "looking_at": null}),
        );
        assert_eq!(status, 422, "{set}: {reply}");
        assert_eq!(reply["error"]["code"], "read_failed", "{reply}");
        let (status, reply) =
            serve.post("/assistant/agent", json!({"set": set, "agent": "claude"}));
        assert_eq!(status, 422, "{set}: {reply}");
        let (status, reply) = serve.post("/assistant/events", json!({"set": set}));
        assert_eq!(status, 422, "{set}: {reply}");
    }
    // Two names of one set are one conversation, shown as first named.
    let mut conversation = serve.conversation("./demo-set");
    let state = conversation.next(Duration::from_secs(10));
    assert_eq!(state["set"], "./demo-set");
    let (status, _) = serve.post(
        "/assistant/messages",
        json!({"set": "demo-set", "text": "hello there"}),
    );
    assert_eq!(status, 200);
    let state = conversation.settled(3);
    assert!(words_of(&state["items"][2]).contains("You said: hello there"));
}

#[test]
fn looking_again_follows_the_agents_on_the_path() {
    let dir = scratch("look-again");
    demo_set(&dir);
    let shims = shims(&dir, &["claude", "codex"]);
    let serve = Serve::start(&dir, &path_with(&shims), &[]);
    let mut conversation = serve.conversation("demo-set");
    conversation.next(Duration::from_secs(10));
    let (status, _) = serve.post(
        "/assistant/agent",
        json!({"set": "demo-set", "agent": "codex"}),
    );
    assert_eq!(status, 200);
    assert_eq!(conversation.settled(2)["agent"], "codex");
    let codex = if cfg!(windows) { "codex.cmd" } else { "codex" };
    fs::remove_file(shims.join(codex)).unwrap();
    let (status, reply) = serve.post("/assistant/find", json!({}));
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["result"]["agents"][1]["found"], false);
    let state = conversation.settled(3);
    assert_eq!(state["agent"], "claude", "the server and the page agree");
    let note = state["items"][2]["text"].as_str().unwrap();
    assert!(
        note.starts_with("Looked on the PATH again: Codex is gone. Now on Claude Code"),
        "{note}"
    );
}
