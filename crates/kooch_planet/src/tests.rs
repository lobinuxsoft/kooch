use glam::{DVec3, IVec3};

use super::{
    BLOCK_BYTES, BlockKey, BlockPool, DEFAULT_CAPACITY, Limits, Planets, Residency, Shape, View,
    select,
};

/// Radius and distance from Earth at closest approach, in km.
const SOLAR_SYSTEM: [(&str, f64, f64); 9] = [
    ("Earth", 6371.0, 0.0),
    ("Moon", 1737.4, 384_400.0),
    ("Venus", 6051.8, 41.4e6),
    ("Mars", 3389.5, 78.3e6),
    ("Mercury", 2439.7, 91.7e6),
    ("Jupiter", 69_911.0, 628.7e6),
    ("Saturn", 58_232.0, 1.28e9),
    ("Uranus", 25_362.0, 2.72e9),
    ("Neptune", 24_622.0, 4.35e9),
];

/// Every planet of the solar system, the viewer `height` metres above Earth towards the Moon.
fn solar_system(height: f64) -> Planets {
    let mut planets = Planets::default();
    let earth = SOLAR_SYSTEM[0].1 * 1000.0;
    for (_, radius, distance) in SOLAR_SYSTEM {
        let radius = radius * 1000.0;
        let shape = Shape {
            radius,
            relief: radius * 0.0015,
            depth: 2000.0,
            voxel: 0.5,
        };
        let viewer = DVec3::new(0.0, earth + height - distance * 1000.0, 0.0);
        let limits = Limits {
            floor: 64,
            ceiling: DEFAULT_CAPACITY,
        };
        planets.push(shape, &[viewer], limits);
    }
    planets
}

/// Wavelength of the proxy hills, in metres.
const HILLS: f64 = 20_000.0;

/// Stand-in for the generator's range analysis: hills of height `relief` on the sphere, bounded
/// by their Lipschitz constant, so a block farther than that from the surface is proved uniform.
fn hills(planets: &Planets) -> impl Fn(BlockKey, DVec3, DVec3) -> bool + '_ {
    |key, min, max| {
        let shape = planets.shape[key.planet as usize];
        let centre = (min + max) * 0.5;
        let wave = centre / HILLS;
        let height = shape.relief * wave.x.sin() * wave.y.sin() * wave.z.sin();
        let sdf = centre.length() - shape.radius - height;
        let lipschitz = 1.0 + shape.relief * 3f64.sqrt() / HILLS;
        sdf.abs() <= lipschitz * (max - min).length() * 0.5
    }
}

/// #313 step 2 measurement: the whole solar system fits the default pool from the ground, low
/// orbit and halfway to the Moon. Run with `--nocapture` for the table.
#[test]
fn the_solar_system_fits() {
    let view = View::new(800.0, 60f64.to_radians());
    let mut wanted = Vec::new();
    for height in [1.8, 400_000.0, 192_000_000.0] {
        let planets = solar_system(height);
        let worst = select(
            &planets,
            &view,
            DEFAULT_CAPACITY,
            hills(&planets),
            |_| false,
            &mut wanted,
        );
        let mut per_planet = [0u32; SOLAR_SYSTEM.len()];
        for key in &wanted {
            per_planet[key.planet as usize] += 1;
        }
        let mib = wanted.len() as f64 * BLOCK_BYTES as f64 / (1024.0 * 1024.0);
        eprintln!(
            "height {height} m: {} blocks, {mib:.1} MiB, worst {worst:.1} px",
            wanted.len()
        );
        for ((name, ..), count) in SOLAR_SYSTEM.iter().zip(per_planet) {
            eprintln!("  {name}: {count}");
        }
        assert!(wanted.len() as u32 <= DEFAULT_CAPACITY);
        assert!(per_planet[0] > 0, "Earth is in view from {height} m");
    }
}

/// Touching leaves are at most one level apart, so a seam meets one level change (#393).
#[test]
fn touching_leaves_are_balanced() {
    let view = View::new(800.0, 60f64.to_radians());
    let planets = solar_system(1.8);
    let mut wanted = Vec::new();
    select(
        &planets,
        &view,
        DEFAULT_CAPACITY,
        hills(&planets),
        |_| false,
        &mut wanted,
    );
    let earth: Vec<BlockKey> = wanted.into_iter().filter(|k| k.planet == 0).collect();
    // A block's span in finest-level cells, low inclusive, high exclusive.
    let span = |k: &BlockKey| {
        (
            k.cell << k.level as i32,
            (k.cell + IVec3::ONE) << k.level as i32,
        )
    };
    for a in &earth {
        let (alo, ahi) = span(a);
        for b in earth.iter().filter(|b| b.level > a.level + 1) {
            let (blo, bhi) = span(b);
            let touch = (0..3).all(|i| alo[i] <= bhi[i] && blo[i] <= ahi[i]);
            assert!(!touch, "{a:?} touches {b:?}");
        }
    }
}

fn key(cell: i32) -> BlockKey {
    BlockKey {
        planet: 0,
        level: 0,
        cell: IVec3::splat(cell),
    }
}

/// A full pool frees the blocks no longer wanted before it hands out new ones.
#[test]
fn a_full_pool_swaps() {
    let mut pool = BlockPool::new(2);
    let mut residency = Residency::default();
    pool.apply(&[key(0), key(1)], &mut residency);
    pool.apply(&[key(1), key(2)], &mut residency);
    assert_eq!(residency.removed.len(), 1);
    assert_eq!(residency.added, residency.removed);
    assert_eq!(pool.slot(key(0)), None);
    assert!(pool.slot(key(2)).is_some());
}
