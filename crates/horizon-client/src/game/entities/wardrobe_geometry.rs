//! Extra coat, shoulder and boot volumes attached to the normal avatar bones.
use horizon_assets::resource_source::ResourceSource;
use horizon_client::mobs::{Part, Polygon};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Detail {
    /// Min then max XYZ relative to the armor cube, whose bounds are 0..1.
    bounds: [f32; 6],
    /// A fabric swatch in the 64x32 atlas. An omitted swatch uses the model's full face UVs.
    uv: Option<[f32; 4]>,
    /// Horizontal and depth scale at the bottom of the piece.
    taper: Option<[f32; 2]>,
    /// This bone uses its authored volumes in place of the vanilla armor shell.
    #[serde(default)]
    replace: bool,
    /// XYZ Euler angles in degrees about the panel's top-center attachment.
    #[serde(default)]
    rotation: [f32; 3],
}

pub(super) fn load(
    root: &ResourceSource,
    bases: &HashMap<&str, Part>,
) -> HashMap<String, HashMap<String, Part>> {
    const PREFIX: &str = "assets/jianghu/wardrobe_geometry/";
    root.list(PREFIX)
        .into_iter()
        .filter_map(|path| {
            let name = path.strip_prefix(PREFIX)?.strip_suffix(".json")?;
            let bytes = root.read_limited(&path, 32768).ok()?;
            let specs = serde_json::from_slice::<HashMap<String, HashMap<String, Vec<Detail>>>>(&bytes).ok()?;
            let shapes = specs.into_iter().filter_map(|(slot, details)| {
                let mut part = bases.get(slot.as_str())?.clone();
                add_details(&mut part, &details);
                Some((slot, part))
            }).collect();
            Some((format!("jianghu:{name}"), shapes))
        })
        .collect()
}

fn add_details(part: &mut Part, details: &HashMap<String, Vec<Detail>>) {
    if let Some(specs) = details.get(&part.name)
        && let Some(cube) = part.polygons.get(..6)
    {
        let mut replace = false;
        let mut faces = Vec::new();
        for spec in specs.iter().take(6) {
            if let Some(piece) = cuboid(cube, spec) {
                replace |= spec.replace;
                faces.extend(piece);
            }
        }
        if replace {
            part.polygons = faces;
        } else {
            part.polygons.extend(faces);
        }
    }
    for child in &mut part.children {
        add_details(child, details);
    }
}

fn cuboid(cube: &[Polygon], detail: &Detail) -> Option<Vec<Polygon>> {
    let bounds = detail.bounds;
    let taper = detail.taper.unwrap_or([1.0; 2]);
    if (0..3).any(|axis| !(-0.75..=1.75).contains(&bounds[axis])
        || !(-0.75..=1.75).contains(&bounds[axis + 3]) || bounds[axis] >= bounds[axis + 3])
        || detail.uv.is_some_and(|[u0, v0, u1, v1]| {
            !(0.0 <= u0 && u0 + 1.0 <= u1 && u1 <= 64.0
                && 0.0 <= v0 && v0 + 1.0 <= v1 && v1 <= 32.0)
        })
        || taper.iter().any(|value| !(0.30..=1.80).contains(value))
        || detail.rotation.iter().any(|angle| !(-45.0..=45.0).contains(angle))
    {
        return None;
    }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for vertex in cube.iter().flat_map(|poly| &poly.vertices) {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(vertex[axis]);
            hi[axis] = hi[axis].max(vertex[axis]);
        }
    }
    if (0..3).any(|axis| hi[axis] <= lo[axis]) {
        return None;
    }
    let unit_to_model = |axis: usize, value: f32| lo[axis] + (hi[axis] - lo[axis]) * value;
    let center_x = unit_to_model(0, (bounds[0] + bounds[3]) * 0.5);
    let center_z = unit_to_model(2, (bounds[2] + bounds[5]) * 0.5);
    let turn = (detail.rotation != [0.0; 3]).then(|| {
        let [x, y, z] = detail.rotation.map(f32::to_radians);
        let rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, x, y, z);
        let pivot = glam::Vec3::new(center_x, unit_to_model(1, bounds[1]), center_z);
        (rotation, pivot)
    });
    let point = |v: &[f32; 5]| glam::Vec3::new(v[0], v[1], v[2]);
    let tapered = taper != [1.0; 2];
    let mut faces = Vec::with_capacity(cube.len());
    for mut face in cube.iter().cloned() {
        let swatch = detail.uv.map(|uv| {
            let min_u = face.vertices.iter().map(|v| v[3]).fold(f32::INFINITY, f32::min);
            let min_v = face.vertices.iter().map(|v| v[4]).fold(f32::INFINITY, f32::min);
            (uv, min_u, min_v)
        });
        for vertex in &mut face.vertices {
            let bottom = vertex[1] == hi[1];
            for axis in 0..3 {
                let bound = if vertex[axis] == lo[axis] { bounds[axis] } else { bounds[axis + 3] };
                vertex[axis] = unit_to_model(axis, bound);
            }
            if bottom && tapered {
                vertex[0] = center_x + (vertex[0] - center_x) * taper[0];
                vertex[2] = center_z + (vertex[2] - center_z) * taper[1];
            }
            if let Some((uv, min_u, min_v)) = swatch {
                vertex[3] = if vertex[3] == min_u { uv[0] + 0.25 } else { uv[2] - 0.25 } / 64.0;
                vertex[4] = if vertex[4] == min_v { uv[1] + 0.25 } else { uv[3] - 0.25 } / 32.0;
            }
        }
        if tapered {
            let [a, b, c, _] = face.vertices.each_ref().map(point);
            let surface = (b - a).cross(c - a).normalize_or_zero();
            if surface != glam::Vec3::ZERO {
                let original = glam::Vec3::from_array(face.normal);
                face.normal = if surface.dot(original) < 0.0 { -surface } else { surface }.to_array();
            }
        }
        if let Some((rotation, pivot)) = turn {
            for vertex in &mut face.vertices {
                let rotated = pivot + rotation * (point(vertex) - pivot);
                vertex[..3].copy_from_slice(&rotated.to_array());
            }
            face.normal = (rotation * glam::Vec3::from_array(face.normal)).to_array();
        }
        faces.push(face);
    }
    Some(faces)
}
