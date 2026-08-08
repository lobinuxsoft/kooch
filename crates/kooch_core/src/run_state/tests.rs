use super::*;

fn counting_system(resources: &mut Resources) {
    let count = resources.get_mut::<u32>().expect("counter");
    *count += 1;
}

#[test]
fn gated_system_is_skipped_while_not_playing() {
    let mut resources = Resources::new();
    resources.insert(0u32);
    let mut gated = run_if_playing(counting_system);

    // No Playing resource at all — a world being edited.
    gated(&mut resources);
    assert_eq!(*resources.get::<u32>().unwrap(), 0);

    Playing::set(&mut resources, false);
    gated(&mut resources);
    assert_eq!(*resources.get::<u32>().unwrap(), 0);
}

#[test]
fn gated_system_runs_and_stops_live() {
    let mut resources = Resources::new();
    resources.insert(0u32);
    let mut gated = run_if_playing(counting_system);

    Playing::set(&mut resources, true);
    gated(&mut resources);
    gated(&mut resources);
    assert_eq!(*resources.get::<u32>().unwrap(), 2);

    // Flipping it off stops the same registered system.
    Playing::set(&mut resources, false);
    gated(&mut resources);
    assert_eq!(*resources.get::<u32>().unwrap(), 2);
}
