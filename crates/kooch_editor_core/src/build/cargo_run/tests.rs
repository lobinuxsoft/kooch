use std::process::Command;
use std::time::{Duration, Instant};

use super::CargoRun;

/// What cargo leaves behind when only its pid is killed: a child that outlives the parent. `sh`
/// stands in for cargo and `sleep` for rustc.
#[test]
fn a_stop_reaches_grandchildren() {
    let marker = format!("kooch_1358_{}", std::process::id());
    let mut command = Command::new("sh");
    command.args(["-c", &format!("exec -a {marker} sleep 30 & wait")]);
    let mut run = CargoRun::spawn(command).expect("sh spawns");

    let alive = || {
        Command::new("pgrep")
            .args(["-f", &marker])
            .status()
            .is_ok_and(|s| s.success())
    };
    let started = Instant::now();
    while !alive() && started.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(alive(), "the grandchild never started");

    run.stop();
    let stopped = Instant::now();
    while !run.gone() && stopped.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(run.gone(), "the tree was still alive after the stop");
    assert!(!alive(), "the grandchild survived the stop");
}
