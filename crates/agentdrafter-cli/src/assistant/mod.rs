//! The assistant (#156, drafting app build 8): the drafter's own agent,
//! started by `agentdrafter serve` and pointed at `agentdrafter mcp
//! --assistant` on the drawing set the viewer has open.
//!
//! `serve` looks for each agent it has an adapter for on the PATH when it
//! starts, Claude Code (`claude`) and Codex (`codex`), and again when the
//! page asks. It keeps one conversation for each drawing set the page names,
//! on one agent, in memory. Each message the drafter sends is one run of
//! the agent's command line, its prompt on standard input, resuming the
//! session the first run began; the adapter for that agent turns what the
//! run prints into the conversation's parts: its words, and each command it
//! calls, with its input and reply. The page watches the conversation as a
//! stream of events.
//!
//! The assistant's edits are always proposals. The MCP server it is given
//! serves only the commands that read and `propose`, which the core names,
//! and tells it so; the agent's own tools are taken away (Claude Code's
//! built-in tools) or held to reading (Codex's shell). So what it would
//! change waits, drawn for the drafter to keep or discard, and it never
//! changes the set itself.
//!
//! This holds no drawing logic (ADR-0007): what the drafter is looking at
//! is recorded and read by the core's `look-at` and `looking-at`, and the
//! agent reaches the set only through the core's commands.

mod claude;
mod codex;

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agentdrafter_core::{find_command, Error, ErrorCode, Interface, Reply, Settings};
use serde::Deserialize;
use serde_json::{json, Value};

/// The name of the assistant's MCP server, as the agent knows it: its
/// tools are `mcp__agentdrafter__<command>` to Claude Code.
pub(crate) const SERVER: &str = "agentdrafter";

/// How long one message may run before its agent is stopped.
const LONGEST_TURN: Duration = Duration::from_secs(20 * 60);

/// The longest message the drafter may send, in characters.
const LONGEST_MESSAGE: usize = 20_000;

/// An agent AgentDrafter has an adapter for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Agent {
    Claude,
    Codex,
}

impl Agent {
    const ALL: [Agent; 2] = [Agent::Claude, Agent::Codex];

    /// Its key, as the page names it.
    fn key(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }

    /// Its name, as the panel shows it.
    fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }

    /// The command it is found by on the PATH.
    fn command(self) -> &'static str {
        self.key()
    }

    /// How it is installed: the line, and its own page.
    fn install(self) -> (&'static str, &'static str) {
        match self {
            Agent::Claude => (
                "npm install -g @anthropic-ai/claude-code",
                "https://docs.anthropic.com/en/docs/claude-code/setup",
            ),
            Agent::Codex => (
                "npm install -g @openai/codex",
                "https://github.com/openai/codex",
            ),
        }
    }

    fn of(key: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|agent| agent.key() == key)
    }
}

/// What a line of an agent's output says, as its adapter reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AgentEvent {
    /// The session or thread the run is in, which the next message resumes.
    Session(String),
    /// Words the agent says to the drafter.
    Words(String),
    /// A tool called, by the registry's name for one of the assistant's
    /// server's, with its input.
    Started {
        id: String,
        tool: String,
        input: Value,
    },
    /// A tool's reply: the command's, as the command line prints it.
    Done { id: String, reply: Value, ok: bool },
    /// The run failed, and why.
    Failed(String),
}

impl AgentEvent {
    /// A tool's reply from the text it gave: a command's JSON reply, or, for
    /// any other tool, its text.
    fn done(id: String, said: &str, refused: bool) -> AgentEvent {
        let reply = match serde_json::from_str::<Value>(said) {
            Ok(reply @ Value::Object(_)) => reply,
            _ if refused => json!({ "ok": false, "error": { "message": said } }),
            _ => json!({ "ok": true, "text": said }),
        };
        let ok = !refused && reply.get("ok").and_then(Value::as_bool) != Some(false);
        AgentEvent::Done { id, reply, ok }
    }
}

