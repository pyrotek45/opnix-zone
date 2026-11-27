//! Turns block models into GPU meshes.
//!
//! Performance is the point of this module. A model with thirty parts is
//! *baked* into at most four meshes (lit body, unlit glow, and two leg
//! groups), all vertex-coloured so the entire game shares two materials.
//! Identical models share mesh handles, which lets the renderer batch a horde
//! of a thousand enemies into a handful of instanced draw calls.

use bevy::{
    asset::RenderAssetUsages,
    pbr::NotShadowCaster,
    platform::collections::HashMap,
    prelude::*,
    render::mesh::{MeshBuilder as _, PrimitiveTopology},
};
use oz_core::{
    model::{Anim, BlockModel, Hull, Paint, Part, Shape, convex_hull},
    worldgen::srgb_to_linear,
};

/// Unit primitive as flat triangle data.
struct Template {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
}

impl Template {
    fn new(mut mesh: Mesh, flat: bool) -> Self {
        mesh.duplicate_vertices();
        if flat {
            mesh.compute_flat_normals();
        }
        let read = |mesh: &Mesh, id| -> Vec<Vec3> {
            mesh.attribute(id).and_then(|a| a.as_float3()).map(|v| v.iter().map(|p| Vec3::from(*p)).collect()).unwrap_or_default()
        };
        Self { positions: read(&mesh, Mesh::ATTRIBUTE_POSITION), normals: read(&mesh, Mesh::ATTRIBUTE_NORMAL) }
    }
}

/// Accumulates painted primitives into one vertex-coloured mesh.
#[derive(Default)]
pub struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

impl MeshBuilder {
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    pub fn build(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, KEEP_FOR_BOUNDS)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
    }
}

/// Meshes keep their vertex data on the CPU as well as the GPU. Bevy can only
/// compute a mesh's bounding box (used for culling) while that data exists,
/// and shared meshes are spawned long after they are built: without it,
/// robots and props were culled while in plain view and flickered in and out.
pub const KEEP_FOR_BOUNDS: RenderAssetUsages = RenderAssetUsages::all();

/// A model's box grown a fraction of a millimetre, the thinner the more (never more than
/// a third of its thickness), so a plate or stripe laid flush on a casing is drawn just in
/// front of it instead of flickering against it. The model-sized cousin of
/// [`oz_core::worldgen::draw_margin`].
pub fn seated(part: &Part) -> Part {
    if part.shape != Shape::Cube {
        return *part;
    }
    let thin = part.size.min_element().max(1e-4);
    // Thickness sets the step; the paint picks a notch within it, so two parts of different
    // colour laid in one plane never fight over it either.
    let notch = Paint::ALL.iter().position(|p| *p == part.paint).unwrap_or(0) as f32 * 0.00004;
    let margin = ((0.000015 / thin).clamp(0.0001, 0.0015) + notch).min(thin * 0.3);
    Part { size: part.size + Vec3::splat(margin * 2.0), ..*part }
}

pub fn linear(paint: Paint) -> [f32; 4] {
    let [r, g, b] = paint.rgb().map(srgb_to_linear);
    [r, g, b, 1.0]
}

/// A model ready to spawn: shared mesh handles plus the few parts that animate on their own.
#[derive(Clone, Default)]
pub struct Baked {
    pub lit: Option<Handle<Mesh>>,
    pub unlit: Option<Handle<Mesh>>,
    /// Leg parts, split by gait phase so each group swings as one entity.
    pub legs: [Option<Handle<Mesh>>; 2],
    pub extras: Vec<Part>,
}

/// One half of an enemy's legs. Animated by `actors`.
#[derive(Component, Clone, Copy)]
pub struct LegGroup(pub usize);

/// A part that animates on its own (spinning drill bit, pulsing core).
#[derive(Component, Clone, Copy)]
pub struct AnimPart {
    pub anim: Anim,
    pub base: Transform,
}

