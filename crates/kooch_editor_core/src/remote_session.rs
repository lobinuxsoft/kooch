//! Client-side handle to a project running in `--remote` mode.
//!
//! The standalone editor cannot compile a project's component types, so
//! instead of loading the project it **launches** it (`cargo run --
//! --remote`) and drives its ECS over HTTP through
//! [`kooch_remote::RemoteClient`]. This is the editor's half of the remote
//! protocol; the project's half is [`kooch_remote::RemotePlugin`].
//!
//! The lifecycle mirrors [`PlayState`](crate::play_state::PlayState): the
//! child process is spawned with its stdout/stderr captured, polled for
//! exit, and killed on drop. On top of that this holds the client and a
//! cached snapshot of the remote world, refreshed on demand so the
//! editor's panels can render remote state through the same DTOs they use
//! for a local ECS.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use kooch_remote::protocol::{ComponentSchema, EntitySnapshot};
use kooch_remote::{NAME_ENV, RemoteClient};

use crate::remote_mirror::RemoteMirror;

/// The editor's remote-mode state: the active session (if any) and the
/// local mirror of its scene.
///
/// A single resource so the session and the mirror it feeds never drift.
/// `session == None` means the editor is in ordinary local mode; a
/// `Some` session that is [`ConnectionState::Connected`] is what flips
/// the edit dispatch to route through the wire.
#[derive(Default)]
pub struct RemoteState {
    /// The launched project, or `None` in local mode.
    pub session: Option<RemoteSession>,
    /// The local ECS reconstruction of the remote scene.
    pub mirror: RemoteMirror,
    /// Whether the project is running its gameplay systems.
    ///
    /// Mirrors the `Playing` gate on the project's side. A freshly
    /// connected project always starts paused, so this starts `false`
    /// and only the editor's Play/Stop moves it.
    pub playing: bool,
    /// The last line the project said while connecting, and every line
    /// of it.
    ///
    /// The output is drained into the log each frame, so without keeping
    /// a copy there is nothing left to *show*: opening a project builds
    /// it — twenty-two seconds, measured — and the editor looked dead for
    /// all of it (#672). The tail is what a progress banner reads; the
    /// whole thing is what a failed build needs to be copyable.
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
///
/// Stored as an editor resource. `None` of its network calls block for
/// longer than the client's short per-request timeout, so a stalled
/// project cannot wedge the editor's frame loop.
pub struct RemoteSession {
    /// The project child process, if this session launched one. `None`
    /// when attached to an already-running server (tests, external run).
    child: Option<Child>,
    /// Captured child stdout/stderr, newest last.
    output: Arc<Mutex<Vec<String>>>,
    /// HTTP client bound to the project's server port.
    client: RemoteClient,
    state: ConnectionState,
    /// Last entity snapshot pulled by [`Self::refresh`].
    snapshot: Vec<EntitySnapshot>,
    /// Component schema, pulled once on connect.
    schema: Vec<ComponentSchema>,
    /// Why the snapshot stopped tracking the project, or `None` while it
    /// tracks.
    ///
    /// A refresh that fails leaves the previous snapshot in place, which
    /// is right for a hiccup and a lie for anything lasting: the editor
    /// goes on showing a world that no longer exists and answering edits
    /// against it. A stale mirror has to be visibly stale.
    stale: Option<String>,
}

impl RemoteSession {
    /// Launches `cargo run -- --remote` for the project at `manifest_path`
    /// and returns a session in [`ConnectionState::Connecting`].
    ///
    /// `engine_root` is forwarded as `KOOCH_ENGINE_ROOT` so the project's
    /// asset pipeline resolves engine-shipped assets — without it the
    /// remote world loads but renders no meshes (see
    /// [`PlayState::launch`](crate::play_state::PlayState::launch)).
    ///
    /// The build/boot is asynchronous: the returned session is not yet
    /// connected. Drive [`Self::poll_ready`] each frame until it reports
    /// [`ConnectionState::Connected`].
    pub fn launch(manifest_path: &Path, engine_root: Option<&Path>) -> std::io::Result<Self> {
        let output = Arc::new(Mutex::new(Vec::new()));

        // A name unique to this launch. The old fixed port meant an
        // orphaned project — one that outlived a crashed editor — still
        // held it, so the next editor connected to *that* and mirrored a
        // dead session's world in silence. Yesterday's process cannot
        // answer to a name minted today.
        let socket = unique_socket_name();

        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--manifest-path")
            .arg(manifest_path)
            .arg("--")
            .arg("--remote")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(engine_root) = engine_root {
            cmd.env("KOOCH_ENGINE_ROOT", engine_root);
        }
        if let Some(project_root) = manifest_path.parent() {
            cmd.env("KOOCH_PROJECT_ROOT", project_root);
            // `cargo run --manifest-path` does NOT move the child's
            // working directory to the manifest's folder — it inherits
            // the editor's. Without this the project resolves its boot
            // scene (`scenes/default.scene`, cwd-relative) against
            // the editor's directory and comes up with an empty world.
            cmd.current_dir(project_root);
        }
        cmd.env(NAME_ENV, &socket);
        if std::env::var_os("RUST_LOG").is_none() {
            cmd.env("RUST_LOG", "info");
        }
        // Unconditional, unlike the filter above: the editor reads this
        // rather than a person, and a formatted line arrives as one opaque
        // string that loses the level and target the Console filters on.
        // Someone who set `RUST_LOG` wanted different *levels*, not a
        // different wire format.
        cmd.env("KOOCH_LOG_FORMAT", "json");

        let mut child = cmd.spawn()?;
        capture(child.stdout.take(), &output);
        capture(child.stderr.take(), &output);

        tracing::info!("remote project launching");
        Ok(Self {
            child: Some(child),
            output,
            client: RemoteClient::new(socket),
            state: ConnectionState::Connecting,
            snapshot: Vec::new(),
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
            client: RemoteClient::new(socket),
            state: ConnectionState::Connecting,
            snapshot: Vec::new(),
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

    /// Puts the session into `Connected` with a given schema, without a
    /// server.
    ///
    /// Test-only. The real path pulls the schema during the handshake; a
    /// test about *what the editor does with a schema* should not have to
    /// stand up a socket to state one.
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
    ///
    /// While [`ConnectionState::Connecting`], pings the server; on the
    /// first success, pulls the schema + an initial snapshot and moves to
    /// [`ConnectionState::Connected`]. Once connected it keeps watching
    /// the child: a project that crashes or is closed drops to
    /// [`ConnectionState::Failed`] rather than leaving the editor driving
    /// a process that is no longer there.
    ///
    /// Cheap in every state: a refused connection returns immediately and
    /// the liveness check is a non-blocking wait, so this can run every
    /// frame during the project's build without stalling.
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
            self.snapshot = self.client.list_entities().unwrap_or_default();
            self.state = ConnectionState::Connected;
            tracing::info!("remote project connected");
        }
        self.state
    }

    /// Re-pulls the entity snapshot from the server. No-op unless
    /// connected. The previous snapshot survives a failure, so a transient
    /// hiccup does not blank the editor — but the session is marked stale
    /// until one succeeds, because a mirror that stopped tracking looks
    /// exactly like a world where nothing happens to be moving.
    ///
    /// The complaint is `warn!` and it is said once. It used to be
    /// `debug!`, invisible under `RUST_LOG=info`, so a snapshot that froze
    /// for good did so in silence.
    pub fn refresh(&mut self) {
        if self.state != ConnectionState::Connected {
            return;
        }
        match self.client.list_entities() {
            Ok(entities) => {
                self.snapshot = entities;
                if self.stale.take().is_some() {
                    tracing::info!("remote snapshot is tracking the project again");
                }
            }
            Err(e) => {
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
///
/// The editor's pid plus a counter: unique across concurrent editors and
/// across relaunches of one, which is the property the old fixed port
/// lacked. Kept short and alphanumeric because Windows named pipes and
/// Linux abstract sockets have different rules about what a name may
/// contain, and the intersection is narrow.
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