/// The command that starts `program`, with no window of its own on Windows.
fn program(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// The assistant of this `serve`: the agents found and every conversation.
pub(crate) struct Assistant {
    found: Mutex<Vec<(Agent, PathBuf)>>,
    talks: Mutex<HashMap<PathBuf, Talk>>,
    changed: Condvar,
    /// The command that runs AgentDrafter, which the agent starts its MCP
    /// server with.
    own: Vec<OsString>,
}

/// One conversation, on one drawing set.
struct Talk {
    agent: Option<Agent>,
    /// The session the agent's runs continue, once the first has named it.
    session: Option<String>,
    busy: bool,
    items: Vec<Value>,
    /// Moves on with every change, for the event streams.
    version: u64,
}

static ASSISTANT: OnceLock<Assistant> = OnceLock::new();

/// The assistant, which looks for the agents on the PATH the first time.
pub(crate) fn assistant() -> &'static Assistant {
    ASSISTANT.get_or_init(|| Assistant {
        found: Mutex::new(find_agents()),
        talks: Mutex::new(HashMap::new()),
        changed: Condvar::new(),
        own: own_command(),
    })
}

/// The command that runs AgentDrafter: this binary, or, in the Python
/// package's command, its Python with `-m agentdrafter`.
fn own_command() -> Vec<OsString> {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("agentdrafter"));
    let python = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_ascii_lowercase().starts_with("python"));
    if python {
        vec![exe.into_os_string(), "-m".into(), "agentdrafter".into()]
    } else {
        vec![exe.into_os_string()]
    }
}

/// Each agent found on the PATH, with where.
fn find_agents() -> Vec<(Agent, PathBuf)> {
    Agent::ALL
        .into_iter()
        .filter_map(|agent| on_path(agent.command()).map(|path| (agent, path)))
        .collect()
}

/// Where `name` is on the PATH: an executable file of that name, or on
/// Windows with one of the extensions PATHEXT lists.
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        extensions
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| format!("{name}{}", e.to_ascii_lowercase()))
            .collect()
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(&path)
        .flat_map(|folder| names.iter().map(move |n| folder.join(n)))
        .find(|candidate| runnable(candidate))
}

#[cfg(unix)]
fn runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> bool {
    path.is_file()
}

/// What the drafter is looking at, as the page sends it with a message.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LookingAt {
    sheet: String,
    #[serde(default)]
    layout: Option<String>,
    #[serde(default)]
    selected: Vec<String>,
    /// The objects' names, as the page shows them, in the same order.
    #[serde(default)]
    names: Vec<Option<String>>,
}

/// A message from the drafter.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Message {
    set: PathBuf,
    text: String,
    #[serde(default)]
    via: Option<String>,
    #[serde(default)]
    looking_at: Option<LookingAt>,
}

/// The picker: the agent a set's conversation goes on with.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Choice {
    set: PathBuf,
    agent: String,
}

/// An error a route of the assistant's replies with, and its status.
pub(crate) type Refusal = (u16, Error);

impl Assistant {
    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, Talk>> {
        self.talks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn found(&self) -> Vec<(Agent, PathBuf)> {
        self.found.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The agents, found or not, and how to install each: `GET /assistant`.
    pub(crate) fn agents(&self) -> Value {
        let found = self.found();
        let agents: Vec<Value> = Agent::ALL
            .into_iter()
            .map(|agent| {
                json!({
                    "agent": agent.key(),
                    "name": agent.name(),
                    "command": agent.command(),
                    "found": found.iter().any(|(a, _)| *a == agent),
                })
            })
            .collect();
        let install: Vec<Value> = Agent::ALL
            .into_iter()
            .map(|agent| {
                let (line, page) = agent.install();
                json!({ "agent": agent.key(), "name": agent.name(), "line": line, "page": page })
            })
            .collect();
        json!({ "agents": agents, "install": install })
    }

    /// Looks on the PATH again: `POST /assistant/find`. A conversation with
    /// no agent goes on the first one found.
    pub(crate) fn find_again(&self) -> Value {
        let found = find_agents();
        *self.found.lock().unwrap_or_else(|e| e.into_inner()) = found.clone();
        let mut talks = self.lock();
        for (set, talk) in talks.iter_mut() {
            if talk.agent.is_none() {
                if let Some((agent, _)) = found.first() {
                    talk.agent = Some(*agent);
                    talk.note(format!(
                        "Looked on the PATH again: found {}. {}",
                        agent.name(),
                        started_words(*agent, set)
                    ));
                }
            }
        }
        drop(talks);
        self.changed.notify_all();
        self.agents()
    }

    /// The conversation on `set`, begun on the first agent found if there
    /// is none yet.
    fn talk<'a>(&self, talks: &'a mut HashMap<PathBuf, Talk>, set: &Path) -> &'a mut Talk {
        talks.entry(set.to_owned()).or_insert_with(|| {
            let agent = self.found().first().map(|(agent, _)| *agent);
            let mut talk = Talk {
                agent,
                session: None,
                busy: false,
                items: Vec::new(),
                version: 0,
            };
            if let Some(agent) = agent {
                talk.note(started_words(agent, set));
            }
            talk
        })
    }