#[derive(Resource)]
pub struct Art {
    templates: HashMap<Shape, Template>,
    primitives: HashMap<Shape, Handle<Mesh>>,
    /// White materials: vertex colours carry all the colour.
    pub lit: Handle<StandardMaterial>,
    pub unlit: Handle<StandardMaterial>,
    paints: HashMap<Paint, Handle<StandardMaterial>>,
}

impl Art {
    pub fn primitive(&self, shape: Shape) -> Handle<Mesh> {
        self.primitives[&shape].clone()
    }

    /// Solid-colour material for stand-alone primitives (particles, tracers, animated parts).
    pub fn paint(&self, paint: Paint) -> Handle<StandardMaterial> {
        self.paints[&paint].clone()
    }

    pub fn add_part(&self, builder: &mut MeshBuilder, part: &Part, offset: Transform) {
        let template = &self.templates[&part.shape];
        let local = offset * Transform { translation: part.pos, rotation: part.rot, scale: part.size };
        let matrix = local.compute_matrix();
        let rotation = local.rotation;
        let color = linear(part.paint);
        for (position, normal) in template.positions.iter().zip(&template.normals) {
            builder.positions.push(matrix.transform_point3(*position).to_array());
            // Inverse-transpose for non-uniform scale keeps facets lit correctly.
            builder.normals.push((rotation * (*normal / part.size.max(Vec3::splat(1e-4)))).normalize_or_zero().to_array());
            builder.colors.push(color);
        }
    }

    /// One flat triangle, facing the way it winds (counter-clockwise from the front).
    pub fn add_triangle(&self, builder: &mut MeshBuilder, [a, b, c]: [Vec3; 3], paint: Paint) {
        let normal = (b - a).cross(c - a).normalize_or_zero().to_array();
        let color = linear(paint);
        for p in [a, b, c] {
            builder.positions.push(p.to_array());
            builder.normals.push(normal);
            builder.colors.push(color);
        }
    }

    /// A fused solid: its convex hull, faceted.
    pub fn add_hull(&self, builder: &mut MeshBuilder, hull: &Hull, offset: Transform) {
        let matrix = offset.compute_matrix();
        let color = linear(hull.paint);
        for triangle in convex_hull(&hull.points) {
            let [a, b, c] = triangle.map(|p| matrix.transform_point3(p));
            let normal = (b - a).cross(c - a).normalize_or_zero().to_array();
            for p in [a, b, c] {
                builder.positions.push(p.to_array());
                builder.normals.push(normal);
                builder.colors.push(color);
            }
        }
    }

    /// Merges a whole model (any paints) into lit and unlit builders at `offset`.
    pub fn add_model(&self, lit: &mut MeshBuilder, unlit: &mut MeshBuilder, model: &BlockModel, offset: Transform) {
        for part in &model.parts {
            self.add_part(if part.paint.unlit() { &mut *unlit } else { &mut *lit }, &seated(part), offset);
        }
        for hull in &model.hulls {
            self.add_hull(if hull.paint.unlit() { &mut *unlit } else { &mut *lit }, hull, offset);
        }
    }

    pub fn bake(&self, model: &BlockModel, meshes: &mut Assets<Mesh>) -> Baked {
        let (mut lit, mut unlit) = (MeshBuilder::default(), MeshBuilder::default());
        let mut legs = [MeshBuilder::default(), MeshBuilder::default()];
        let mut extras = Vec::new();
        for part in &model.parts {
            match part.anim {
                Anim::None | Anim::Mag | Anim::Slide => self.add_part(if part.paint.unlit() { &mut unlit } else { &mut lit }, &seated(part), Transform::IDENTITY),
                Anim::Leg { phase } => self.add_part(&mut legs[(phase.fract() >= 0.5) as usize], &seated(part), Transform::IDENTITY),
                _ => extras.push(*part),
            }
        }
        for hull in &model.hulls {
            match hull.anim {
                Anim::Leg { phase } => self.add_hull(&mut legs[(phase.fract() >= 0.5) as usize], hull, Transform::IDENTITY),
                _ => self.add_hull(if hull.paint.unlit() { &mut unlit } else { &mut lit }, hull, Transform::IDENTITY),
            }
        }
        let mut finish = |builder: MeshBuilder| (!builder.is_empty()).then(|| meshes.add(builder.build()));
        let [a, b] = legs;
        Baked { lit: finish(lit), unlit: finish(unlit), legs: [finish(a), finish(b)], extras }
    }

