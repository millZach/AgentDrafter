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

use agentdrafter_core::{
    find_command, looking_at_left_off, Error, ErrorCode, Interface, Reply, Settings,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// The name of the assistant's MCP server, as the agent knows it: its
/// tools are `mcp__agentdrafter__<command>` to Claude Code.
pub(crate) const SERVER: &str = "agentdrafter";

/// How long one message may run before its agent is stopped.
const LONGEST_TURN: Duration = Duration::from_secs(20 * 60);

/// How long an agent that has ended is given for what it printed last to
/// arrive, before whatever it left running is stopped.
const AFTER_EXIT: Duration = Duration::from_secs(2);

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

/// The assistant's MCP server, as an agent starts it: the command that runs
/// AgentDrafter, then `mcp --assistant`, bound to the conversation's set and
/// given the message's `looking-at` reply.
pub(crate) struct McpServer {
    command: String,
    args: Vec<String>,
}

impl McpServer {
    fn new(own: &[OsString], set: &Path, looking_at: &Path) -> Self {
        let (program, args) = own.split_first().expect("a command has a program");
        let mut args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        args.extend([
            "mcp".to_owned(),
            "--assistant".to_owned(),
            "--set".to_owned(),
            set.to_string_lossy().into_owned(),
            "--looking-at".to_owned(),
            looking_at.to_string_lossy().into_owned(),
        ]);
        McpServer {
            command: program.to_string_lossy().into_owned(),
            args,
        }
    }
}

/// A folder of the system's temporary folder made for one message's run
/// alone, private to its user, which holds the files the agent is started
/// with; removed when the run ends.
struct TurnFiles {
    folder: PathBuf,
}

impl TurnFiles {
    /// Makes a new folder, with a random name no other folder has: it is
    /// created only if nothing has that name.
    fn new() -> std::io::Result<Self> {
        loop {
            let mut bytes = [0u8; 12];
            getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
            let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            let folder = std::env::temp_dir().join(format!("agentdrafter-assistant-{name}"));
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&folder) {
                Ok(()) => return Ok(TurnFiles { folder }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Writes `name` in the folder, as a new file.
    fn write(&self, name: &str, value: &Value) -> std::io::Result<PathBuf> {
        let path = self.folder.join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(value.to_string().as_bytes())?;
        Ok(path)
    }
}

impl Drop for TurnFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
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

/// One conversation, on one drawing set, kept by the set's canonical path.
struct Talk {
    /// The set as the page named it first, which the agent is told.
    named: PathBuf,
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
    /// no agent goes on the first one found, and one whose agent is gone on
    /// another, or none, as a new conversation, so the server and the page
    /// agree on it. A conversation working on a message is left as it is
    /// until its turn ends, when [`Self::finish`] reconciles it (#232).
    pub(crate) fn find_again(&self) -> Value {
        self.apply_found(find_agents())
    }

    fn apply_found(&self, found: Vec<(Agent, PathBuf)>) -> Value {
        *self.found.lock().unwrap_or_else(|e| e.into_inner()) = found.clone();
        let mut talks = self.lock();
        for talk in talks.values_mut() {
            reconcile_talk(talk, &found);
        }
        drop(talks);
        self.changed.notify_all();
        self.agents()
    }

    /// The canonical path of the drawing set `set` names, which keys its
    /// conversation, begun on the first agent found if there is none yet.
    /// A folder that is not a drawing set's is refused, as the core's
    /// `looking-at` refuses it; if `set` is a valid set whose optional
    /// `.agentdrafter/looking-at.json` record is damaged, treat the damaged
    /// record as absent so Chat stays available (#231).
    pub(crate) fn open(&self, set: &Path) -> Result<PathBuf, Refusal> {
        let command = find_command("looking-at").expect("the registry holds `looking-at`");
        let reply = command.run(
            json!({ "set": set }),
            &Settings::default(),
            Interface::Viewer,
        );
        if !reply.ok {
            let error = reply.error.unwrap_or_else(|| {
                Error::new(ErrorCode::ReadFailed, "the drawing set could not be read")
            });
            let damaged_record = error.code == ErrorCode::ReadFailed
                && error
                    .file
                    .as_deref()
                    .is_some_and(|f| Path::new(f) == set.join(".agentdrafter/looking-at.json"));
            if !damaged_record {
                return Err((status_of(&error), error));
            }
        }
        let key = std::fs::canonicalize(set).map_err(|e| {
            let error = Error::new(
                ErrorCode::ReadFailed,
                format!("the drawing set `{}` cannot be found: {e}", set.display()),
            );
            (422, error)
        })?;
        let mut talks = self.lock();
        self.talk(&mut talks, &key, set);
        Ok(key)
    }

    /// The conversation kept by `key`, begun as named `set` if there is none.
    fn talk<'a>(
        &self,
        talks: &'a mut HashMap<PathBuf, Talk>,
        key: &Path,
        set: &Path,
    ) -> &'a mut Talk {
        talks.entry(key.to_owned()).or_insert_with(|| {
            let agent = self.found().first().map(|(agent, _)| *agent);
            let mut talk = Talk {
                named: set.to_owned(),
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

    /// The conversation kept by `key` as the page shows it, and its
    /// version.
    pub(crate) fn snapshot(&self, key: &Path) -> (u64, String) {
        let mut talks = self.lock();
        let talk = self.talk(&mut talks, key, key);
        let reply = Reply::from(Ok(json!({
            "set": talk.named,
            "canonical_set": key,
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
        let key = self.open(&choice.set)?;
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
        let talk = self.talk(&mut talks, &key, &choice.set);
        if talk.busy {
            return Err(busy(talk));
        }
        if talk.agent != Some(agent) {
            talk.agent = Some(agent);
            talk.session = None;
            talk.note(format!(
                "Now {} A new conversation.",
                started_words(agent, &talk.named).replacen("On ", "on ", 1)
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
        let key = self.open(&message.set)?;
        let (agent, path, session, first, named) = {
            let mut talks = self.lock();
            let talk = self.talk(&mut talks, &key, &message.set);
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
            (agent, path, talk.session.clone(), first, talk.named.clone())
        };
        // What the drafter is looking at is the core's: recorded through
        // `look-at`, as the page records it, before the agent is told. The
        // agent's `looking-at` gives what this message carried, or that the
        // drafter left it off, whatever the viewer records meanwhile.
        let (looked, snapshot) = match &message.looking_at {
            Some(looking) => {
                let (looked, reply) = self.record(&named, looking, settings)?;
                (Some(looked), reply)
            }
            None => (
                None,
                serde_json::to_value(looking_at_left_off(&named))
                    .expect("a reply serializes to JSON"),
            ),
        };
        let prompt = prompt(&named, &text, looked.as_ref(), first);
        let item = {
            let mut talks = self.lock();
            let talk = self.talk(&mut talks, &key, &named);
            if talk.busy {
                return Err(busy(talk));
            }
            // Another client may have rescanned or switched the agent while
            // `record` ran without the lock held (#232).
            if talk.agent != Some(agent) || !self.found().iter().any(|(a, _)| *a == agent) {
                return Err(no_agent(Some(agent)));
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
            set: key.clone(),
            agent,
            program: path,
            session,
            prompt,
            looking_at: snapshot,
            item: item + 1,
            limit: LONGEST_TURN,
        };
        if let Err(e) = std::thread::Builder::new()
            .name("agentdrafter-assistant".into())
            .spawn(move || self.run(turn))
        {
            self.finish(&key, item + 1, Some(format!("could not start: {e}")));
        }
        Ok(json!({ "item": item }))
    }

    /// Records what the drafter is looking at through the core's `look-at`,
    /// as the viewer does, and gives it back as the core gave it, with the
    /// core's reply, which is what `looking-at` gives for this message.
    fn record(
        &self,
        set: &Path,
        looking: &LookingAt,
        settings: &Settings,
    ) -> Result<(LookingAt, Value), Refusal> {
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
            return Err((status_of(&error), error));
        }
        let whole = serde_json::to_value(&reply).expect("a reply serializes to JSON");
        let result = whole["result"].clone();
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
        let looked = LookingAt {
            sheet: result["sheet"]
                .as_str()
                .unwrap_or(&looking.sheet)
                .to_owned(),
            layout: result["layout"].as_str().map(str::to_owned),
            selected,
            names,
        };
        Ok((looked, whole))
    }

    /// Runs one message on its agent, and streams what it does into the
    /// conversation.
    fn run(&'static self, turn: Turn) {
        let outcome = self.drive(&turn);
        self.finish(&turn.set, turn.item, outcome.err());
    }

    fn drive(&'static self, turn: &Turn) -> Result<(), String> {
        let name = turn.agent.name();
        let unwritten = |e: std::io::Error| format!("{name} could not be given its files: {e}");
        // The files the agent is started with, in a folder made for this run
        // alone, which goes when it ends.
        let files = TurnFiles::new().map_err(unwritten)?;
        let looking_at = files
            .write("looking-at.json", &turn.looking_at)
            .map_err(unwritten)?;
        let server = McpServer::new(&self.own, &turn.set, &looking_at);
        let mut command = match turn.agent {
            Agent::Claude => {
                let config = files
                    .write("claude-mcp.json", &claude::config(&server))
                    .map_err(unwritten)?;
                let settings = files
                    .write("claude-settings.json", &claude::settings())
                    .map_err(unwritten)?;
                claude::command(&turn.program, &config, &settings, turn.session.as_deref())
            }
            Agent::Codex => codex::command(&turn.program, &server, turn.session.as_deref()),
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        own_group(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("{name} could not be started: {e}"))?;
        // The run is watched from the moment it starts: an agent that never
        // reads its prompt, never prints, or never ends is stopped, with
        // everything it started, at the deadline.
        let pid = child.id();
        let stdin = child.stdin.take();
        let prompt = turn.prompt.clone();
        // Each pipe is fed and read on a thread of its own, and none is
        // waited for without a bound: a process the agent started may hold
        // one open after the agent has gone.
        std::thread::spawn(move || {
            // An agent that stops before reading its prompt says why on its
            // standard error.
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(prompt.as_bytes());
            }
        });
        let stdout = child.stdout.take().expect("standard output is piped");
        let stderr = child.stderr.take().expect("standard error is piped");
        let said = Arc::new(Mutex::new(String::new()));
        {
            let said = Arc::clone(&said);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                let mut stderr = stderr;
                while let Ok(n) = stderr.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let mut text = said.lock().unwrap_or_else(|e| e.into_inner());
                    if text.len() < 1 << 20 {
                        text.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    }
                }
            });
        }
        let (lines, heard) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        // The run is watched from the moment it starts: an agent that never
        // reads its prompt, never prints, or never ends is stopped, with
        // everything it started, at the deadline. One that ends is given a
        // moment for what is still on its way, then whatever it left
        // running is stopped too.
        let started = Instant::now();
        let mut parts: HashMap<String, usize> = HashMap::new();
        let mut failed: Option<String> = None;
        let mut stopped = false;
        let mut status: Option<std::io::Result<std::process::ExitStatus>> = None;
        let mut ended: Option<Instant> = None;
        let mut closed = false;
        loop {
            if status.is_none() {
                match child.try_wait() {
                    Ok(Some(exit)) => {
                        status = Some(Ok(exit));
                        ended = Some(Instant::now());
                    }
                    Ok(None) => {}
                    Err(e) => {
                        status = Some(Err(e));
                        ended = Some(Instant::now());
                    }
                }
            }
            if closed && status.is_some() {
                break;
            }
            if ended.is_some_and(|at| at.elapsed() >= AFTER_EXIT) {
                break;
            }
            if status.is_none() && started.elapsed() >= turn.limit {
                stopped = true;
                kill_tree(pid);
                status = Some(child.wait());
                break;
            }
            if closed {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            match heard.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => self.read_line(turn, &line, &mut parts, &mut failed),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => closed = true,
            }
        }
        // Whatever the agent left running, holding its pipes, goes with it;
        // what it printed before is still read.
        if !stopped {
            kill_tree(pid);
        }
        while let Ok(line) = heard.try_recv() {
            self.read_line(turn, &line, &mut parts, &mut failed);
        }
        let status = status.unwrap_or_else(|| child.wait());
        let said = said.lock().unwrap_or_else(|e| e.into_inner()).clone();
        drop(files);
        if stopped {
            return Err(format!("{name} was stopped after {}", minutes(turn.limit)));
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

    /// Reads one line of the agent's output into the conversation, noting a
    /// failure it reports.
    fn read_line(
        &self,
        turn: &Turn,
        line: &str,
        parts: &mut HashMap<String, usize>,
        failed: &mut Option<String>,
    ) {
        let events = match turn.agent {
            Agent::Claude => claude::read(line),
            Agent::Codex => codex::read(line),
        };
        for event in events {
            if let AgentEvent::Failed(why) = &event {
                *failed = Some(why.clone());
            }
            self.apply(turn, parts, event);
        }
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
                let canonical_set = canonical_set_of(&input);
                list.push(json!({
                    "kind": "command",
                    "command": tool,
                    "input": input,
                    "reply": null,
                    "ok": null,
                    "canonical_set": canonical_set,
                }));
                parts.insert(id, list.len() - 1);
            }
            AgentEvent::Done { id, reply, ok } => match parts.get(&id) {
                Some(&at) => {
                    if list[at]["canonical_set"].is_null() {
                        if let Some(canonical_set) = reply.get("result").and_then(canonical_set_of)
                        {
                            list[at]["canonical_set"] = json!(canonical_set);
                        }
                    }
                    list[at]["reply"] = reply;
                    list[at]["ok"] = json!(ok);
                }
                None => {
                    let canonical_set = reply.get("result").and_then(canonical_set_of);
                    list.push(json!({
                        "kind": "command",
                        "command": "tool",
                        "input": {},
                        "reply": reply,
                        "ok": ok,
                        "canonical_set": canonical_set,
                    }));
                }
            },
            AgentEvent::Failed(_) => {}
        }
        talk.version += 1;
        drop(talks);
        self.changed.notify_all();
    }

    /// Ends a message's run: done, why it failed (if it did), and reconciles
    /// the conversation's agent against the latest PATH scan in case a rescan
    /// happened while the turn was running (#232).
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
            // Read `found` while holding `talks` (same lock order as `send`)
            // so a concurrent `apply_found` cannot slip between setting
            // `busy = false` and checking whether `talk.agent` is still present.
            let found = self.found();
            reconcile_talk(talk, &found);
        }
        drop(talks);
        self.changed.notify_all();
    }
}

/// Brings an idle conversation's agent into line with `found`: a conversation
/// with no agent takes the first one found, and one whose agent is gone
/// switches to another (or none) as a new conversation (#232).
fn reconcile_talk(talk: &mut Talk, found: &[(Agent, PathBuf)]) {
    let first = found.first().map(|(agent, _)| *agent);
    let gone = talk.agent.filter(|a| !found.iter().any(|(f, _)| f == a));
    if talk.busy || (talk.agent.is_some() && gone.is_none()) {
        return;
    }
    if talk.agent.is_none() && first.is_none() {
        return;
    }
    talk.agent = first;
    talk.session = None;
    let looked = match gone {
        Some(gone) => format!("Looked on the PATH again: {} is gone.", gone.name()),
        None => "Looked on the PATH again.".to_owned(),
    };
    let now = match first {
        Some(agent) => format!(
            "{looked} Now {} A new conversation.",
            started_words(agent, &talk.named).replacen("On ", "on ", 1)
        ),
        None => format!("{looked} No agent is found now."),
    };
    talk.note(now);
}

/// Resolves the canonical path of the `set` field on a command input or reply
/// result when present and reachable on disk (#227).
fn canonical_set_of(value: &Value) -> Option<PathBuf> {
    let raw = value.get("set").and_then(Value::as_str)?;
    std::fs::canonicalize(raw).ok()
}

/// The status a refusal the core gave is sent with: 400 for a wrong
/// request, 422 for any other.
fn status_of(error: &Error) -> u16 {
    if error.code == ErrorCode::Usage {
        400
    } else {
        422
    }
}

/// How long `limit` is, in words.
fn minutes(limit: Duration) -> String {
    match limit.as_secs() {
        s if s >= 60 && s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

/// Starts the agent in a process group of its own, on Unix, so the
/// watchdog can stop it with everything it started.
fn own_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// Stops the process `pid` and every process it started: its process group
/// on Unix, its tree on Windows. Its pipes close with them, so what reads
/// its output ends.
fn kill_tree(pid: u32) {
    let mut kill = if cfg!(windows) {
        let mut kill = Command::new("taskkill");
        kill.args(["/T", "/F", "/PID", &pid.to_string()]);
        kill
    } else {
        let mut kill = Command::new("kill");
        kill.args(["-KILL", "--", &format!("-{pid}")]);
        kill
    };
    let _ = kill
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
    /// The conversation's key: the set's canonical path, which the
    /// assistant's MCP server is bound to.
    set: PathBuf,
    agent: Agent,
    program: PathBuf,
    session: Option<String>,
    prompt: String,
    /// The `looking-at` reply for this message: what it carried, or that
    /// the drafter left it off.
    looking_at: Value,
    /// The conversation's item it fills.
    item: usize,
    /// How long it may run before it is stopped.
    limit: Duration,
}

/// The note a conversation on `agent` begins with.
fn started_words(agent: Agent, set: &Path) -> String {
    format!(
        "On {}: AgentDrafter started {} and pointed it at agentdrafter mcp, on {}.",
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
             drafter's drawing, and lands only when they keep it. When they ask how to do \
             something, call `guide` with the steps for them to do on their own drawing. Answer \
             in a few plain sentences.\n\n"
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
        None => prompt.push_str(&format!(
            "[The drafter left out what they are looking at for this message, on `{set}`: do \
             not assume a sheet or a selection.]\n\n"
        )),
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

    /// A shim in a folder of its own that runs the fake agent as Claude
    /// Code, with `setting` in its environment, such as `FAKE_AGENT_HANG=1`.
    fn fake_claude(name: &str, setting: &str) -> (PathBuf, PathBuf) {
        let folder =
            std::env::temp_dir().join(format!("agentdrafter-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-agent")
            .join("fake_agent.py");
        let (key, value) = setting.split_once('=').unwrap();
        let shim = if cfg!(windows) {
            let shim = folder.join("claude.cmd");
            std::fs::write(
                &shim,
                format!(
                    "@set {key}={value}
@python \"{}\" claude %*
",
                    script.display()
                ),
            )
            .unwrap();
            shim
        } else {
            let shim = folder.join("claude");
            std::fs::write(
                &shim,
                format!(
                    "#!/bin/sh
{key}={value} exec python3 \"{}\" claude \"$@\"
",
                    script.display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            shim
        };
        (folder, shim)
    }

    /// Runs one message on `shim` within `limit`, in a conversation of its
    /// own, and gives the conversation's answer, whether it is still busy,
    /// and how long the run took.
    fn run_on(folder: &Path, shim: PathBuf, limit: Duration) -> (Value, bool, Duration) {
        let set = folder.join("set");
        let assistant = assistant();
        assistant.lock().insert(
            set.clone(),
            Talk {
                named: set.clone(),
                agent: Some(Agent::Claude),
                session: None,
                busy: true,
                items: vec![
                    json!({"kind": "drafter", "text": "hello"}),
                    json!({"kind": "assistant", "parts": [], "done": false, "error": null}),
                ],
                version: 0,
            },
        );
        let began = Instant::now();
        assistant.run(Turn {
            set: set.clone(),
            agent: Agent::Claude,
            program: shim,
            session: None,
            prompt: "hello
"
            .into(),
            looking_at: json!({"ok": true}),
            item: 1,
            limit,
        });
        let took = began.elapsed();
        let talks = assistant.lock();
        let talk = &talks[&set];
        (talk.items[1].clone(), talk.busy, took)
    }

    #[test]
    fn an_agent_that_ends_leaving_a_process_holding_its_pipes_ends_its_turn() {
        let (folder, shim) = fake_claude("stray", "FAKE_AGENT_STRAY=1");
        let (answer, busy, took) = run_on(&folder, shim, Duration::from_secs(120));
        assert!(
            took < Duration::from_secs(30),
            "the turn ends at once: {took:?}"
        );
        assert!(!busy, "the chat leaves busy");
        assert_eq!(answer["done"], true);
        assert_eq!(answer["error"], Value::Null, "{answer}");
        assert_eq!(
            answer["parts"][0]["text"],
            "(fake agent) I left a helper running."
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_hung_agent_is_stopped_with_what_it_started_at_the_deadline() {
        // A shim that runs the fake agent hung, as Claude Code.
        let folder = std::env::temp_dir().join(format!("agentdrafter-hung-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-agent")
            .join("fake_agent.py");
        let shim = if cfg!(windows) {
            let shim = folder.join("claude.cmd");
            std::fs::write(
                &shim,
                format!(
                    "@set FAKE_AGENT_HANG=1
@python \"{}\" claude %*
",
                    script.display()
                ),
            )
            .unwrap();
            shim
        } else {
            let shim = folder.join("claude");
            std::fs::write(
                &shim,
                format!(
                    "#!/bin/sh
FAKE_AGENT_HANG=1 exec python3 \"{}\" claude \"$@\"
",
                    script.display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            shim
        };
        let set = folder.join("set");
        let assistant = assistant();
        {
            let mut talks = assistant.lock();
            talks.insert(
                set.clone(),
                Talk {
                    named: set.clone(),
                    agent: Some(Agent::Claude),
                    session: None,
                    busy: true,
                    items: vec![
                        json!({"kind": "drafter", "text": "hello"}),
                        json!({"kind": "assistant", "parts": [], "done": false, "error": null}),
                    ],
                    version: 0,
                },
            );
        }
        let began = Instant::now();
        assistant.run(Turn {
            set: set.clone(),
            agent: Agent::Claude,
            program: shim,
            session: None,
            prompt: "hello
"
            .into(),
            looking_at: json!({"ok": true}),
            item: 1,
            limit: Duration::from_secs(2),
        });
        let took = began.elapsed();
        assert!(
            took < Duration::from_secs(30),
            "stopped at the deadline: {took:?}"
        );
        let talks = assistant.lock();
        let talk = &talks[&set];
        assert!(!talk.busy, "the chat leaves busy");
        assert_eq!(talk.items[1]["done"], true);
        let error = talk.items[1]["error"].as_str().unwrap();
        assert_eq!(error, "Claude Code was stopped after 2 seconds");
        drop(talks);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_time_is_written_in_utc() {
        let now = utc_now();
        assert_eq!(now.len(), 20, "{now}");
        assert!(now.starts_with("20") && now.ends_with('Z'), "{now}");
    }

    #[test]
    fn conversation_snapshot_and_command_parts_carry_canonical_set_identity() {
        let folder =
            std::env::temp_dir().join(format!("agentdrafter-canonical-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let set_dir = folder.join("demo-plant");
        let other_dir = folder.join("other-plant");
        std::fs::create_dir_all(&set_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        let canonical = std::fs::canonicalize(&set_dir).unwrap();
        let other_canonical = std::fs::canonicalize(&other_dir).unwrap();
        // A differently spelled path to the same set (`demo-plant/../demo-plant`).
        let alt_spelling = set_dir.join("..").join("demo-plant");

        let assistant = Assistant {
            found: Mutex::new(Vec::new()),
            talks: Mutex::new(HashMap::new()),
            changed: Condvar::new(),
            own: vec!["agentdrafter".into()],
        };
        {
            let mut talks = assistant.lock();
            talks.insert(
                canonical.clone(),
                Talk {
                    named: alt_spelling.clone(),
                    agent: Some(Agent::Claude),
                    session: None,
                    busy: true,
                    items: vec![
                        json!({"kind": "drafter", "text": "move it"}),
                        json!({"kind": "assistant", "parts": [], "done": false, "error": null}),
                    ],
                    version: 1,
                },
            );
        }
        let turn = Turn {
            set: canonical.clone(),
            agent: Agent::Claude,
            program: PathBuf::from("claude"),
            session: None,
            prompt: String::new(),
            looking_at: json!({"ok": true}),
            item: 1,
            limit: Duration::from_secs(5),
        };
        let mut parts = HashMap::new();
        // 1. `propose` naming the alternate spelling in `input.set` resolves to `canonical`.
        assistant.apply(
            &turn,
            &mut parts,
            AgentEvent::Started {
                id: "t1".into(),
                tool: "propose".into(),
                input: json!({"set": alt_spelling}),
            },
        );
        assistant.apply(
            &turn,
            &mut parts,
            AgentEvent::Done {
                id: "t1".into(),
                reply: json!({"ok": true, "result": {"proposal": 1}}),
                ok: true,
            },
        );
        // 2. `guide` whose `set` appears only in `reply.result.set` resolves to `canonical`.
        assistant.apply(
            &turn,
            &mut parts,
            AgentEvent::Started {
                id: "t2".into(),
                tool: "guide".into(),
                input: json!({}),
            },
        );
        assistant.apply(
            &turn,
            &mut parts,
            AgentEvent::Done {
                id: "t2".into(),
                reply: json!({"ok": true, "result": {"guide": 2, "set": alt_spelling}}),
                ok: true,
            },
        );
        // 3. A command naming another set gets that other set's canonical path.
        assistant.apply(
            &turn,
            &mut parts,
            AgentEvent::Started {
                id: "t3".into(),
                tool: "propose".into(),
                input: json!({"set": other_dir}),
            },
        );

        let (_, line) = assistant.snapshot(&canonical);
        let parsed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["result"]["canonical_set"], json!(canonical));
        let recorded = parsed["result"]["items"][1]["parts"].as_array().unwrap();
        assert_eq!(recorded[0]["canonical_set"], json!(canonical));
        assert_eq!(recorded[1]["canonical_set"], json!(canonical));
        assert_eq!(recorded[2]["canonical_set"], json!(other_canonical));

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn rescan_during_busy_turn_reconciles_agent_when_turn_finishes() {
        let set_a = PathBuf::from("set-a");
        let set_b = PathBuf::from("set-b");
        let set_c = PathBuf::from("set-c");
        let make_talk = |named: &Path, agent: Agent, session: &str| Talk {
            named: named.to_owned(),
            agent: Some(agent),
            session: Some(session.to_owned()),
            busy: true,
            items: vec![
                json!({"kind": "drafter", "text": "hello"}),
                json!({"kind": "assistant", "parts": [], "done": false, "error": null}),
            ],
            version: 1,
        };

        // 1. Active agent disappears during turn, another agent (Codex) remains.
        let assistant = Assistant {
            found: Mutex::new(vec![
                (Agent::Claude, PathBuf::from("/bin/claude")),
                (Agent::Codex, PathBuf::from("/bin/codex")),
            ]),
            talks: Mutex::new(HashMap::new()),
            changed: Condvar::new(),
            own: vec!["agentdrafter".into()],
        };
        {
            let mut talks = assistant.lock();
            talks.insert(set_a.clone(), make_talk(&set_a, Agent::Claude, "sess-1"));
            talks.insert(set_b.clone(), make_talk(&set_b, Agent::Codex, "sess-2"));
        }

        // Rescan while both turns are busy: Claude is gone, Codex remains.
        assistant.apply_found(vec![(Agent::Codex, PathBuf::from("/bin/codex"))]);
        {
            let talks = assistant.lock();
            assert_eq!(
                talks[&set_a].agent,
                Some(Agent::Claude),
                "busy turn is left undisturbed until it finishes"
            );
            assert_eq!(talks[&set_a].items.len(), 2);
        }

        // Finishing set_a's turn reconciles it onto Codex and clears the stale session.
        assistant.finish(&set_a, 1, None);
        {
            let talks = assistant.lock();
            let talk = &talks[&set_a];
            assert!(!talk.busy);
            assert_eq!(talk.agent, Some(Agent::Codex));
            assert_eq!(talk.session, None);
            assert_eq!(talk.items[1]["done"], true);
            let note = talk.items[2]["text"].as_str().unwrap();
            assert!(
                note.contains("Claude Code is gone") && note.contains("Codex"),
                "reconcile note: {note}"
            );
        }

        // Finishing set_b's turn (whose Codex is still present) keeps Codex and its session.
        assistant.finish(&set_b, 1, None);
        {
            let talks = assistant.lock();
            let talk = &talks[&set_b];
            assert!(!talk.busy);
            assert_eq!(talk.agent, Some(Agent::Codex));
            assert_eq!(talk.session.as_deref(), Some("sess-2"));
            assert_eq!(talk.items.len(), 2, "no spurious note when agent remains");
        }

        // 2. Active agent disappears during turn and no agents remain.
        {
            let mut talks = assistant.lock();
            talks.insert(set_c.clone(), make_talk(&set_c, Agent::Codex, "sess-3"));
        }
        assistant.apply_found(Vec::new());
        assistant.finish(&set_c, 1, None);
        {
            let talks = assistant.lock();
            let talk = &talks[&set_c];
            assert!(!talk.busy);
            assert_eq!(talk.agent, None);
            assert_eq!(talk.session, None);
            let note = talk.items[2]["text"].as_str().unwrap();
            assert!(
                note.contains("Codex is gone") && note.contains("No agent is found now"),
                "no-agent reconcile note: {note}"
            );
        }
    }
}