    /// The conversation on `set` as the page shows it, and its version.
    pub(crate) fn snapshot(&self, set: &Path) -> (u64, String) {
        let mut talks = self.lock();
        let talk = self.talk(&mut talks, set);
        let reply = Reply::from(Ok(json!({
            "set": set,
            "agent": talk.agent.map(Agent::key),
            "busy": talk.busy,
            "items": talk.items,
        })));
        // A reply holds only strings, numbers, lists and string-keyed maps,
        // which always serialize, and JSON escapes every line break.
        let line = serde_json::to_string(&reply).expect("a reply serializes to JSON");
        (talk.version, line)
    }

    /// Waits up to `most` for the conversation on `set` to move on from
    /// `version`.
    pub(crate) fn wait(&self, set: &Path, version: u64, most: Duration) {
        let talks = self.lock();
        let moved = |talks: &mut HashMap<PathBuf, Talk>| {
            talks.get(set).is_some_and(|talk| talk.version != version)
        };
        let _ = self
            .changed
            .wait_timeout_while(talks, most, |talks| !moved(talks));
    }

    /// The picker: the conversation on the set goes on, anew, with another
    /// agent.
    pub(crate) fn choose(&self, choice: &Choice) -> Result<Value, Refusal> {
        let agent = Agent::of(&choice.agent).ok_or_else(|| {
            (
                400,
                Error::usage(format!(
                    "the agents are claude and codex, not `{}`",
                    choice.agent
                )),
            )
        })?;
        if !self.found().iter().any(|(a, _)| *a == agent) {
            return Err(no_agent(Some(agent)));
        }
        let mut talks = self.lock();
        let talk = self.talk(&mut talks, &choice.set);
        if talk.busy {
            return Err(busy(talk));
        }
        if talk.agent != Some(agent) {
            talk.agent = Some(agent);
            talk.session = None;
            talk.note(format!(
                "Now {} A new conversation.",
                started_words(agent, &choice.set).replacen("On ", "on ", 1)
            ));
        }
        drop(talks);
        self.changed.notify_all();
        Ok(json!({ "agent": agent.key() }))
    }

    /// A message from the drafter: recorded with what they are looking at,
    /// through the core, and passed to the agent, whose work streams into
    /// the conversation.
    pub(crate) fn send(
        &'static self,
        message: Message,
        settings: &Settings,
    ) -> Result<Value, Refusal> {
        let text = message.text.trim().to_owned();
        if text.is_empty() {
            return Err((400, Error::usage("a message to the assistant has words")));
        }
        if text.chars().count() > LONGEST_MESSAGE {
            return Err((
                400,
                Error::usage(format!(
                    "a message to the assistant holds at most {LONGEST_MESSAGE} characters"
                )),
            ));
        }
        let via = match message.via.as_deref() {
            None | Some("chat") => "chat",
            Some("command_bar") => "command_bar",
            Some(other) => {
                return Err((
                    400,
                    Error::usage(format!(
                        "a message comes `via` the chat or the command_bar, not `{other}`"
                    )),
                ))
            }
        };
        let (agent, path, session, first) = {
            let mut talks = self.lock();
            let talk = self.talk(&mut talks, &message.set);
            let Some(agent) = talk.agent else {
                return Err(no_agent(None));
            };
            if talk.busy {
                return Err(busy(talk));
            }
            let Some(path) = self
                .found()
                .into_iter()
                .find(|(a, _)| *a == agent)
                .map(|(_, p)| p)
            else {
                return Err(no_agent(Some(agent)));
            };
            let first = talk.session.is_none();
            (agent, path, talk.session.clone(), first)
        };
        // What the drafter is looking at is the core's: recorded through
        // `look-at`, as the page records it, before the agent is told.
        let looked = match &message.looking_at {
            Some(looking) => Some(self.record(&message.set, looking, settings)?),
            None => None,
        };
        let prompt = prompt(&message.set, &text, looked.as_ref(), first);
        let item = {
            let mut talks = self.lock();
            let talk = self.talk(&mut talks, &message.set);
            if talk.busy {
                return Err(busy(talk));
            }
            talk.items.push(json!({
                "kind": "drafter",
                "text": text,
                "via": via,
                "looking_at": looked.as_ref().map(|l| json!({
                    "sheet": l.sheet,
                    "layout": l.layout,
                    "selected": l.selected,
                    "names": l.names,
                })),
                "at": utc_now(),
            }));
            let item = talk.items.len() - 1;
            talk.items.push(json!({
                "kind": "assistant",
                "agent": agent.key(),
                "name": agent.name(),
                "at": utc_now(),
                "parts": [],
                "done": false,
                "error": null,
            }));
            talk.busy = true;
            talk.version += 1;
            item
        };
        self.changed.notify_all();
        let turn = Turn {
            set: message.set.clone(),
            agent,
            program: path,
            session,
            prompt,
            item: item + 1,
        };
        if let Err(e) = std::thread::Builder::new()
            .name("agentdrafter-assistant".into())
            .spawn(move || self.run(turn))
        {
            self.finish(
                &message.set,
                item + 1,
                Some(format!("could not start: {e}")),
            );
        }
        Ok(json!({ "item": item }))
    }