    /// Attaches a baked model's meshes as children of `root`.
    pub fn spawn(&self, commands: &mut Commands, root: Entity, baked: &Baked) {
        commands.entity(root).insert(Visibility::Inherited).with_children(|parent| {
            if let Some(mesh) = &baked.lit {
                parent.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(self.lit.clone())));
            }
            if let Some(mesh) = &baked.unlit {
                parent.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(self.unlit.clone()), NotShadowCaster));
            }
            for (group, mesh) in baked.legs.iter().enumerate() {
                if let Some(mesh) = mesh {
                    parent.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(self.lit.clone()), LegGroup(group), NotShadowCaster));
                }
            }
            for part in &baked.extras {
                let base = Transform { translation: part.pos, rotation: part.rot, scale: part.size };
                parent.spawn((Mesh3d(self.primitive(part.shape)), MeshMaterial3d(self.paint(part.paint)), base, AnimPart { anim: part.anim, base }, NotShadowCaster));
            }
        });
    }
}

pub struct ArtPlugin;

impl Plugin for ArtPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, setup_art);
    }
}

fn setup_art(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let shapes = [
        (Shape::Cube, Cuboid::default().mesh().build(), false),
        (Shape::Shard, Sphere::new(1.0).mesh().ico(0).expect("valid subdivision"), true),
        (Shape::Cylinder, Cylinder::new(0.5, 1.0).mesh().resolution(8).build(), false),
        (Shape::Cone, Cone { radius: 0.5, height: 1.0 }.mesh().resolution(6).build(), true),
    ];
    let mut templates = HashMap::default();
    let mut primitives = HashMap::default();
    for (shape, mesh, flat) in shapes {
        let template = Template::new(mesh, flat);
        // Stand-alone primitives reuse the flattened data so they match baked models exactly.
        let standalone = Mesh::new(PrimitiveTopology::TriangleList, KEEP_FOR_BOUNDS)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, template.positions.iter().map(|p| p.to_array()).collect::<Vec<_>>())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, template.normals.iter().map(|n| n.to_array()).collect::<Vec<_>>());
        primitives.insert(shape, meshes.add(standalone));
        templates.insert(shape, template);
    }
    let paints = Paint::ALL
        .into_iter()
        .map(|paint| {
            let [r, g, b] = paint.rgb();
            (paint, materials.add(StandardMaterial { base_color: Color::srgb(r, g, b), unlit: paint.unlit(), perceptual_roughness: 0.95, ..default() }))
        })
        .collect();
    commands.insert_resource(Art {
        templates,
        primitives,
        lit: materials.add(StandardMaterial { perceptual_roughness: 0.95, ..default() }),
        unlit: materials.add(StandardMaterial { unlit: true, ..default() }),
        paints,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared meshes are spawned long after they are built. If their vertex
    /// data were dropped after upload, Bevy could not compute their bounds and
    /// robots would be culled while in plain view (the "flickering robot" bug).
    #[test]
    fn meshes_keep_what_culling_needs() {
        let builder = MeshBuilder { positions: vec![[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [0.0, 2.0, 1.0]], normals: vec![[0.0, 1.0, 0.0]; 3], colors: vec![[1.0; 4]; 3] };
        let mesh = builder.build();
        assert!(mesh.asset_usage.contains(RenderAssetUsages::MAIN_WORLD));
        use bevy::render::mesh::MeshAabb;
        let aabb = mesh.compute_aabb().expect("bounds");
        assert!((aabb.half_extents.y - 1.0).abs() < 1e-5);
    }
}
