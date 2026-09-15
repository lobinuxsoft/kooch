use super::*;

/// 🔴 Naga validates a shader alone; only wgpu checks it against the layout. Both shading paths,
/// with the default surface and a custom one, built on a real device.
#[test]
fn both_paths_build_a_custom_surface() {
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        return;
    };
    let features = kooch_core::gpu::vbuf64_features();
    if !adapter.features().contains(features) {
        return;
    }
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: features,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("device");
    let red = "fn surface(input: SurfaceInput) -> SurfaceOutput {
        var out: SurfaceOutput;
        out.base_color = vec3<f32>(1.0, 0.0, 0.0);
        out.normal = normalize(input.world_normal);
        out.roughness = 0.5;
        return out;
    }";
    let red = crate::material::Shader::parse(red).unwrap();
    crate::meshlet::validate_surface(&red.params_wgsl(), &red.source).unwrap();

    let meshlet_bgl = crate::meshlet::meshlet_bind_group_layout(&device);
    let compute = ComputeShading::new(&device, &meshlet_bgl);
    build_pipeline(
        &device,
        &compute.layout,
        &red.params_wgsl(),
        &red.source,
        false,
    );
    build_pipeline(
        &device,
        &compute.layout,
        &red.params_wgsl(),
        &red.source,
        true,
    );
    // 🔴 The template reads `material_values` and binds all four textures: the bindings a shader
    // with no parameters never touches, and naga never checks against the layout.
    let template = crate::material::Shader::parse(crate::meshlet::NEW_SURFACE_SHADER).unwrap();
    let wgsl = template.params_wgsl();
    build_pipeline(&device, &compute.layout, &wgsl, &template.source, false);
    let fourth = crate::meshlet::NEW_SURFACE_SHADER.replace(
        "var metal_roughness: texture_2d<f32>;",
        "var metal_roughness: texture_2d<f32>;\nvar extra: texture_2d<f32>;",
    );
    let fourth = crate::material::Shader::parse(&fourth).unwrap();
    let body = fourth.source.replace(
        "out.emissive = base * p.emissive;",
        "out.emissive = sample_surface(extra, input, uv, p.uv_scale).rgb * p.emissive;",
    );
    build_pipeline(
        &device,
        &compute.layout,
        &fourth.params_wgsl(),
        &body,
        false,
    );
    super::super::two_pass::MaterialTwoPass::new(&device, &meshlet_bgl);
}

/// 🔴 An unset variable has to read as `None`, not as `Some(false)`.
#[test]
fn an_unset_variable_says_nothing() {
    assert_eq!(parse_enabled(None), None);
    // …and so does a spelling the parser does not recognise. A typo
    // during a measurement run must not silently change which path is
    // being measured.
    assert_eq!(parse_enabled(Some("")), None);
    assert_eq!(parse_enabled(Some("yes")), None);
    assert_eq!(parse_enabled(Some("ON")), None);
}

#[test]
fn the_spellings_a_measurement_run_would_use_all_work() {
    for raw in ["on", "1", "true"] {
        assert_eq!(
            parse_enabled(Some(raw)),
            Some(true),
            "KOOCH_COMPUTE_SHADING={raw}",
        );
    }
    // Both directions, because the asset's default is now `true`: a
    // capture that needs the fragment path has to be able to ask for it.
    for raw in ["off", "0", "false"] {
        assert_eq!(
            parse_enabled(Some(raw)),
            Some(false),
            "KOOCH_COMPUTE_SHADING={raw}",
        );
    }
}

/// The shader declares the shading target at a fixed binding, and a
/// group-0 layout that disagreed would fail at pipeline creation with a
/// message about binding counts rather than about this.
#[test]
fn the_colour_target_sits_past_the_contact_shadow_bindings() {
    assert!(COLOR_OUT_BINDING > MATERIAL_PASS_CONTACT_DEPTH_BINDING);
    assert!(MATERIAL_COMPUTE_FRAME.contains(&format!("@group(0) @binding({COLOR_OUT_BINDING})")));
}
