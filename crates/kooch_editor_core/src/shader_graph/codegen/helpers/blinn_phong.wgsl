fn graph_blinn_phong(to_normal: vec3<f32>, to_light: vec3<f32>, to_eye: vec3<f32>, shininess: f32) -> vec2<f32> {
    let n = normalize(to_normal);
    let l = normalize(to_light);
    let half_way = normalize(l + normalize(to_eye));
    let specular = pow(max(dot(n, half_way), 0.0), max(shininess, 1.0));
    return vec2<f32>(max(dot(n, l), 0.0), specular);
}
