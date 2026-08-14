//! Connects to a running game's puffin server, records, and writes a
//! `.puffin` — the editor's "Save capture" button without the editor.
//!
//! The panel is the only other client, and it needs a person watching
//! for the right moment to press Save. That is fine for one capture and
//! bad for the thing captures are actually for: an A/B needs *two*
//! runs of the same route, and a click at the wrong moment silently
//! makes them incomparable.
//!
//! Read the result with the `read_capture` example. This writes the
//! file and does not analyse it — a scratchpad script that summed
//! scopes without descending the tree once produced an issue built on a
//! false premise, and the fix was to leave the reading to the tool that
//! models parents.
//!
//! Run with:
//!   cargo run -p kooch_editor_core --features profiling \
//!     --example capture_remote -- 192.168.0.36:8585 out.puffin --seconds 30

use std::time::{Duration, Instant};

struct Args {
    addr: String,
    output: std::path::PathBuf,
    seconds: u64,
    min_frames: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut raw = std::env::args().skip(1);
    let addr = raw.next().ok_or("expected <addr> <output.puffin>")?;
    let output = raw.next().ok_or("expected <output.puffin>")?.into();
    let mut seconds = 30;
    let mut min_frames = 0;
    while let Some(flag) = raw.next() {
        // 🔴 The value comes off the same iterator. Peeking at
        // `env::args()` instead leaves it here, and the next turn of the
        // loop reads the number as the next flag — which is exactly what
        // this did on its first run.
        let mut number = || -> Result<u64, String> {
            raw.next()
                .ok_or_else(|| format!("{flag} needs a number"))?
                .parse()
                .map_err(|_| format!("{flag} needs a number"))
        };
        match flag.as_str() {
            "--seconds" => seconds = number()?,
            "--min-frames" => min_frames = number()? as usize,
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Args {
        addr,
        output,
        seconds,
        min_frames,
    })
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: capture_remote <addr> <output.puffin> \
                 [--seconds N] [--min-frames N]"
            );
            std::process::exit(2);
        }
    };

    // 🔴 The client must outlive the loop. Its `Drop` clears the flag
    // its receiving thread runs on, so a `Client` that is not held is a
    // connection that closes immediately and a capture of nothing.
    let client = puffin_http::Client::new(args.addr.clone());
    println!("connecting to {} …", args.addr);

    let deadline = Instant::now() + Duration::from_secs(args.seconds);
    let mut announced = false;
    let mut frames = 0;
    while Instant::now() < deadline || frames < args.min_frames {
        std::thread::sleep(Duration::from_millis(500));
        if !client.connected() {
            if announced {
                println!("connection dropped; waiting for it to come back");
                announced = false;
            }
            continue;
        }
        if !announced {
            println!("connected");
            announced = true;
        }
        frames = client.frame_view().all_uniq().count();
        print!("\r{frames} frames");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    println!();

    let view = client.frame_view();
    if frames == 0 {
        eprintln!(
            "no frames arrived. The game must be built with the profiling feature \
             (the preset's `profiling: true`), and it logs the address it listens on \
             at startup."
        );
        std::process::exit(1);
    }

    let file = match std::fs::File::create(&args.output) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("could not create {}: {error}", args.output.display());
            std::process::exit(1);
        }
    };
    let mut file = std::io::BufWriter::new(file);
    if let Err(error) = view.write(&mut file) {
        eprintln!("could not write the capture: {error}");
        std::process::exit(1);
    }
    println!("wrote {} frames to {}", frames, args.output.display());
    println!(
        "read it with: cargo run -p kooch_editor_core --features profiling \
         --example read_capture -- {} --split",
        args.output.display()
    );
}
