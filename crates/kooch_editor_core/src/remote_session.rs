//! Client-side handle to a project running in `--remote` mode.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use kooch_remote::protocol::{ComponentSchema, EntityId, EntitySnapshot};
use kooch_remote::{NAME_ENV, RemoteClient};

use crate::remote_mirror::RemoteMirror;

/// The editor's remote-mode state: the active session (if any) and the local mirror of its scene.
#[derive(Default)]
pub struct RemoteState {
    /// The launched project, or `None` in local mode.
    pub session: Option<RemoteSession>,
    /// The local ECS reconstruction of the remote scene.
    pub mirror: RemoteMirror,
    /// Whether the project is running its gameplay systems.
    pub playing: bool,
    /// Whether the last input snapshot sent to the host was an idle one.
    pub last_input_was_idle: bool,
    /// Entities the project has just created for us, waiting to be selected once the mirror knows
    /// about them.
    pub pending_selection: Vec<kooch_remote::protocol::EntityId>,
    /// The last line the project said while connecting, and every line of it.
    pub connect_output: Vec<String>,
}

impl RemoteState {
    /// Creates empty (local-mode) state.
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when a session exists and has connected — the condition
    /// under which edits route to the server instead of the local ECS.
    pub fn is_connected(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.state() == ConnectionState::Connected)
    }
}

/// Where a session is in its connect handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// The project is launching or booting; the server has not answered a
    /// ping yet.
    Connecting,
    /// The server answered; the snapshot is live.
    Connected,
    /// The child process exited before the server ever answered.
    Failed,
}

/// A launched-and-connected (or connecting) remote project.
pub struct RemoteSession {
    /// The project child process, if this session launched one. `None`
    /// when attached to an already-running server (tests, external run).
    child: Option<Child>,
    /// Captured child stdout/stderr, newest last.
    output: Arc<Mutex<Vec<String>>>,
    /// HTTP client bound to the project's server port.
    ///
    /// Shared so [`MovedPump`] can call it from its own thread.
    client: Arc<RemoteClient>,
    state: ConnectionState,
    /// Last entity snapshot pulled by [`Self::refresh`].
    snapshot: Vec<EntitySnapshot>,
    /// What the project last said its own frame cost. The editor's HUD
    /// shows its own frame; this is the process that is actually
    /// simulating (#699).
    host_metrics: Option<kooch_remote::protocol::HostMetrics>,
    /// The scenes the project has open, as it last reported them.
    scenes: Option<Vec<kooch_remote::protocol::SceneEntry>>,
    /// What the project schedules, and which of it is running.
    systems: Option<Vec<kooch_remote::protocol::SystemEntry>>,
    /// Whether the last [`Self::refresh`] actually changed the world.
    changed_last_refresh: bool,
    /// The revision the project last handed out, passed back on the next pull so it can answer with
    /// a diff.
    revision: Option<u64>,
    /// The worker that keeps the play-mode transform delta fresh.
    pump: Option<crate::moved_pump::MovedPump>,
    /// Component schema, pulled once on connect.
    schema: Vec<ComponentSchema>,
    /// Why the snapshot stopped tracking the project, or `None` while it tracks.
    stale: Option<String>,
}

impl RemoteSession {
    /// Launches `cargo run -- --remote` for the project at `manifest_path` and returns a session in
    /// [`ConnectionState::Connecting`].
    pub fn launch(manifest_path: &Path, engine_root: Option<&Path>) -> std::io::Result<Self> {
        let output = Arc::new(Mutex::new(Vec::new()));

        // A name unique to this launch. The old fixed port meant an orphaned project — one that
        // outlived a crashed editor — still held it, so the next editor connected to *that* and
        // mirrored a dead session's world in silence.
        let socket = unique_socket_name();

        let mut cmd = Command::new("cargo");
        cmd.arg("run").arg("--manifest-path").arg(manifest_path);
        // The remote server lives behind the project's `editor` feature, in a binary a game build
        // does not produce (#558). Named explicitly because the default `--bin` is the game, and
        // the game does not answer a socket.
        crate::cargo_args::authoring(&mut cmd);
        cmd.arg("--bin")
            .arg(crate::cargo_args::editor_bin(
                &crate::cargo_args::crate_name(manifest_path),
            ))
            .arg("--")
            .arg("--remote")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(engine_root) = engine_root {
            cmd.env("KOOCH_ENGINE_ROOT", engine_root);
        }
        if let Some(project_root) = manifest_path.parent() {
            cmd.env("KOOCH_PROJECT_ROOT", project_root);
            // `cargo run --manifest-path` does NOT move the child's working directory to the
            // manifest's folder — it inherits the editor's.
            cmd.current_dir(project_root);
        }
        cmd.env(NAME_ENV, &socket);
        if std::env::var_os("RUST_LOG").is_none() {
            cmd.env("RUST_LOG", "info");
        }
        // Unconditional, unlike the filter above: the editor reads this rather than a person, and a
        // formatted line arrives as one opaque string that loses the level and target the Console
        // filters on.
        cmd.env("KOOCH_LOG_FORMAT", "json");

        let mut child = cmd.spawn()?;
        capture(child.stdout.take(), &output);
        capture(child.stderr.take(), &output);

        tracing::info!("remote project launching");
        Ok(Self {
            child: Some(child),
            output,
            client: Arc::new(RemoteClient::new(socket)),
            state: ConnectionState::Connecting,
            snapshot: Vec::new(),
            host_metrics: None,
            scenes: None,
            systems: None,
            changed_last_refresh: true,
            revision: None,
            pump: None,
            schema: Vec::new(),
            stale: None,
        })
    }

