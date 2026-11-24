//! Extra coat, shoulder and boot volumes attached to the normal avatar bones.
use glam::{EulerRot, Quat, Vec3};
use horizon_assets::resource_source::ResourceSource;
use horizon_client::mobs::{Part, Polygon};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Detail {
    /// Min then max XYZ relative to the armor cube, whose bounds are 0..1.
    /// Use either `bounds` or `corners` for a piece.
    bounds: Option<[f32; 6]>,
    /// Eight XYZ points in armor-cube units. The logical corner index is
    /// high-X + 2 * high-Y + 4 * high-Z, with high-Y at the bottom.
    /// Coincident corners may close an edge of a wedge.
    corners: Option<[[f32; 3]; 8]>,
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
            let bytes = root.read_limited(&path, 128 * 1024).ok()?;
            let specs =
                serde_json::from_slice::<HashMap<String, HashMap<String, Vec<Detail>>>>(&bytes)
                    .ok()?;
            let shapes = specs
                .into_iter()
                .filter_map(|(slot, details)| {
                    let mut part = bases.get(slot.as_str())?.clone();
                    add_details(&mut part, &details);
                    Some((slot, part))
                })
                .collect();
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
        for spec in specs.iter().take(24) {
            if let Some(piece) = volume(cube, spec) {
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

fn volume(cube: &[Polygon], detail: &Detail) -> Option<Vec<Polygon>> {
    let taper = detail.taper.unwrap_or([1.0; 2]);
    if detail.uv.is_some_and(|[u0, v0, u1, v1]| {
        !(0.0 <= u0 && u0 + 1.0 <= u1 && u1 <= 64.0 && 0.0 <= v0 && v0 + 1.0 <= v1 && v1 <= 32.0)
    }) || taper.iter().any(|value| !(0.30..=1.80).contains(value))
        || detail
            .rotation
            .iter()
            .any(|angle| !(-45.0..=45.0).contains(angle))
    {
        return None;
    }
    let unit_corners = match (detail.bounds, detail.corners) {
        (Some(bounds), None)
            if bounds.iter().all(|x| (-0.75..=1.75).contains(x))
                && (0..3).all(|axis| bounds[axis] < bounds[axis + 3]) =>
        {
            std::array::from_fn(|corner| {
                std::array::from_fn(|axis| bounds[axis + 3 * ((corner >> axis) & 1)])
            })
        }
        (None, Some(corners)) if corners.iter().flatten().all(|x| (-2.0..=3.0).contains(x)) => {
            corners
        }
        _ => return None,
    };
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for vertex in cube.iter().flat_map(|poly| &poly.vertices) {
        lo = lo.min(position(vertex));
        hi = hi.max(position(vertex));
    }
    let span = hi - lo;
    if span.min_element() <= 0.0 {
        return None;
    }
    let mut corners = unit_corners.map(|point| lo + span * Vec3::from_array(point));
    let pivot = (corners[0] + corners[1] + corners[4] + corners[5]) * 0.25;
    if taper != [1.0; 2] {
        for index in [2, 3, 6, 7] {
            corners[index] = pivot + (corners[index] - pivot) * Vec3::new(taper[0], 1.0, taper[1]);
        }
    }
    let [x, y, z] = detail.rotation.map(f32::to_radians);
    let rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
    if detail.rotation != [0.0; 3] {
        corners = corners.map(|point| pivot + rotation * (point - pivot));
    }
    let center = corners.iter().copied().sum::<Vec3>() * 0.125;
    let shaped = taper != [1.0; 2] || detail.corners.is_some();
    let mut faces = Vec::with_capacity(cube.len());
    for mut face in cube.iter().cloned() {
        let reference = face.vertices.each_ref().map(position);
        let swatch = detail.uv.map(|uv| {
            let min_u = face
                .vertices
                .iter()
                .map(|v| v[3])
                .fold(f32::INFINITY, f32::min);
            let min_v = face
                .vertices
                .iter()
                .map(|v| v[4])
                .fold(f32::INFINITY, f32::min);
            (uv, min_u, min_v)
        });
        for vertex in &mut face.vertices {
            let corner = usize::from(vertex[0] != lo.x)
                | (usize::from(vertex[1] != lo.y) << 1)
                | (usize::from(vertex[2] != lo.z) << 2);
            vertex[..3].copy_from_slice(&corners[corner].to_array());
            if let Some((uv, min_u, min_v)) = swatch {
                vertex[3] = if vertex[3] == min_u {
                    uv[0] + 0.25
                } else {
                    uv[2] - 0.25
                } / 64.0;
                vertex[4] = if vertex[4] == min_v {
                    uv[1] + 0.25
                } else {
                    uv[3] - 0.25
                } / 32.0;
            }
        }
        let original = Vec3::from_array(face.normal);
        face.normal = (rotation * original).to_array();
        if shaped {
            let [a, b, c, _] = reference;
            let winding = (b - a).cross(c - a).dot(original).signum();
            let [a, b, c, d] = face.vertices.each_ref().map(position);
            let first = ((b - a).cross(c - a) * winding).try_normalize();
            let second = ((c - a).cross(d - a) * winding).try_normalize();
            if [first, second]
                .into_iter()
                .flatten()
                .any(|normal| normal.dot(a - center) <= 0.0)
            {
                return None;
            }
            if let (Some(first), Some(second)) = (first, second)
                && first.dot(second) >= 0.99999
            {
                face.normal = first.to_array();
            } else {
                // The renderer indexes each quad as [0,1,2] and [0,2,3].
                // Repeat the final vertex to give each sloped triangle its own normal.
                let [a, b, c, d] = face.vertices;
                for (vertices, normal) in [([a, b, c, c], first), ([a, c, d, d], second)] {
                    if let Some(normal) = normal {
                        faces.push(Polygon {
                            vertices,
                            normal: normal.to_array(),
                            cull_back: face.cull_back,
                        });
                    }
                }
                continue;
            }
        }
        faces.push(face);
    }
    (!faces.is_empty()).then_some(faces)
}

fn position([x, y, z, ..]: &[f32; 5]) -> Vec3 {
    Vec3::new(*x, *y, *z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_cube() -> Vec<Polygon> {
        [
            ([0, 2, 3, 1], [0.0, 0.0, -1.0]),
            ([4, 5, 7, 6], [0.0, 0.0, 1.0]),
            ([0, 4, 6, 2], [-1.0, 0.0, 0.0]),
            ([1, 3, 7, 5], [1.0, 0.0, 0.0]),
            ([0, 1, 5, 4], [0.0, -1.0, 0.0]),
            ([2, 6, 7, 3], [0.0, 1.0, 0.0]),
        ]
        .map(|(corners, normal)| Polygon {
            normal,
            vertices: std::array::from_fn(|i| {
                let index = corners[i];
                [
                    (index & 1) as f32,
                    ((index >> 1) & 1) as f32,
                    ((index >> 2) & 1) as f32,
                    i as f32 * 0.125,
                    i as f32 * 0.25,
                ]
            }),
            cull_back: true,
        })
        .into()
    }

    #[test]
    fn asymmetric_volume_shades_each_bent_facet_and_keeps_its_uvs() {
        let cube = reference_cube();
        let spec: Detail = serde_json::from_str(
            r#"{"corners":[[0,0,0],[1,0,0],[0,1,0],[1,1,0.25],
                [0,0,1],[1,0,1],[0,1,1],[1,1,1]],"rotation":[0,15,0]}"#,
        )
        .unwrap();
        let faces = volume(&cube, &spec).unwrap();
        let (first, second) = (&faces[0], &faces[1]);
        let [a, b, c, d] = cube[0].vertices;
        for (face, old) in [(first, [a, b, c, c]), (second, [a, c, d, d])] {
            assert_eq!(face.vertices[2], face.vertices[3]);
            assert!(face.cull_back);
            for (vertex, previous) in face.vertices.iter().zip(old) {
                assert_eq!(vertex[3..], previous[3..]);
            }
            let [a, b, c, _] = face.vertices.each_ref().map(position);
            let normal = (b - a).cross(c - a).normalize();
            assert!(normal.abs_diff_eq(Vec3::from_array(face.normal), 1.0e-6));
        }
        assert_ne!(first.normal, second.normal);
    }

    #[test]
    fn legacy_bounds_render_more_than_twelve_details_and_keep_taper() {
        let mut part = Part {
            name: "body".into(),
            pose: [0.0; 9],
            polygons: reference_cube(),
            children: Vec::new(),
        };
        let spec = r#"{"bounds":[0,0,0,1,1,1],"taper":[0.5,1],"replace":true}"#;
        let details = (0..15)
            .map(|_| serde_json::from_str(spec).unwrap())
            .collect();
        add_details(&mut part, &HashMap::from([("body".into(), details)]));
        assert_eq!(part.polygons.len(), 15 * 6);
        assert_eq!(
            position(&part.polygons[0].vertices[1]),
            Vec3::new(0.25, 1.0, 0.0)
        );
    }

    #[test]
    fn closed_wedge_has_no_zero_area_faces() {
        let spec: Detail = serde_json::from_str(
            r#"{"corners":[[0,0,0],[1,0,0],[0,1,0],[1,1,0],
                [0,0,0],[1,0,0],[0,1,1],[1,1,1]]}"#,
        )
        .unwrap();
        let faces = volume(&reference_cube(), &spec).unwrap();
        assert_eq!(faces.len(), 5);
        for face in faces {
            let [a, b, c, _] = face.vertices.each_ref().map(position);
            assert!((b - a).cross(c - a).length_squared() > 0.0);
        }
    }
}