    /// Records what the drafter is looking at through the core's `look-at`,
    /// as the viewer does, and gives it back as the core gave it.
    fn record(
        &self,
        set: &Path,
        looking: &LookingAt,
        settings: &Settings,
    ) -> Result<LookingAt, Refusal> {
        let command = find_command("look-at").expect("the registry holds `look-at`");
        let reply = command.run(
            json!({
                "set": set,
                "sheet": looking.sheet,
                "layout": looking.layout,
                "selected": looking.selected,
            }),
            settings,
            Interface::Viewer,
        );
        if !reply.ok {
            let error = reply.error.unwrap_or_else(|| {
                Error::new(
                    ErrorCode::ReadFailed,
                    "what the drafter is looking at was not recorded",
                )
            });
            let status = if error.code == ErrorCode::Usage {
                400
            } else {
                422
            };
            return Err((status, error));
        }
        let result: Value = reply
            .result
            .and_then(|r| serde_json::from_str(r.get()).ok())
            .unwrap_or(Value::Null);
        let selected: Vec<String> = result["selected"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(|i| i.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        // A name follows its object's ID, which the core may have taken as
        // a repeat.
        let names = selected
            .iter()
            .map(|id| {
                looking
                    .selected
                    .iter()
                    .position(|given| given.eq_ignore_ascii_case(id))
                    .and_then(|i| looking.names.get(i).cloned().flatten())
            })
            .collect();
        Ok(LookingAt {
            sheet: result["sheet"]
                .as_str()
                .unwrap_or(&looking.sheet)
                .to_owned(),
            layout: result["layout"].as_str().map(str::to_owned),
            selected,
            names,
        })
    }

    /// Runs one message on its agent, and streams what it does into the
    /// conversation.
    fn run(&'static self, turn: Turn) {
        let outcome = self.drive(&turn);
        self.finish(&turn.set, turn.item, outcome.err());
    }

    fn drive(&'static self, turn: &Turn) -> Result<(), String> {
        let mut command = match turn.agent {
            Agent::Claude => {
                let config = self.claude_config()?;
                claude::command(&turn.program, &config, turn.session.as_deref())
            }
            Agent::Codex => codex::command(&turn.program, &self.own, turn.session.as_deref()),
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("{} could not be started: {e}", turn.agent.name()))?;
        if let Some(mut stdin) = child.stdin.take() {
            // An agent that stops before reading its prompt says why on its
            // standard error.
            let _ = stdin.write_all(turn.prompt.as_bytes());
        }
        let stdout = child.stdout.take().expect("standard output is piped");
        let stderr = child.stderr.take().expect("standard error is piped");
        let said = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr)
                .take(1 << 20)
                .read_to_string(&mut text);
            text
        });
        let child = Arc::new(Mutex::new(child));
        let done = Arc::new(Mutex::new(false));
        let watchdog = {
            let (child, done) = (Arc::clone(&child), Arc::clone(&done));
            std::thread::spawn(move || {
                let started = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_millis(250));
                    if *done.lock().unwrap_or_else(|e| e.into_inner()) {
                        return false;
                    }
                    if started.elapsed() >= LONGEST_TURN {
                        let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
                        return true;
                    }
                }
            })
        };
        let mut parts: HashMap<String, usize> = HashMap::new();
        let mut failed: Option<String> = None;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let events = match turn.agent {
                Agent::Claude => claude::read(&line),
                Agent::Codex => codex::read(&line),
            };
            for event in events {
                if let AgentEvent::Failed(why) = &event {
                    failed = Some(why.clone());
                }
                self.apply(turn, &mut parts, event);
            }
        }
        let status = child.lock().unwrap_or_else(|e| e.into_inner()).wait();
        *done.lock().unwrap_or_else(|e| e.into_inner()) = true;
        let stopped = watchdog.join().unwrap_or(false);
        let said = said.join().unwrap_or_default();
        let name = turn.agent.name();
        if stopped {
            return Err(format!(
                "{name} was stopped after {} minutes",
                LONGEST_TURN.as_secs() / 60
            ));
        }
        if let Some(why) = failed {
            return Err(format!("{name} stopped: {why}"));
        }
        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => {
                let last = said
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim();
                Err(if last.is_empty() {
                    format!("{name} stopped ({status})")
                } else {
                    format!("{name} stopped: {last}")
                })
            }
            Err(e) => Err(format!("{name} could not be waited for: {e}")),
        }
    }

    /// The MCP configuration Claude Code is given, written to a file once
    /// for this `serve`.
    fn claude_config(&self) -> Result<PathBuf, String> {
        static WRITTEN: OnceLock<Result<PathBuf, String>> = OnceLock::new();
        WRITTEN
            .get_or_init(|| {
                let folder = std::env::temp_dir()
                    .join(format!("agentdrafter-assistant-{}", std::process::id()));
                let path = folder.join("claude-mcp.json");
                std::fs::create_dir_all(&folder)
                    .and_then(|()| std::fs::write(&path, claude::config(&self.own).to_string()))
                    .map(|()| path)
                    .map_err(|e| {
                        format!("the MCP configuration for Claude Code could not be written: {e}")
                    })
            })
            .clone()
    }

    /// Adds what one line of the agent's output said to the conversation.
    fn apply(&self, turn: &Turn, parts: &mut HashMap<String, usize>, event: AgentEvent) {
        let mut talks = self.lock();
        let Some(talk) = talks.get_mut(&turn.set) else {
            return;
        };
        if let AgentEvent::Session(session) = &event {
            talk.session = Some(session.clone());
            return;
        }
        let Some(list) = talk
            .items
            .get_mut(turn.item)
            .and_then(|item| item["parts"].as_array_mut())
        else {
            return;
        };
        match event {
            AgentEvent::Session(_) => {}
            AgentEvent::Words(text) => list.push(json!({ "kind": "words", "text": text })),
            AgentEvent::Started { id, tool, input } => {
                list.push(json!({
                    "kind": "command",
                    "command": tool,
                    "input": input,
                    "reply": null,
                    "ok": null,
                }));
                parts.insert(id, list.len() - 1);
            }
            AgentEvent::Done { id, reply, ok } => match parts.get(&id) {
                Some(&at) => {
                    list[at]["reply"] = reply;
                    list[at]["ok"] = json!(ok);
                }
                None => list.push(json!({
                    "kind": "command",
                    "command": "tool",
                    "input": {},
                    "reply": reply,
                    "ok": ok,
                })),
            },
            AgentEvent::Failed(_) => {}
        }
        talk.version += 1;
        drop(talks);
        self.changed.notify_all();
    }

    /// Ends a message's run: done, and why it failed, if it did.
    fn finish(&self, set: &Path, item: usize, error: Option<String>) {
        let mut talks = self.lock();
        if let Some(talk) = talks.get_mut(set) {
            if let Some(entry) = talk.items.get_mut(item) {
                entry["done"] = json!(true);
                if let Some(error) = error {
                    entry["error"] = json!(error);
                }
            }
            talk.busy = false;
            talk.version += 1;
        }
        drop(talks);
        self.changed.notify_all();
    }
}