    /// Attaches to a server already listening on `socket`, launching no
    /// process. Used to drive an externally-run project and by tests.
    pub fn attach(socket: impl Into<String>) -> Self {
        Self {
            child: None,
            output: Arc::new(Mutex::new(Vec::new())),
            client: Arc::new(RemoteClient::new(socket)),
            state: ConnectionState::Connecting,
            snapshot: Vec::new(),
            host_metrics: None,
            scenes: None,
            systems: None,
            changed_last_refresh: true,
            revision: None,
            pump: None,
            schema: Vec::new(),
            stale: None,
        }
    }

    /// The current handshake state.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// The last pulled entity snapshot.
    pub fn snapshot(&self) -> &[EntitySnapshot] {
        &self.snapshot
    }

    /// Puts the session into `Connected` with a given schema, without a server.
    #[cfg(test)]
    pub(crate) fn connected_with_schema_for_test(&mut self, schema: Vec<ComponentSchema>) {
        self.state = ConnectionState::Connected;
        self.schema = schema;
    }

    /// The component schema pulled on connect.
    pub fn schema(&self) -> &[ComponentSchema] {
        &self.schema
    }

    /// The underlying client, for issuing edits (set field, add/remove).
    pub fn client(&self) -> &RemoteClient {
        &self.client
    }

    /// Advances the session's state one step.
    pub fn poll_ready(&mut self) -> ConnectionState {
        if self.state == ConnectionState::Failed {
            return self.state;
        }
        if self.child_exited() {
            self.state = ConnectionState::Failed;
            return self.state;
        }
        if self.state == ConnectionState::Connected {
            return self.state;
        }
        if self.client.ping().is_ok() {
            self.schema = self.client.get_schema().unwrap_or_default();
            // Through the `since` form even though there is nothing to diff against: the plain one
            // returns entities alone, and the open scene set would then be unknown until the first
            // refresh — one frame of the panel listing nothing.
            if let Ok(update) = self.client.list_entities_since(None) {
                self.snapshot = update.entities;
                self.scenes = update.scenes;
            }
            self.state = ConnectionState::Connected;
            tracing::info!("remote project connected");
        }
        self.state
    }

    /// Re-pulls the entity snapshot from the server. No-op unless connected.
    pub fn changed_last_refresh(&self) -> bool {
        self.changed_last_refresh
    }

    /// What the project's process last reported its own frame cost to be.
    pub fn host_metrics(&self) -> Option<kooch_remote::protocol::HostMetrics> {
        self.host_metrics
    }

    /// The scenes the project has open, or `None` if it has not said.
    pub fn open_scenes(&self) -> Option<&[kooch_remote::protocol::SceneEntry]> {
        self.scenes.as_deref()
    }

    /// What the project schedules, or `None` if it has not said.
    pub fn systems(&self) -> Option<&[kooch_remote::protocol::SystemEntry]> {
        self.systems.as_deref()
    }

    /// Asks the project what it schedules, and whether each is running.
    pub fn refresh_systems(&mut self) {
        if self.state != ConnectionState::Connected {
            return;
        }
        match self.client.list_systems() {
            Ok(systems) => self.systems = Some(systems),
            // Left as it was. A host too old to answer should leave the
            // panel showing what it last knew rather than emptying it.
            Err(e) => tracing::debug!("the project did not list its systems: {e}"),
        }
    }

    /// Turns the background transform pull on or off (#1014).
    pub fn set_pulling(&mut self, pulling: bool) {
        if !pulling && self.pump.is_none() {
            return;
        }
        let client = Arc::clone(&self.client);
        self.pump
            .get_or_insert_with(|| crate::moved_pump::MovedPump::spawn(client))
            .set_running(pulling);
    }

