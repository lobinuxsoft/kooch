//! Packs a directory and reports ratio and read speed.
//!
//! ```sh
//! cargo run --release -p kooch_pack --example measure -- assets /tmp/out.kpack
//! ```
//!
//! An example rather than a test: the numbers depend on what it is
//! pointed at, and a test that asserted a ratio would be asserting the
//! contents of somebody's `assets/`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use kooch_pack::{Pack, PackKey, PackWriter};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (root, dest) = match (args.next(), args.next()) {
        (Some(root), Some(dest)) => (PathBuf::from(root), PathBuf::from(dest)),
        _ => {
            eprintln!("usage: measure <dir> <out.kpack>");
            std::process::exit(2);
        }
    };

    let mut list = Vec::new();
    files(&root, &mut list);
    let key = PackKey::generate();
    let raw: u64 = list
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();

    let started = Instant::now();
    let mut writer = PackWriter::new(std::fs::File::create(&dest).unwrap(), &key).unwrap();
    for path in &list {
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        writer.add_file(&name, path).unwrap();
    }
    writer.finish().unwrap();
    let packing = started.elapsed();
    let packed = std::fs::metadata(&dest).unwrap().len();

    let started = Instant::now();
    let mut pack = Pack::open(&dest, &key).unwrap();
    let count = pack.verify().unwrap();
    let reading = started.elapsed();

    println!("files      {count}");
    println!("raw        {:.2} MiB", raw as f64 / 1048576.0);
    println!(
        "packed     {:.2} MiB  ({:.1}% of raw)",
        packed as f64 / 1048576.0,
        packed as f64 * 100.0 / raw as f64,
    );
    println!("pack       {:.2} s", packing.as_secs_f64());
    println!(
        "read all   {:.0} ms  ({:.0} MiB/s)",
        reading.as_secs_f64() * 1000.0,
        raw as f64 / 1048576.0 / reading.as_secs_f64(),
    );
}