impl Talk {
    fn note(&mut self, text: String) {
        self.items
            .push(json!({ "kind": "note", "text": text, "at": utc_now() }));
        self.version += 1;
    }
}

/// One message's run.
struct Turn {
    set: PathBuf,
    agent: Agent,
    program: PathBuf,
    session: Option<String>,
    prompt: String,
    /// The conversation's item it fills.
    item: usize,
}

/// The note a conversation on `agent` begins with.
fn started_words(agent: Agent, set: &Path) -> String {
    format!(
        "On {}: AgentDrafter starts {} pointed at agentdrafter mcp, on {}.",
        agent.name(),
        agent.command(),
        set.display()
    )
}

fn busy(talk: &Talk) -> Refusal {
    let name = talk.agent.map_or("The assistant", Agent::name);
    (
        409,
        Error::new(
            ErrorCode::AssistantBusy,
            format!("{name} is still working: it answers one message at a time"),
        ),
    )
}

fn no_agent(agent: Option<Agent>) -> Refusal {
    let message = match agent {
        Some(agent) => format!(
            "{} was not found on the PATH when AgentDrafter looked",
            agent.name()
        ),
        None => "no agent was found on the PATH: install Claude Code or Codex, then look again"
            .to_owned(),
    };
    (422, Error::new(ErrorCode::NoAgent, message))
}