    /// The play-mode pull: what moved, and nothing else (#1012).
    pub fn refresh_moved(&mut self) -> Option<Vec<kooch_remote::protocol::MovedTransform>> {
        if self.state != ConnectionState::Connected {
            return Some(Vec::new());
        }
        let Some(pump) = self.pump.as_ref() else {
            // `set_pulling` has not run yet. Nothing has been asked, so
            // there is nothing to answer with; the full pull covers it.
            return None;
        };
        let mut pulled = Vec::new();
        pump.drain(&mut pulled);

        let mut moved = Vec::new();
        // Either the project declined or the exchange failed. In both
        // cases what this side holds cannot be diffed onto.
        let mut structural = false;
        for reply in pulled {
            match reply {
                crate::moved_pump::Pulled::Update(update) => {
                    if update.host.is_some() {
                        self.host_metrics = update.host;
                    }
                    // A despawn is structure. Let the full path handle
                    // it rather than teaching the cheap one to unmap ids.
                    if update.full || !update.removed.is_empty() {
                        structural = true;
                        moved.clear();
                    } else {
                        moved.extend(update.moved);
                    }
                }
                crate::moved_pump::Pulled::Failed(reason) => {
                    structural = true;
                    moved.clear();
                    if self.stale.replace(reason.clone()).is_none() {
                        tracing::warn!(
                            "the remote snapshot stopped updating: {reason}. \
                             The editor is showing the last world it could read",
                        );
                    }
                }
            }
        }
        self.changed_last_refresh = !moved.is_empty();
        if structural {
            // The SNAPSHOT revision is the one that must not be trusted
            // now: the full pull that follows has to be a full one.
            self.revision = None;
            return None;
        }
        Some(moved)
    }

    pub fn refresh(&mut self) {
        if self.state != ConnectionState::Connected {
            return;
        }
        match self.client.list_entities_since(self.revision) {
            Ok(update) => {
                // `full` is the project's decision, not ours: it sends everything whenever it
                // cannot honour the revision we hold. Merging a full reply would keep entities it
                // had deliberately left out.
                self.changed_last_refresh =
                    update.full || !update.entities.is_empty() || !update.removed.is_empty();
                if update.full {
                    self.snapshot = update.entities;
                } else {
                    merge_into(&mut self.snapshot, update.entities, &update.removed);
                }
                self.revision = Some(update.revision);
                // Kept rather than overwritten with `None`: a pull that reaches an older host, or
                // one that has not finished its first frame, should leave the last known numbers on
                // screen instead of blanking them every other frame.
                if update.host.is_some() {
                    self.host_metrics = update.host;
                }
                // Same reasoning as the metrics above, and it matters more: blanking the open set
                // on a reply from an older host would empty the World panel of every scene while
                // the entities that belong to them keep arriving.
                if update.scenes.is_some() {
                    self.scenes = update.scenes;
                }
                if self.stale.take().is_some() {
                    tracing::info!("remote snapshot is tracking the project again");
                }
            }
            Err(e) => {
                // Drop the revision: the next pull has to be a full one. The snapshot we keep
                // showing is the last good world, and a diff computed against a revision we may
                // have diverged from would layer new errors on top of it.
                self.revision = None;
                // The snapshot is unchanged because the pull failed, not because the world stood
                // still. Saying "nothing changed" would be true and misleading — but it is also
                // harmless here, since the mirror already matches what we hold.
                self.changed_last_refresh = false;
                let reason = e.to_string();
                if self.stale.replace(reason.clone()).is_none() {
                    tracing::warn!(
                        "the remote snapshot stopped updating: {reason}. \
                         The editor is showing the last world it could read",
                    );
                }
            }
        }
    }

    /// Why the snapshot stopped tracking the project, or `None` while it
    /// tracks. Shown in the menu bar's remote indicator.
    pub fn stale_reason(&self) -> Option<&str> {
        self.stale.as_deref()
    }

    /// Drains captured child output lines.
    pub fn drain_output(&self) -> Vec<String> {
        self.output
            .lock()
            .map(|mut buf| std::mem::take(&mut *buf))
            .unwrap_or_default()
    }

    /// Kills the child process, if any.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            tracing::info!("remote project stopped");
        }
    }

    /// `true` once the child process has exited (or if there is no child).
    fn child_exited(&mut self) -> bool {
        match self.child.as_mut() {
            None => false,
            Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
        }
    }
}

impl Drop for RemoteSession {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A socket name no other launch will produce.
fn unique_socket_name() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("kooch_{}_{}.sock", std::process::id(), n)
}

/// Spawns a thread that appends each line of `stream` to `sink`.
fn capture<R>(stream: Option<R>, sink: &Arc<Mutex<Vec<String>>>)
where
    R: std::io::Read + Send + 'static,
{
    let Some(stream) = stream else { return };
    let sink = Arc::clone(sink);
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(mut buf) = sink.lock() {
                buf.push(line);
            }
        }
    });
}

/// Folds a diff into a world.
fn merge_into(
    snapshot: &mut Vec<EntitySnapshot>,
    changed: Vec<EntitySnapshot>,
    removed: &[EntityId],
) {
    if !removed.is_empty() {
        snapshot.retain(|e| !removed.contains(&e.id));
    }
    for entity in changed {
        match snapshot.iter_mut().find(|e| e.id == entity.id) {
            Some(existing) => *existing = entity,
            None => snapshot.push(entity),
        }
    }
    // The project sends its world sorted by index and downstream reads
    // that as authored order; appending would put every new entity last
    // regardless of where it belongs.
    snapshot.sort_by_key(|e| e.id.index);
}

#[cfg(test)]
mod merge_tests;

#[cfg(test)]
mod changed_flag_tests;
