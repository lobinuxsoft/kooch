use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::{ComponentNames, ComponentRegistry};
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::name::Name;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::transform::Transform;
use kooch_remote::handlers::handle;
use kooch_remote::protocol::Method;
use kooch_remote::server::RemoteServer;

use super::*;

fn socket_name() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "kooch_pump_{}_{}_{}.sock",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn seeded_ecs() -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources.insert(DynamicComponents::new());
    resources.insert(ComponentNames::new());
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Name>();
        registry.register_cpu_reflected::<Transform>();
    }
    handle(
        &kooch_remote::protocol::Request {
            id: 0,
            notify: false,
            method: Method::Spawn {
                name: Some("Hero".into()),
                scene: None,
                parent: None,
            },
        },
        &mut resources,
    );
    resources
}

/// A project answering on its own thread, stopped when the guard drops.
struct FakeHost {
    socket: String,
    done: Arc<AtomicBool>,
}

impl FakeHost {
    fn start() -> Self {
        let server = RemoteServer::start(&socket_name()).expect("bind a socket");
        let socket = server.name().to_owned();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut resources = seeded_ecs();
            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let response = handle(&item.request, &mut resources);
                    let _ = item.reply.send(response);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Self { socket, done }
    }

    fn client(&self) -> Arc<RemoteClient> {
        Arc::new(RemoteClient::new(&self.socket))
    }
}

impl Drop for FakeHost {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
    }
}

/// Drains until something arrives or the deadline passes.
fn wait_for_reply(pump: &MovedPump) -> Vec<Pulled> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    while out.is_empty() && Instant::now() < deadline {
        pump.drain(&mut out);
        std::thread::sleep(Duration::from_millis(2));
    }
    out
}

#[test]
fn a_parked_pump_asks_nothing() {
    let host = FakeHost::start();
    let pump = MovedPump::spawn(host.client());

    std::thread::sleep(Duration::from_millis(50));
    let mut out = Vec::new();
    pump.drain(&mut out);
    assert!(out.is_empty(), "a pump nobody started should not pull");
}

#[test]
fn the_reply_arrives_off_thread() {
    let host = FakeHost::start();
    let pump = MovedPump::spawn(host.client());
    pump.set_running(true);

    let out = wait_for_reply(&pump);
    assert!(!out.is_empty(), "the pump never delivered a reply");
    assert!(matches!(out[0], Pulled::Update(_)));
}

#[test]
fn draining_never_blocks_the_caller() {
    let host = FakeHost::start();
    let pump = MovedPump::spawn(host.client());
    pump.set_running(true);

    // The point of the whole file: the editor's call costs nothing even
    // while the worker sits inside a round trip.
    let started = Instant::now();
    for _ in 0..100 {
        let mut out = Vec::new();
        pump.drain(&mut out);
    }
    assert!(
        started.elapsed() < Duration::from_millis(50),
        "100 drains took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_silent_project_reports_failure() {
    let pump = MovedPump::spawn(Arc::new(RemoteClient::new(
        "kooch_pump_nobody_listens.sock",
    )));
    pump.set_running(true);

    let out = wait_for_reply(&pump);
    assert!(!out.is_empty(), "a dead socket should still report");
    assert!(matches!(out[0], Pulled::Failed(_)));
}

#[test]
fn stopping_the_pump_stops_the_pulls() {
    let host = FakeHost::start();
    let pump = MovedPump::spawn(host.client());
    pump.set_running(true);
    assert!(!wait_for_reply(&pump).is_empty());

    pump.set_running(false);
    // One in flight may still land; after that the inbox stays empty.
    std::thread::sleep(Duration::from_millis(100));
    let mut drop_in_flight = Vec::new();
    pump.drain(&mut drop_in_flight);

    std::thread::sleep(Duration::from_millis(100));
    let mut out = Vec::new();
    pump.drain(&mut out);
    assert!(out.is_empty(), "a stopped pump kept pulling");
}