/// What the agent is told with the drafter's message: the first time in a
/// session, what it is and what it may do; each time, what the drafter is
/// looking at, or that they left it out; then their words.
fn prompt(set: &Path, text: &str, looking: Option<&LookingAt>, first: bool) -> String {
    let set = set.display();
    let mut prompt = String::new();
    if first {
        prompt.push_str(&format!(
            "You are the drafter's assistant in AgentDrafter's drafting app. The drafter has the \
             drawing set `{set}` open in the viewer, and talks to you from its chat. Reach the set \
             only through the `{SERVER}` MCP server's tools, naming the set as `{set}`. You never \
             change the set yourself: to change it, call `propose` with the change set `change` \
             would take (`operations` lists what one can hold). It waits, drawn dashed on the \
             drafter's drawing, and lands only when they keep it. Answer in a few plain \
             sentences.\n\n"
        ));
    }
    match looking {
        Some(looking) => {
            let on = match &looking.layout {
                Some(layout) => format!("sheet {}, layout {layout}", looking.sheet),
                None => format!("sheet {}", looking.sheet),
            };
            let selected: Vec<String> = looking
                .selected
                .iter()
                .enumerate()
                .map(|(i, id)| match looking.names.get(i).cloned().flatten() {
                    Some(name) if !name.eq_ignore_ascii_case(id) => format!("{id} ({name})"),
                    _ => id.clone(),
                })
                .collect();
            let selected = if selected.is_empty() {
                "nothing selected".to_owned()
            } else {
                format!("{} selected", selected.join(", "))
            };
            prompt.push_str(&format!(
                "[The drafter is looking at {on} of `{set}`, with {selected}. `looking-at` gives \
                 it too.]\n\n"
            ));
        }
        None => prompt.push_str(
            "[The drafter left out what they are looking at for this message: do not assume \
             a sheet or a selection.]\n\n",
        ),
    }
    prompt.push_str("The drafter says:\n");
    prompt.push_str(text);
    prompt.push('\n');
    prompt
}

/// The time now in UTC, as `YYYY-MM-DDTHH:MM:SSZ`.
fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_says_what_the_drafter_looks_at_or_that_they_left_it_out() {
        let looking = LookingAt {
            sheet: "DP-LD-101".into(),
            layout: None,
            selected: vec!["DP-LD-101/device".into()],
            names: vec![Some("FT-101".into())],
        };
        let first = prompt(
            Path::new("examples/demo-plant"),
            "move this 1 to the right",
            Some(&looking),
            true,
        );
        assert!(first.contains("call `propose`"), "{first}");
        assert!(first.contains(
            "[The drafter is looking at sheet DP-LD-101 of `examples/demo-plant`, with DP-LD-101/device (FT-101) selected."
        ), "{first}");
        assert!(first.ends_with("The drafter says:\nmove this 1 to the right\n"));
        let later = prompt(Path::new("examples/demo-plant"), "and again", None, false);
        assert!(!later.contains("call `propose`"), "told once a session");
        assert!(
            later.contains("left out what they are looking at"),
            "{later}"
        );
    }

    #[test]
    fn a_reply_is_the_commands_or_the_tools_text() {
        assert_eq!(
            AgentEvent::done(
                "t".into(),
                r#"{"ok":false,"error":{"code":"usage"}}"#,
                false
            ),
            AgentEvent::Done {
                id: "t".into(),
                reply: json!({"ok": false, "error": {"code": "usage"}}),
                ok: false
            }
        );
        assert_eq!(
            AgentEvent::done("t".into(), "plain", false),
            AgentEvent::Done {
                id: "t".into(),
                reply: json!({"ok": true, "text": "plain"}),
                ok: true
            }
        );
    }

    #[test]
    fn the_time_is_written_in_utc() {
        let now = utc_now();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.starts_with("20") && now.ends_with('Z'), "{now}");
    }
}
