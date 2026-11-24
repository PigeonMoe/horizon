//! Extra coat, shoulder and boot volumes attached to the normal avatar bones.
use horizon_assets::resource_source::ResourceSource;
use horizon_client::mobs::{Part, Polygon};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Detail {
    /// Min then max XYZ relative to the armor cube, whose bounds are 0..1.
    bounds: [f32; 6],
    /// Min then max UV in the outfit's 64x32 equipment texture.
    uv: [f32; 4],
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
        let faces = specs.iter().take(6).filter_map(|spec| cuboid(cube, spec))
            .flatten().collect::<Vec<_>>();
        part.polygons.extend(faces);
    }
    for child in &mut part.children {
        add_details(child, details);
    }
}

fn cuboid(cube: &[Polygon], detail: &Detail) -> Option<Vec<Polygon>> {
    let bounds = detail.bounds;
    let uv = detail.uv;
    if (0..3).any(|axis| !(-0.75..=1.75).contains(&bounds[axis])
        || !(-0.75..=1.75).contains(&bounds[axis + 3]) || bounds[axis] >= bounds[axis + 3])
        || !(0.0 <= uv[0] && uv[0] + 1.0 <= uv[2] && uv[2] <= 64.0
          && 0.0 <= uv[1] && uv[1] + 1.0 <= uv[3] && uv[3] <= 32.0)
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
    Some(cube.iter().cloned().map(|mut face| {
        let min_u = face.vertices.iter().map(|v| v[3]).fold(f32::INFINITY, f32::min);
        let min_v = face.vertices.iter().map(|v| v[4]).fold(f32::INFINITY, f32::min);
        for v in &mut face.vertices {
            for axis in 0..3 {
                let bound = if v[axis] == lo[axis] { bounds[axis] } else { bounds[axis + 3] };
                v[axis] = lo[axis] + (hi[axis] - lo[axis]) * bound;
            }
            v[3] = if v[3] == min_u { uv[0] + 0.25 } else { uv[2] - 0.25 } / 64.0;
            v[4] = if v[4] == min_v { uv[1] + 0.25 } else { uv[3] - 0.25 } / 32.0;
        }
        face
    }).collect())
}
