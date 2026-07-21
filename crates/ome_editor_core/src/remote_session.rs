//! Client-side handle to a project running in `--remote` mode.
//!
//! The standalone editor cannot compile a project's component types, so
//! instead of loading the project it **launches** it (`cargo run --
//! --remote`) and drives its ECS over HTTP through
//! [`ome_remote::RemoteClient`]. This is the editor's half of the remote
//! protocol; the project's half is [`ome_remote::RemotePlugin`].
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
use std::time::Duration;

use ome_remote::protocol::{ComponentSchema, EntitySnapshot};
use ome_remote::{DEFAULT_PORT, RemoteClient};

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
}

impl RemoteSession {
    /// Launches `cargo run -- --remote` for the project at `manifest_path`
    /// and returns a session in [`ConnectionState::Connecting`].
    ///
    /// `engine_root` is forwarded as `OME_ENGINE_ROOT` so the project's
    /// asset pipeline resolves engine-shipped assets — without it the
    /// remote world loads but renders no meshes (see
    /// [`PlayState::launch`](crate::play_state::PlayState::launch)).
    ///
    /// The build/boot is asynchronous: the returned session is not yet
    /// connected. Drive [`Self::poll_ready`] each frame until it reports
    /// [`ConnectionState::Connected`].
    pub fn launch(manifest_path: &Path, engine_root: Option<&Path>) -> std::io::Result<Self> {
        let output = Arc::new(Mutex::new(Vec::new()));

        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("--manifest-path")
            .arg(manifest_path)
            .arg("--")
            .arg("--remote")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(engine_root) = engine_root {
            cmd.env("OME_ENGINE_ROOT", engine_root);
        }
        if let Some(project_root) = manifest_path.parent() {
            cmd.env("OME_PROJECT_ROOT", project_root);
        }
        if std::env::var_os("RUST_LOG").is_none() {
            cmd.env("RUST_LOG", "info");
        }

        let mut child = cmd.spawn()?;
        capture(child.stdout.take(), &output);
        capture(child.stderr.take(), &output);

        tracing::info!("remote project launching");
        Ok(Self {
            child: Some(child),
            output,
            client: RemoteClient::new(DEFAULT_PORT).with_timeout(Duration::from_millis(500)),
            state: ConnectionState::Connecting,
            snapshot: Vec::new(),
            schema: Vec::new(),
        })
    }

    /// Attaches to a server already listening on `port`, launching no
    /// process. Used to drive an externally-run project and by tests.
    pub fn attach(port: u16) -> Self {
        Self {
            child: None,
            output: Arc::new(Mutex::new(Vec::new())),
            client: RemoteClient::new(port).with_timeout(Duration::from_millis(500)),
            state: ConnectionState::Connecting,
            snapshot: Vec::new(),
            schema: Vec::new(),
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

    /// The component schema pulled on connect.
    pub fn schema(&self) -> &[ComponentSchema] {
        &self.schema
    }

    /// The underlying client, for issuing edits (set field, add/remove).
    pub fn client(&self) -> &RemoteClient {
        &self.client
    }

    /// While [`ConnectionState::Connecting`], pings the server; on the
    /// first success, pulls the schema + an initial snapshot and moves to
    /// [`ConnectionState::Connected`]. If the child process exited before
    /// connecting, moves to [`ConnectionState::Failed`].
    ///
    /// A no-op once connected. Cheap while connecting: a refused
    /// connection returns immediately, so this can run every frame during
    /// the project's build without stalling.
    pub fn poll_ready(&mut self) -> ConnectionState {
        if self.state != ConnectionState::Connecting {
            return self.state;
        }
        if self.child_exited() {
            self.state = ConnectionState::Failed;
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
    /// connected. Errors are logged and leave the previous snapshot in
    /// place, so a transient hiccup does not blank the editor.
    pub fn refresh(&mut self) {
        if self.state != ConnectionState::Connected {
            return;
        }
        match self.client.list_entities() {
            Ok(entities) => self.snapshot = entities,
            Err(e) => tracing::debug!("remote refresh failed: {e}"),
        }
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
