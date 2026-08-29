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

use kooch_remote::protocol::{ComponentSchema, EntityId, EntitySnapshot};
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
    /// Whether the last input snapshot sent to the host was an idle one.
    ///
    /// The gate that stops a resting keyboard costing a round trip every
    /// frame. Starts `false` so the first snapshot always goes: it is the
    /// one that releases whatever the host thinks is still held.
    pub last_input_was_idle: bool,
    /// Entities the project has just created for us, waiting to be
    /// selected once the mirror knows about them.
    ///
    /// 🔴 A creation cannot select what it made on the spot. The project
    /// answers with *its* id, and the editor's selection is made of
    /// mirror handles that do not exist until the next snapshot arrives.
    /// So the intent is parked here and spent by the sync — which is the
    /// difference between duplicating an entity and duplicating an
    /// entity, then hunting for it in a list of six hundred.
    pub pending_selection: Vec<kooch_remote::protocol::EntityId>,
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
    ///
    /// Kept whole even though the project now sends only what changed:
    /// the delta is folded in here, so everything downstream — the
    /// mirror, the panels, the stats — still sees the entire world and
    /// did not have to learn about revisions.
    snapshot: Vec<EntitySnapshot>,
    /// What the project last said its own frame cost. The editor's HUD
    /// shows its own frame; this is the process that is actually
    /// simulating (#699).
    host_metrics: Option<kooch_remote::protocol::HostMetrics>,
    /// The scenes the project has open, as it last reported them.
    ///
    /// 🔴 This is the list the World panel must draw, and the editor
    /// cannot supply it. The editor's own `SceneManager` seeds one
    /// unsaved scene with a random id at startup — reasonable for local
    /// mode, meaningless here — while the project holds the real files
    /// under ids of its own. Drawn from the editor's copy, the panel
    /// showed an `Untitled` scene nothing belongs to and filed every
    /// mirrored entity under "Unsaved", since the scene each one names
    /// was in nobody's list.
    ///
    /// `None` until the project answers, which is what keeps local mode
    /// reading its own manager. `Some(vec![])` is a project that has
    /// closed every scene — a different thing, and the panel should show
    /// it rather than falling back to a list the editor made up.
    scenes: Option<Vec<kooch_remote::protocol::SceneEntry>>,
    /// Whether the last [`Self::refresh`] actually changed the world.
    ///
    /// The mirror walks every entity to apply a snapshot, which costs
    /// about 7.5 ms on 610 of them — more than the pull itself now that
    /// the pull is a diff (#691). A delta that carried nothing means the
    /// mirror would rediscover, entity by entity, that nothing moved.
    ///
    /// Starts `true` so the world that arrives with the handshake is
    /// applied: at that point the mirror is empty and the snapshot is
    /// not.
    changed_last_refresh: bool,
    /// The revision the project last handed out, passed back on the next
    /// pull so it can answer with a diff.
    ///
    /// `None` until the first reply, and reset by anything that makes
    /// the local snapshot untrustworthy — a failed refresh leaves the
    /// old world in place, and diffing onto a world we are not sure of
    /// would compound the error silently.
    revision: Option<u64>,
    /// The revision of the last [`Self::refresh_moved`] reply.
    ///
    /// 🔴 Its OWN counter, and that is not tidiness. The host keeps a
    /// cache per method and each bumps only when its own reply carried
    /// something, so the two numbers diverge the moment the editor uses
    /// both. Sharing one made every cheap pull hand the host a revision
    /// it had never issued, which it correctly answered `full` — so the
    /// full pull ran anyway, every frame, and the cheap path was dead
    /// code that measured as no improvement at all.
    moved_revision: Option<u64>,
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
        cmd.arg("run").arg("--manifest-path").arg(manifest_path);
        // The remote server lives behind the project's `editor` feature,
        // in a binary a game build does not produce (#558). Named
        // explicitly because the default `--bin` is the game, and the
        // game does not answer a socket.
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
            host_metrics: None,
            scenes: None,
            changed_last_refresh: true,
            revision: None,
            moved_revision: None,
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
            host_metrics: None,
            scenes: None,
            changed_last_refresh: true,
            revision: None,
            moved_revision: None,
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
            // Through the `since` form even though there is nothing to
            // diff against: the plain one returns entities alone, and
            // the open scene set would then be unknown until the first
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

    /// Re-pulls the entity snapshot from the server. No-op unless
    /// connected. The previous snapshot survives a failure, so a transient
    /// hiccup does not blank the editor — but the session is marked stale
    /// until one succeeds, because a mirror that stopped tracking looks
    /// exactly like a world where nothing happens to be moving.
    ///
    /// The complaint is `warn!` and it is said once. It used to be
    /// `debug!`, invisible under `RUST_LOG=info`, so a snapshot that froze
    /// for good did so in silence.
    /// Whether the last [`Self::refresh`] brought anything new.
    ///
    /// `false` means the mirror already matches the world and applying
    /// the snapshot would walk every entity to discover that.
    pub fn changed_last_refresh(&self) -> bool {
        self.changed_last_refresh
    }

    /// What the project's process last reported its own frame cost to be.
    ///
    /// `None` in local mode, before the first pull, and from a host too
    /// old to send it.
    pub fn host_metrics(&self) -> Option<kooch_remote::protocol::HostMetrics> {
        self.host_metrics
    }

    /// The scenes the project has open, or `None` if it has not said.
    ///
    /// The two are not the same answer: `None` is a project that has
    /// not replied yet or a host too old to send the field, and a caller
    /// should fall back to whatever it knows locally. `Some` is the open
    /// set, empty included.
    pub fn open_scenes(&self) -> Option<&[kooch_remote::protocol::SceneEntry]> {
        self.scenes.as_deref()
    }

    /// The play-mode pull: what moved, and nothing else (#1012).
    ///
    /// Returns the transforms to write, or `None` when the host refused
    /// the question — the entity set changed and the caller has to
    /// [`Self::refresh`] instead on this frame.
    ///
    /// 🔴 The revision is SHARED with `refresh`, deliberately. Two
    /// counters would drift the moment the editor alternated between the
    /// two pulls, and a diff computed against the wrong one describes a
    /// world nobody holds. The host keeps a cache per method and both
    /// answer `full` when the revision handed to them is not theirs, so
    /// the first pull after a switch is a full one and correct.
    pub fn refresh_moved(&mut self) -> Option<Vec<kooch_remote::protocol::MovedTransform>> {
        if self.state != ConnectionState::Connected {
            return Some(Vec::new());
        }
        match self.client.list_moved_since(self.moved_revision) {
            Ok(update) => {
                if update.host.is_some() {
                    self.host_metrics = update.host;
                }
                // Kept whatever the answer: the host has described a
                // world to this counter and the next cheap pull has to
                // diff against that one. Dropping it here is what made
                // every reply `full`.
                self.moved_revision = Some(update.revision);
                if update.full {
                    // The host declined — its entity set changed. The
                    // SNAPSHOT revision is the one that must not be
                    // trusted now, since the full pull that follows has
                    // to be a full one.
                    self.revision = None;
                    return None;
                }
                self.changed_last_refresh = !update.moved.is_empty() || !update.removed.is_empty();
                if !update.removed.is_empty() {
                    // A despawn is structure. Let the full path handle
                    // it rather than teaching the cheap one to unmap ids.
                    self.revision = None;
                    return None;
                }
                Some(update.moved)
            }
            Err(e) => {
                self.revision = None;
                self.moved_revision = None;
                self.changed_last_refresh = false;
                let reason = e.to_string();
                if self.stale.replace(reason.clone()).is_none() {
                    tracing::warn!(
                        "the remote snapshot stopped updating: {reason}. \
                         The editor is showing the last world it could read",
                    );
                }
                Some(Vec::new())
            }
        }
    }

    pub fn refresh(&mut self) {
        if self.state != ConnectionState::Connected {
            return;
        }
        match self.client.list_entities_since(self.revision) {
            Ok(update) => {
                // `full` is the project's decision, not ours: it sends
                // everything whenever it cannot honour the revision we
                // hold. Merging a full reply would keep entities it had
                // deliberately left out.
                // A full reply always counts as a change: it arrives
                // precisely when the project could not honour our
                // revision, so what we hold cannot be trusted to match.
                self.changed_last_refresh =
                    update.full || !update.entities.is_empty() || !update.removed.is_empty();
                if update.full {
                    self.snapshot = update.entities;
                } else {
                    merge_into(&mut self.snapshot, update.entities, &update.removed);
                }
                self.revision = Some(update.revision);
                // Kept rather than overwritten with `None`: a pull that
                // reaches an older host, or one that has not finished its
                // first frame, should leave the last known numbers on
                // screen instead of blanking them every other frame.
                if update.host.is_some() {
                    self.host_metrics = update.host;
                }
                // Same reasoning as the metrics above, and it matters
                // more: blanking the open set on a reply from an older
                // host would empty the World panel of every scene while
                // the entities that belong to them keep arriving.
                if update.scenes.is_some() {
                    self.scenes = update.scenes;
                }
                if self.stale.take().is_some() {
                    tracing::info!("remote snapshot is tracking the project again");
                }
            }
            Err(e) => {
                // Drop the revision: the next pull has to be a full one.
                // The snapshot we keep showing is the last good world,
                // and a diff computed against a revision we may have
                // diverged from would layer new errors on top of it.
                self.revision = None;
                // The snapshot is unchanged because the pull failed, not
                // because the world stood still. Saying "nothing changed"
                // would be true and misleading — but it is also harmless
                // here, since the mirror already matches what we hold.
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

/// Folds a diff into a world.
///
/// A free function rather than a method: it is the part of the delta
/// path that can be wrong in ways nobody notices — a dropped removal
/// leaves a deleted entity on screen, editable, with every edit going
/// nowhere — and testing it should not require standing up a project to
/// talk to.
///
/// Order matters. Removals go first: an index despawned and reused
/// inside one revision arrives as both a removal and a change, and
/// removing afterwards would delete what had just been added.
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
