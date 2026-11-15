//! VISOR client, step 1 of the Client build spec: load the bundle, build the terrain with the 0.5 m photo, frame it with an
//! orbit camera, and show the map basics (progress, north arrow, scale bar, credit, Curiosity's path layer).
//! No measured number or data size is hard-coded: sizes come from manifest.json; every word comes from text/en.json (compiled in).
use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::render::render_resource::TextureFormat;
use bevy::prelude::*;

mod bundle;
mod plan;
mod scene;
mod text;
mod ui;

use plan::{compute, crew, planner, share, simrun, sunlight};
use scene::{astronaut, basemap, curiosity, inspect, camera, checks, controls, labels, layers, mapview, mipmap, rover_path, terrain, trust, walk, walkscene};
use ui::{brief, card, chrome, hud, layerpanel, model, palette, pin};

use bundle::Inbox;

/// Files the first view needs (manifest.json first; the others are requested once the manifest is read).
// walk_back_s.bin stays in the bundle as the Python reference (route-core is tested against it); the app computes its own.
const FILES: [&str; 26] = [
    "manifest.json", "heightmap.bin", "ortho_0p5m.jpg", "rover_path.json", "route.json", "stops.json", "slope_3m.bin",
    "rover_check.json", "route_ensemble.json", "route_usage.bin", "conditions.json",
    "radiation.json", "readiness.bin", "readiness.json", "navcam_check.json",
    "ridges.json", "eva_ops.json", "astronaut.json", "astronaut.bin", "trek_layers.json",
    "curiosity.json", "curiosity.bin", "sun_check.json", "chemistry.json", "navcam_4718_points.bin", "panos.json",
];
/// Files the app works without (a missing one is skipped, not an error).
const OPTIONAL: [&str; 18] = ["panos.json", "navcam_4718_points.bin", "chemistry.json", "sun_check.json", "curiosity.json", "curiosity.bin", "trek_layers.json", "astronaut.json", "astronaut.bin", "ridges.json", "eva_ops.json", "route_ensemble.json", "route_usage.bin", "conditions.json", "radiation.json", "readiness.bin",
                              "readiness.json", "navcam_check.json"];

/// Bundle documents the planner needs again when it rebuilds the panel.
#[derive(Resource)]
pub struct BundleDocs {
    pub manifest: serde_json::Value,
    /// route uncertainty ensemble for the published plan (optional file)
    pub ensemble: Option<serde_json::Value>,
    /// stops.json: facts, sources, photos for the stop cards
    pub stops_doc: serde_json::Value,
    /// conditions.json: daily REMS weather for the study sols (optional file)
    pub conditions: Option<serde_json::Value>,
    /// radiation.json: RAD per stop sol and DONKI events (optional file)
    pub radiation: Option<serde_json::Value>,
    /// readiness.json (data readiness part B) and navcam_check.json, for the readiness table (optional files)
    pub readiness: Option<serde_json::Value>,
    pub navcam: Option<serde_json::Value>,
    /// ridges.json: ridge tracing summary (optional file; no map layer)
    pub ridges: Option<serde_json::Value>,
    /// eva_ops.json: sourced EVA values for the crew block (optional file)
    pub eva_ops: Option<serde_json::Value>,
    /// trek_layers.json: NASA Mars Trek layers over the patch (optional file)
    pub trek: Option<serde_json::Value>,
    /// sun_check.json: our sun model vs NASA Mars Trek's SPICE sun-angle service (optional file)
    pub sun_check: Option<serde_json::Value>,
    /// chemistry.json: ChemCam + APXS results along the walk (optional file)
    pub chem: Option<serde_json::Value>,
    /// rover_check.json (GeoJSON export of the rover-checked stops)
    pub rover_check: Option<serde_json::Value>,
    /// panos.json: Curiosity's Navcam 360-degree mosaic per stop (optional file; images load when opened)
    pub panos: Option<serde_json::Value>,
}

/// West longitude and planetographic latitude of the exit stop (stops.json lat/lon are planetocentric, east).
fn stops_doc_site(doc: Option<&serde_json::Value>) -> (f64, f64) {
    let s = doc.map(|d| &d["stops"][0]);
    let lon = s.and_then(|x| x["lon"].as_f64()).unwrap_or(f64::NAN);
    let lat = s.and_then(|x| x["lat"].as_f64()).unwrap_or(f64::NAN);
    (360.0 - lon, route_core::sun::planetographic(lat))
}

/// The shader warm-up camera (see build_scene) and the frames it lives.
#[derive(Component)]
pub struct WarmUp(u8);
const WARM_UP_FRAMES: u8 = 3;

fn warm_up(mut commands: Commands, mut q: Query<(Entity, &mut WarmUp)>, mut warm: ResMut<walk::WalkWarm>) {
    // Walk-only visuals stay on while the warm-up camera lives, so their shaders compile during loading (walk.rs WalkWarm)
    let live = !q.is_empty();
    if warm.0 != live {
        warm.0 = live;
    }
    for (e, mut w) in &mut q {
        if w.0 == 0 {
            commands.entity(e).despawn();
        } else {
            w.0 -= 1;
        }
    }
}

/// Approximate daytime Mars sky tint (an artistic background colour, not a measurement).
const SKY: Color = Color::srgb(0.77, 0.62, 0.48);
/// Colour of the vertical skirt around the cropped terrain block.
const SKIRT: Color = Color::srgb(0.22, 0.15, 0.11);

#[derive(Resource, Default)]
struct Loading {
    manifest: Option<serde_json::Value>,
    heightmap: Option<Vec<u8>>,
    ortho: Option<Vec<u8>>,
    rover_path: Option<serde_json::Value>,
    route: Option<serde_json::Value>,
    stops: Option<serde_json::Value>,
    slope: Option<Vec<u8>>,
    rover_check: Option<serde_json::Value>,
    conditions: Option<serde_json::Value>,
    radiation: Option<serde_json::Value>,
    ensemble: Option<serde_json::Value>,
    usage: Option<Vec<u8>>,
    ready: Option<Vec<u8>>,
    readiness: Option<serde_json::Value>,
    navcam: Option<serde_json::Value>,
    navcam_pts: Option<Vec<u8>>,
    panos: Option<serde_json::Value>,
    ridges: Option<serde_json::Value>,
    eva_ops: Option<serde_json::Value>,
    astro_meta: Option<serde_json::Value>,
    astro_bin: Option<Vec<u8>>,
    rover_meta: Option<serde_json::Value>,
    sun_check: Option<serde_json::Value>,
    chem: Option<serde_json::Value>,
    rover_bin: Option<Vec<u8>>,
    trek: Option<serde_json::Value>,
    received: usize,
    done: bool,
    error: Option<String>,
    /// files that have arrived (a retried request can deliver a second copy: ignored)
    got: Vec<String>,
    /// seconds since the last file arrived, and how many times the missing files were asked for again
    quiet_s: f32,
    retries: u32,
}

/// No file for this long (s) while loading: ask again for every missing file (a stalled request on a flaky connection or
/// a CDN mid-update otherwise left the page at "25 of 26" for minutes). VISUAL / usability choice.
const RETRY_AFTER_S: f32 = 10.0;
const MAX_RETRIES: u32 = 3;

/// Files still missing, in request order (manifest.json first until it arrives).
fn missing(st: &Loading) -> Vec<&'static str> {
    FILES.iter().copied().filter(|f| !st.got.iter().any(|g| g == f)).collect()
}

/// Stalled loading: re-request the missing files and say which one the page is waiting for.
fn loading_watchdog(time: Res<Time>, inbox: Res<Inbox>, mut st: ResMut<Loading>) {
    if st.done || st.error.is_some() {
        return;
    }
    st.quiet_s += time.delta_secs();
    if st.quiet_s < RETRY_AFTER_S || st.retries >= MAX_RETRIES {
        return;
    }
    st.quiet_s = 0.0;
    st.retries += 1;
    let todo: Vec<&str> = if st.manifest.is_none() { vec!["manifest.json"] } else { missing(&st) };
    for f in &todo {
        inbox.request_try(f, st.retries);
    }
    if let Some(first) = todo.first() {
        warn!("loading stalled; asking again for {} file(s), first {first} (retry {})", todo.len(), st.retries);
        chrome::status(&chrome::fill(text::loading::RETRY, &[("file", first.to_string()), ("n", st.retries.to_string())]));
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "VISOR".into(),
                canvas: Some("#visor".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(leafwing_input_manager::plugin::InputManagerPlugin::<controls::Act>::default())
        .add_plugins(trust::OverlayPlugin)
        .add_plugins(walkscene::WalkScenePlugin)
        .add_plugins(scene::armat::ArPlugin)
        .insert_resource(ClearColor(SKY))
        .init_resource::<Inbox>()
        .init_resource::<Loading>()
        .init_resource::<chrome::OverlayCache>()
        .init_resource::<walk::Mode>()
        .init_resource::<walk::WalkWarm>()
        .init_resource::<mapview::PlanView>()
        .init_resource::<walk::Stops>()
        .init_resource::<hud::HudState>()
        .init_resource::<labels::LabelCache>()
        .init_resource::<checks::Checks>()
        .init_resource::<pin::PinCache>()
        .init_resource::<scene::select::Selection>()
        .init_resource::<scene::place::Placements>()
        .init_resource::<scene::select::FlyTo>()
        .init_resource::<scene::select::Hover>()
        .init_resource::<scene::select::DragPreview>()
        .init_resource::<walk::WalkTarget>()
        .init_resource::<hud::Binoculars>()
        .init_resource::<model::UiBridge>()
        .init_resource::<planner::Robust>()
        .init_resource::<planner::PlanJob>()
        .init_resource::<planner::TourAsk>()
        .init_resource::<compute::Compute>()
        .init_resource::<share::LastPlan>()
        .init_resource::<card::CardState>()
        .init_resource::<crew::CrewState>()
        .init_resource::<crew::EvaPlan>()
        .init_resource::<plan::edits::StationEdits>()
        .add_systems(Update, plan::edits::update.before(card::update_card))
        .init_resource::<astronaut::View>()
        .init_resource::<simrun::SimRun>()
        .init_resource::<simrun::SunNow>()
        .init_resource::<astronaut::Track>()
        .add_systems(Startup, (start_loading, model::mount_ui))
        .add_systems(Update, (receive, build_scene, warm_up).chain())
        .add_systems(Update, loading_watchdog.before(receive))
        .add_systems(PreUpdate, controls::inject_ui_keys.after(bevy::input::InputSystems))
        .add_systems(Update, layerpanel::update.after(trust::toggle))
        .add_systems(Update, (walk::switch_mode, rover_path::toggle, scene::chem::toggle, layers::toggle))
        .add_systems(Update, camera::orbit_controls.run_if(mapview::in_orbit))
        // one selection on the Plan map: click -> station card / drill-in card / fly into 3D (scene/select.rs)
        .add_systems(Update, (scene::select::pick, scene::select::follow_card, scene::select::events, scene::place::events,
                              scene::select::apply, scene::place::eval).chain()
            .after(card::update_card))
        // placed relay / working-spot pins follow the camera (Plan and Walk)
        .add_systems(Update, scene::place::pins_follow.after(camera::orbit_controls).after(mapview::map_controls))
        .add_systems(Update, scene::select::fly.after(scene::select::apply).before(camera::orbit_controls))
        .add_systems(Update, scene::select::drag.before(planner::apply_events))
        .add_systems(Update, scene::select::show_rover.after(scene::select::apply))
        .add_systems(Update, scene::select::relay_run.before(scene::select::pick))
        .add_systems(Update, trust::update_back.after(trust::toggle))
        .add_systems(Update, scene::select::shade.after(camera::orbit_controls).after(mapview::map_controls).after(trust::toggle))
        .add_systems(Update, mapview::map_controls.run_if(mapview::in_map))
        .add_systems(Update, (mapview::toggle_view, mapview::apply_projection.after(mapview::toggle_view)))
        .add_systems(Update, simrun::update.after(share::handle).before(walk::walk_controls))
        .add_systems(Update, walk::walk_click.before(walk::walk_controls))
        .add_systems(Update, hud::binoculars.after(walk::walk_controls).after(mapview::apply_projection))
        .add_systems(Update, walk::walk_controls.run_if(walk::in_walk))
        .add_systems(Update, walkscene::update.after(walk::walk_controls))
        .add_systems(Update, layers::mode_parts)
        .add_systems(Update, basemap::update)
        .add_systems(Update, scene::navcam::update.after(walk::walk_controls))
        .add_systems(Update, (astronaut::toggle_view, astronaut::update.after(walk::walk_controls)).chain().before(walkscene::update))
        .add_systems(Update, curiosity::update.after(walk::walk_controls).run_if(resource_exists::<curiosity::Poses>))
        .add_systems(Update, inspect::update.run_if(resource_exists::<inspect::InspectData>))
        .add_systems(Update, (chrome::update_overlay, chrome::toggle_keys, walk::mode_fog, rover_path::scale_dots))
        .add_systems(Update, (trust::toggle, checks::toggle, checks::update_marks, pin::update_pin, pin::goto))
        .add_systems(Update, (planner::apply_events, planner::pick_stop, planner::replan, share::handle, card::update_card, sunlight::update_light, brief::update, crew::update).chain().before(trust::toggle))
        .add_systems(Update, (hud::toggle_hud, hud::update_hud.after(walk::walk_controls), labels::update_labels))
        .run();
}

fn start_loading(inbox: Res<Inbox>, commands: Commands) {
    chrome::attr("page", "style", &palette::css_vars());
    controls::spawn_controls(commands);
    chrome::text("credit", text::app::CREDIT_SHORT);
    inbox.request("manifest.json");
}

fn receive(inbox: Res<Inbox>, mut st: ResMut<Loading>) {
    for (name, r) in inbox.take() {
        // basemap images are asked for later, on demand (basemap.rs)
        if basemap::is_basemap_file(&name) {
            if let Ok(mut q) = basemap::ARRIVED.lock() {
                q.push((name, r));
            }
            continue;
        }
        if st.got.contains(&name) {
            continue;
        }
        let bytes = match r {
            Ok(b) => b,
            Err(e) if OPTIONAL.contains(&name.as_str()) => {
                info!("optional {name} not loaded: {e}");
                st.received += 1;
                st.got.push(name.clone());
                st.quiet_s = 0.0;
                continue;
            }
            Err(e) => {
                error!("could not load {name}: {e}");
                chrome::status(&chrome::fill(text::loading::FAILED, &[("file", name.clone())]));
                st.error = Some(format!("Could not load {name}: {e}"));
                continue;
            }
        };
        st.received += 1;
        st.got.push(name.clone());
        st.quiet_s = 0.0;
        match name.as_str() {
            "manifest.json" => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(m) => {
                    for f in &FILES[1..] {
                        inbox.request(f);
                    }
                    st.manifest = Some(m);
                }
                Err(e) => {
                    chrome::status(&FAILED_FALLBACK.replace("{file}", "manifest.json"));
                    st.error = Some(format!("manifest.json is not valid JSON: {e}"));
                }
            },
            "heightmap.bin" => st.heightmap = Some(bytes),
            "ortho_0p5m.jpg" => st.ortho = Some(bytes),
            "rover_path.json" => st.rover_path = serde_json::from_slice(&bytes).ok(),
            "route.json" => st.route = serde_json::from_slice(&bytes).ok(),
            "stops.json" => st.stops = serde_json::from_slice(&bytes).ok(),
            "slope_3m.bin" => st.slope = Some(bytes),
            "rover_check.json" => st.rover_check = serde_json::from_slice(&bytes).ok(),
            "route_ensemble.json" => st.ensemble = serde_json::from_slice(&bytes).ok(),
            "conditions.json" => st.conditions = serde_json::from_slice(&bytes).ok(),
            "radiation.json" => st.radiation = serde_json::from_slice(&bytes).ok(),
            "route_usage.bin" => st.usage = Some(bytes),
            "readiness.bin" => st.ready = Some(bytes),
            "readiness.json" => st.readiness = serde_json::from_slice(&bytes).ok(),
            "navcam_check.json" => st.navcam = serde_json::from_slice(&bytes).ok(),
            "ridges.json" => st.ridges = serde_json::from_slice(&bytes).ok(),
            "eva_ops.json" => st.eva_ops = serde_json::from_slice(&bytes).ok(),
            "astronaut.json" => st.astro_meta = serde_json::from_slice(&bytes).ok(),
            "astronaut.bin" => st.astro_bin = Some(bytes),
            "curiosity.json" => st.rover_meta = serde_json::from_slice(&bytes).ok(),
            "sun_check.json" => st.sun_check = serde_json::from_slice(&bytes).ok(),
            "chemistry.json" => st.chem = serde_json::from_slice(&bytes).ok(),
            "curiosity.bin" => st.rover_bin = Some(bytes),
            "navcam_4718_points.bin" => st.navcam_pts = Some(bytes),
            "panos.json" => st.panos = serde_json::from_slice(&bytes).ok(),
            "trek_layers.json" => st.trek = serde_json::from_slice(&bytes).ok(),
            _ => {}
        }
        if st.error.is_none() && !st.done {
            // name the file still on its way (not the one that just arrived), so a stall shows what it is waiting for
            let waiting = missing(&st).first().map(|f| f.to_string()).unwrap_or(name.clone());
            let msg = chrome::fill(text::loading::FILE, &[("file", waiting), ("done", st.received.to_string()), ("total", FILES.len().to_string())]);
            chrome::status(&msg);
        }
    }
}

fn build_scene(
    mut commands: Commands,
    mut st: ResMut<Loading>,
    windows: Query<&Window>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut terrain_mats: ResMut<Assets<trust::TerrainMaterial>>,
    mut skies: ResMut<Assets<walkscene::SkyMaterial>>,
    mut ar_mats: ResMut<Assets<scene::armat::ArMaterial>>,
    mut suit_mats: ResMut<Assets<scene::armat::SuitMaterial>>,
    mut binds: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
) {
    if st.done || st.error.is_some() || st.received < FILES.len() || st.manifest.is_none() || st.heightmap.is_none() || st.ortho.is_none() {
        return;
    }
    st.done = true;
    let m = st.manifest.as_ref().unwrap();
    let g = &m["grid"];
    let (w, h, cell) = (
        g["width"].as_u64().unwrap_or(0) as usize,
        g["height"].as_u64().unwrap_or(0) as usize,
        g["cell_m"].as_f64().unwrap_or(1.0) as f32,
    );
    let grid = match terrain::Grid::from_bytes(st.heightmap.as_ref().unwrap(), w, h, cell) {
        Ok(gr) => gr,
        Err(e) => {
            error!("{e}");
            chrome::status(&FAILED_FALLBACK.replace("{file}", "heightmap.bin"));
            st.error = Some(e);
            return;
        }
    };
    let photo = match Image::from_buffer(
        st.ortho.as_ref().unwrap(),
        ImageType::Extension("jpg"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::RENDER_WORLD,
    ) {
        Ok(i) => i,
        Err(e) => {
            error!("photo: {e}");
            chrome::status(&FAILED_FALLBACK.replace("{file}", "ortho_0p5m.jpg"));
            st.error = Some(format!("photo: {e}"));
            return;
        }
    };
    let photo = images.add(mipmap::with_mips(photo));
    commands.insert_resource(basemap::Basemaps::new(photo.clone(), st.trek.as_ref()));
    // the cast-shadow mask (W3), written in place when the planner worker answers
    let shadow_tex = images.add(trust::shadow_image(w, h));
    commands.insert_resource(sunlight::SunLight::new(shadow_tex.clone()));
    // the photo plus the trust layers (GPU shader, trust.rs); stand-in layer textures until the slope map is read below
    let material = terrain_mats.add(trust::TerrainMaterial {
        base: StandardMaterial {
            base_color_texture: Some(photo),
            unlit: true, // the HiRISE photo already carries real lighting
            ..default()
        },
        extension: trust::Overlay {
            params: default(),
            slope: images.add(trust::placeholder(TextureFormat::R32Float)),
            marks: images.add(trust::placeholder(TextureFormat::Rg8Unorm)),
            shadow: shadow_tex.clone(),
            ready: images.add(trust::placeholder(TextureFormat::R8Unorm)),
            hl: images.add(trust::highlight_image(w, h)),
            height: images.add(trust::slope_image(&grid)),
            back: images.add(trust::placeholder(TextureFormat::R32Float)),
        },
    });
    let terrain_mesh = meshes.add(grid.mesh());
    walkscene::spawn(&mut commands, &mut meshes, &mut skies);
    scene::armat::init(&mut ar_mats, &mut suit_mats);
    commands.spawn((Mesh3d(terrain_mesh), MeshMaterial3d(material.clone()), Name::new("terrain")));
    let skirt_mat = materials.add(StandardMaterial { base_color: SKIRT, unlit: true, cull_mode: None, ..default() });
    commands.spawn((Mesh3d(meshes.add(grid.skirt(20.0))), MeshMaterial3d(skirt_mat), Name::new("terrain skirt")));

    let pts: Vec<(f32, f32)> = st
        .rover_path
        .as_ref()
        .and_then(|v| v["points"].as_array())
        .map(|a| a.iter().filter_map(|p| Some((p["e_m"].as_f64()? as f32, p["n_m"].as_f64()? as f32))).collect())
        .unwrap_or_default();
    let n_pts = rover_path::spawn(&mut commands, &mut meshes, &mut materials, &grid, &pts);
    if let Some(c) = st.chem.as_ref() {
        scene::chem::spawn(&mut commands, &mut meshes, &mut materials, &grid, &scene::chem::points(c));
    }
    if let Some(rc) = st.rover_check.as_ref() {
        let fp = &rc["footprint_m"];
        let prints: Vec<rover_path::Footprint> = rc["stops"].as_array().map(|a| a.iter().filter_map(|s| Some(rover_path::Footprint {
            e: s["e_m"].as_f64()? as f32, n: s["n_m"].as_f64()? as f32, yaw_deg: s["yaw_deg"].as_f64()? as f32,
            label: s["label"].as_str()?.to_string() })).collect()).unwrap_or_default();
        if let (Some(along), Some(across)) = (fp["along"].as_f64(), fp["across"].as_f64()) {
            rover_path::spawn_footprints(&mut commands, &mut meshes, &mut materials, &grid, &prints, along as f32, across as f32);
        }
    }

    // The published route (route.json polyline) frames the map; the live planner (planner.rs) draws the route and flags.
    let poly: Vec<(f32, f32)> = st
        .route
        .as_ref()
        .and_then(|v| v["polyline_en"].as_array())
        .map(|a| a.iter().filter_map(|p| Some((p[0].as_f64()? as f32, p[1].as_f64()? as f32))).collect())
        .unwrap_or_default();
    let stops: Vec<walk::Stop> = st
        .stops
        .as_ref()
        .and_then(|v| v["stops"].as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| Some(walk::Stop {
                    id: s["id"].as_str()?.to_string(),
                    name: s["name"].as_str()?.to_string(),
                    e: s["e_m"].as_f64()? as f32,
                    n: s["n_m"].as_f64()? as f32,
                }))
                .collect()
        })
        .unwrap_or_default();
    if let Some(first) = stops.first() {
        let next = stops.get(1).unwrap_or(first);
        commands.insert_resource(walk::Walker {
            e: first.e,
            n: first.n,
            heading: walk::bearing_deg(first.e, first.n, next.e, next.n).to_radians(),
            pitch: 0.0,
            next: 1 % stops.len(),
        });
    }
    // the two suited astronauts for Walk mode (NASA 3D Resources model; optional files)
    if let (Some(meta), Some(bin), Some(first)) = (st.astro_meta.as_ref(), st.astro_bin.as_ref(), stops.first()) {
        match astronaut::parse(meta, bin) {
            Some(model) => astronaut::spawn(&mut commands, &mut meshes, &mut binds, model, (first.e, first.n), &grid),
            None => warn!("astronaut model could not be read; Walk mode runs without it"),
        }
    }
    // Navcam stereo 3D of the Nevado Sajama drill workspace (optional file), raised by the measured map - Navcam median
    if let Some(b) = st.navcam_pts.as_ref() {
        scene::navcam::spawn(&mut commands, &mut meshes, b, st.navcam.as_ref());
    }
    // Curiosity parked at its recorded pose at the stop nearest the walker (NASA Science model; optional files)
    if let (Some(meta), Some(bin), Some(sd), Some(rc)) = (st.rover_meta.as_ref(), st.rover_bin.as_ref(), st.stops.as_ref(), st.rover_check.as_ref()) {
        match curiosity::parse(meta, bin) {
            Some(mesh) => {
                let poses = curiosity::poses(sd, rc);
                curiosity::spawn(&mut commands, &mut meshes, &mut ar_mats, mesh, &poses, &grid);
                commands.insert_resource(curiosity::Poses(poses));
            }
            None => warn!("Curiosity model could not be read; Walk mode runs without it"),
        }
    }
    let n_stops = stops.len();
    let st_stops_en: Vec<(f32, f32)> = stops.iter().map(|s| (s.e, s.n)).collect();
    // Walking pace and the slope it depends on: both from the bundle (route.json assumptions, slope_3m.bin).
    let Some(pace) = st.route.as_ref().and_then(walk::Pace::from_route) else {
        chrome::status(&FAILED_FALLBACK.replace("{file}", "route.json"));
        st.error = Some("route.json has no walking-pace assumptions".into());
        return;
    };
    commands.insert_resource(pace);
    match terrain::Grid::from_bytes(st.slope.as_ref().unwrap(), w, h, cell) {
        Ok(sg) => {
            // the live planner: published stops and assumptions; inserting the settings triggers the first plan
            let route_v = st.route.as_ref().unwrap();
            let inputs = planner::PlanInputs {
                slope: sg.z.clone(),
                w,
                h,
                cell: cell as f64,
                rover: pts.iter().map(|&(e, n)| (e as f64, n as f64)).collect(),
                assumptions: route_v["assumptions"].clone(),
                published_stops: stops.clone(),
                published: (route_v["total_length_m"].as_f64().unwrap_or(0.0), route_v["total_time_s"].as_f64().unwrap_or(0.0)),
                heights: grid.z.clone(),
                // the sun's site: the exit stop (Mars24 wants west longitude and planetographic latitude)
                site: stops_doc_site(st.stops.as_ref()),
            };
            // a share link in the address (?v=1&...) sets the first plan; problems with the link show in the panel's note
            #[cfg(target_arch = "wasm32")]
            let first = {
                let q = share::web::query();
                let (mut s, mut warn) = share::decode(&q, &inputs, &share::bundle_id(&m));
                let hours = (route_core::light::FIRST_HOUR as f64, route_core::light::LAST_HOUR as f64);
                let (sol, hr, w2) = share::decode_light(&q, sunlight::SOL_RANGE, hours);
                warn.extend(w2);
                if let Some(x) = sol {
                    sunlight::send(sunlight::LightEvent::Sol(x));
                }
                if let Some(h) = hr {
                    sunlight::send(sunlight::LightEvent::Hour(h));
                }
                s.note = warn.join(" ");
                s
            };
            #[cfg(not(target_arch = "wasm32"))]
            let first = inputs.defaults();
            commands.insert_resource(first);
            commands.insert_resource(inputs);
            commands.insert_resource(BundleDocs { manifest: m.clone(), ensemble: st.ensemble.clone(), stops_doc: st.stops.clone().unwrap_or_default(), conditions: st.conditions.clone(), radiation: st.radiation.clone(),
                                              readiness: st.readiness.clone(), navcam: st.navcam.clone(),
                                              ridges: st.ridges.clone(), eva_ops: st.eva_ops.clone(), trek: st.trek.clone(),
                                              sun_check: st.sun_check.clone(), rover_check: st.rover_check.clone(),
                                              chem: st.chem.clone(), panos: st.panos.clone() });
            // trust layers: corrected-slope bands (key 2) and rover-verified ground (key 4)
            let a = &st.route.as_ref().unwrap()["assumptions"];
            let num = |k: &str| a[k].as_f64().unwrap_or(f64::NAN) as f32;
            let marks = trust::marks_texels(sg.w, sg.h, &pts, num("verified_radius_m"), st.usage.as_deref());
            let (slope_tex, marks_tex) = (images.add(trust::slope_image(&sg)), images.add(trust::marks_image(sg.w, sg.h, marks)));
            if let Some(mut tm) = terrain_mats.get_mut(&material) {
                tm.extension.slope = slope_tex;
                tm.extension.marks = marks_tex;
            }
            let mut layers = trust::Overlays::new(material.clone(), pace.bias_deg, num("caution_deg"), num("slope_limit_deg"));
            // data readiness (key 0): only with a class byte per cell and its legend
            if let (Some(b), Some(_)) = (st.ready.as_ref().filter(|b| b.len() == sg.w * sg.h), st.readiness.as_ref()) {
                let t = images.add(trust::ready_image(sg.w, sg.h, b.clone()));
                if let Some(mut tm) = terrain_mats.get_mut(&material) {
                    tm.extension.ready = t;
                }
                layers.has_ready = true;
            }
            layers.push(&mut terrain_mats);
            commands.insert_resource(layers);
            // point readout (Plan mode): the grids' values under the cursor
            let classes: Vec<(u8, String)> = st.readiness.as_ref().and_then(|r| r["classes"].as_object())
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.parse::<u8>().ok()?, v.as_str()?.to_string()))).collect()).unwrap_or_default();
            let rover: Vec<(f32, f32, i64, String)> = st.rover_check.as_ref().and_then(|r| r["stops"].as_array())
                .map(|a| a.iter().filter_map(|x| Some((x["e_m"].as_f64()? as f32, x["n_m"].as_f64()? as f32, x["sol"].as_i64()?,
                                                         x["label"].as_str()?.to_string()))).collect()).unwrap_or_default();
            commands.insert_resource(inspect::InspectData {
                chem: st.chem.clone(),
                ready: st.ready.clone().filter(|b| b.len() == sg.w * sg.h), classes, rover,
                z_offset: m["grid"]["z_offset_m"].as_f64().unwrap_or(0.0) as f32, grid: m["grid"].clone(), bias: pace.bias_deg,
            });
            inspect::create_element();
            commands.insert_resource(walk::SlopeMap(sg));
        }
        Err(e) => {
            chrome::status(&FAILED_FALLBACK.replace("{file}", "slope_3m.bin"));
            st.error = Some(e);
            return;
        }
    }
    // Top bar: rover-verified ground (route data and the walk-back field come from the live planner).
    let route_v = st.route.as_ref().unwrap();
    commands.insert_resource(hud::RoverGround {
        pts: pts.clone(),
        radius_m: route_v["assumptions"]["verified_radius_m"].as_f64().unwrap_or(0.0) as f32,
    });
    // rover-check marks (key 3)
    let rc = st.rover_check.as_ref().map(checks::parse).unwrap_or_default();
    checks::create_marks(&rc);
    commands.insert_resource(checks::Checks { items: rc, shown: false, cache: Vec::new() }); // off until the rail turns it on

    // Frame the whole terrain: fit the terrain's bounding box (incl. the skirt) to the window, viewed from the south looking north.
    let aspect = windows.single().map(|w| w.width() / w.height().max(1.0)).unwrap_or(16.0 / 9.0);
    let persp = PerspectiveProjection { far: 20_000.0, ..default() };
    let (pitch, fill) = (55f32.to_radians(), 0.92);
    let bmin = Vec3::new(0.0, grid.min_height() - 20.0, -(h as f32) * cell);
    let bmax = Vec3::new(w as f32 * cell, grid.max_height(), 0.0);
    let dist = camera::fit_box(bmin, bmax, 0.0, pitch, persp.fov, aspect, fill);
    let orbit = camera::Orbit {
        focus: (bmin + bmax) / 2.0,
        yaw: 0.0,
        pitch,
        radius: dist,
        min_radius: 20.0,
        max_radius: dist * 3.0,
    };
    // Plan mode opens on the top-down map, framed on the planned loop and its stops, right of the planning panel.
    let win_px = windows.single().map(|w| Vec2::new(w.width(), w.height())).unwrap_or(Vec2::new(1280.0, 800.0));
    let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
    for &(e, n) in poly.iter().chain(st_stops_en.iter()) {
        lo = (lo.0.min(e), lo.1.min(n));
        hi = (hi.0.max(e), hi.1.max(n));
    }
    if lo.0 > hi.0 {
        lo = (0.0, 0.0);
        hi = (w as f32 * cell, h as f32 * cell);
    }
    let map = mapview::fit(lo, hi, win_px, (w.max(h)) as f32 * cell);
    commands.insert_resource(map);
    commands.insert_resource(mapview::PerspectiveSettings(persp.clone()));
    // Shader warm-up: a tiny off-screen perspective view for the first frames, so the GPU compiles the Walk/3D shader
    // variants while the page is still loading, not on the first switch to Walk (a 267 ms freeze there, measured).
    // Same view settings as the main camera; removed after WARM_UP_FRAMES.
    let warm_at = orbit.transform();
    let target = images.add(Image::new_target_texture(64, 64, TextureFormat::Rgba8UnormSrgb, None));
    commands.spawn((
        Camera3d::default(),
        Camera { order: -1, ..default() },
        bevy::camera::RenderTarget::Image(target.into()),
        Projection::Perspective(persp.clone()),
        bevy::render::view::Tonemapping::None,
        bevy::render::view::DebandDither::Disabled,
        DistanceFog { color: SKY, falloff: FogFalloff::Linear { start: dist * 1.2, end: dist * 3.5 }, ..default() },
        warm_at,
        WarmUp(WARM_UP_FRAMES),
    ));
    commands.spawn((
        Camera3d::default(),
        mapview::ortho(map.height_m),
        // Keep the HiRISE photo's own colours (tone mapping would regrade even unlit materials).
        bevy::render::view::Tonemapping::None,
        bevy::render::view::DebandDither::Disabled, // no dithering without tone mapping (Bevy warns otherwise)
        // Light haze toward the sky colour far from the camera, so the crop edge reads as distance rather than a cut.
        DistanceFog { color: SKY, falloff: FogFalloff::Linear { start: dist * 1.2, end: dist * 3.5 }, ..default() },
        map.transform(grid.max_height()),
        orbit,
    ));
    chrome::hide_status();
    info!("terrain {w} x {h} cells at {cell} m built; {n_pts} Curiosity positions; {n_stops} stops; route {} points; camera at {dist:.0} m", poly.len());
    commands.insert_resource(walk::PlanFog(dist * 1.2, dist * 3.5));
    commands.insert_resource(walk::Ground(grid));
    commands.insert_resource(walk::initial_mode());
}

/// Shown when a bundle file is missing or broken (text/en.json loading.failed).
const FAILED_FALLBACK: &str = text::loading::FAILED;

/// Tests against the real bundle in ../assets (run natively: `cargo test -p visor-app` from the repository root).
#[cfg(test)]
mod real_bundle {
    use super::*;
    use crate::ui::panel;

    fn read(name: &str) -> Vec<u8> {
        let inbox = Inbox::default();
        inbox.request(name);
        let mut got = inbox.take();
        assert_eq!(got.len(), 1);
        got.remove(0).1.unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn manifest() -> serde_json::Value {
        serde_json::from_slice(&read("manifest.json")).expect("manifest.json parses")
    }

    #[test]
    fn heightmap_matches_manifest_and_builds_a_finite_mesh() {
        let m = manifest();
        let (w, h) = (m["grid"]["width"].as_u64().unwrap() as usize, m["grid"]["height"].as_u64().unwrap() as usize);
        let cell = m["grid"]["cell_m"].as_f64().unwrap() as f32;
        let g = terrain::Grid::from_bytes(&read("heightmap.bin"), w, h, cell).expect("size matches manifest");
        let mesh = g.mesh();
        assert_eq!(mesh.count_vertices(), w * h);
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            assert!(p.iter().all(|v| v.iter().all(|x| x.is_finite())), "non-finite vertex");
        } else {
            panic!("no positions");
        }
    }

    #[test]
    fn photo_decodes_at_twice_the_grid_and_gets_a_full_mip_chain() {
        let m = manifest();
        let (w, h) = (m["grid"]["width"].as_u64().unwrap() as u32, m["grid"]["height"].as_u64().unwrap() as u32);
        let img = Image::from_buffer(&read("ortho_0p5m.jpg"), ImageType::Extension("jpg"), CompressedImageFormats::NONE, true,
                                     ImageSampler::linear(), RenderAssetUsages::RENDER_WORLD).expect("jpeg decodes");
        let s = img.size();
        assert_eq!((s.x, s.y), (2 * w, 2 * h), "0.5 m photo must cover exactly the 1 m grid");
        assert!(s.x <= 4096 && s.y <= 4096, "texture over 4096 px");
        let img = mipmap::with_mips(img);
        assert_eq!(img.texture_descriptor.mip_level_count, mipmap::level_count(s.x, s.y));
    }

    #[test]
    fn rover_path_points_lie_on_the_terrain() {
        let m = manifest();
        let (w, h) = (m["grid"]["width"].as_u64().unwrap() as usize, m["grid"]["height"].as_u64().unwrap() as usize);
        let g = terrain::Grid::from_bytes(&read("heightmap.bin"), w, h, 1.0).unwrap();
        let zoff = m["grid"]["z_offset_m"].as_f64().unwrap() as f32;
        let rp: serde_json::Value = serde_json::from_slice(&read("rover_path.json")).unwrap();
        for p in rp["points"].as_array().unwrap() {
            let (e, n, el) = (p["e_m"].as_f64().unwrap() as f32, p["n_m"].as_f64().unwrap() as f32, p["dem_elev_m"].as_f64().unwrap() as f32);
            // bilinear height within a metre of the cell value the pipeline recorded
            assert!((g.height_at(e, n) + zoff - el).abs() < 1.0, "sol {} off the terrain", p["sol"]);
        }
    }

    #[test]
    fn route_and_stops_load_and_lie_inside_the_grid() {
        let m = manifest();
        let (w, h) = (m["grid"]["width"].as_f64().unwrap(), m["grid"]["height"].as_f64().unwrap());
        let route: serde_json::Value = serde_json::from_slice(&read("route.json")).unwrap();
        let poly = route["polyline_en"].as_array().unwrap();
        assert!(poly.len() > 100, "route polyline too short");
        assert!(poly.iter().all(|p| { let (e, n) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap()); e >= 0.0 && e < w && n >= 0.0 && n < h }));
        let stops: serde_json::Value = serde_json::from_slice(&read("stops.json")).unwrap();
        let s = stops["stops"].as_array().unwrap();
        assert!(s.len() >= 2);
        assert!(s.iter().all(|x| x["e_m"].as_f64().is_some() && x["n_m"].as_f64().is_some() && x["name"].as_str().is_some()));
        // the walker's pace comes from the route's own assumptions, so walking takes the time the route says
        let pace = walk::Pace::from_route(&route).expect("route.json carries the pace assumptions");
        assert_eq!(pace.speed(0.0), route["assumptions"]["speed_flat_mps"].as_f64().unwrap() as f32);
        let (w_, h_) = (w as usize, h as usize);
        let slope = terrain::Grid::from_bytes(&read("slope_3m.bin"), w_, h_, 1.0).expect("slope grid matches the heightmap grid");
        assert!(slope.z.iter().all(|v| v.is_finite() && (0.0..90.0).contains(v)), "slopes are finite degrees");
    }

    /// The top bar's time to the next stop uses the route's own edge formula: from each leg's start it must reproduce
    /// route.json's leg time (rounded to 1 s there) and length.
    #[test]
    fn top_bar_reproduces_the_route_leg_times() {
        let m = manifest();
        let (w, h) = (m["grid"]["width"].as_u64().unwrap() as usize, m["grid"]["height"].as_u64().unwrap() as usize);
        let route: serde_json::Value = serde_json::from_slice(&read("route.json")).unwrap();
        let pace = walk::Pace::from_route(&route).unwrap();
        let slope = terrain::Grid::from_bytes(&read("slope_3m.bin"), w, h, 1.0).unwrap();
        let poly: Vec<(f32, f32)> = route["polyline_en"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32)).collect();
        let legs_v = route["legs"].as_array().unwrap();
        let legs: Vec<(usize, usize)> = legs_v.iter().map(|l| (l["poly_start"].as_u64().unwrap() as usize, l["poly_end"].as_u64().unwrap() as usize)).collect();
        let rd = hud::RouteData { poly: poly.clone(), legs: legs.clone() };
        for (i, l) in legs_v.iter().enumerate() {
            let p0 = poly[legs[i].0];
            let (d, t, off) = hud::remaining(&rd, i, p0.0, p0.1, &slope, &pace).unwrap();
            assert!(off < 1e-4);
            assert!((t - l["time_s"].as_f64().unwrap() as f32).abs() <= 1.0, "leg {i}: time {t} vs route {}", l["time_s"]);
            assert!((d - l["length_m"].as_f64().unwrap() as f32).abs() <= 0.1, "leg {i}: length {d} vs route {}", l["length_m"]);
        }
        // walk-back grid: zero at the exit stop, and equal to each stop's walk_back_s
        let wb = read("walk_back_s.bin");
        let field = hud::WalkBack { w, h, cell: 1.0, s: wb.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).map(|v| if v == 65535 { f32::INFINITY } else { v as f32 }).collect() };
        let stops: serde_json::Value = serde_json::from_slice(&read("stops.json")).unwrap();
        for s in stops["stops"].as_array().unwrap() {
            let (e, n) = (s["e_m"].as_f64().unwrap() as f32, s["n_m"].as_f64().unwrap() as f32);
            let want = s["walk_back_s"].as_f64().unwrap() as f32;
            assert_eq!(field.at(e, n), Some(want), "walk-back at {}", s["id"]);
        }
    }

    /// The panel's profile and summary agree with route.json: same cells, same bias, same per-leg maxima.
    #[test]
    fn relay_suggestion_on_the_real_bundle() {
        let m = manifest();
        let (w, h) = (m["grid"]["width"].as_u64().unwrap() as usize, m["grid"]["height"].as_u64().unwrap() as usize);
        let route: serde_json::Value = serde_json::from_slice(&read("route.json")).unwrap();
        let heights = terrain::Grid::from_bytes(&read("heightmap.bin"), w, h, 1.0).unwrap();
        let slope = terrain::Grid::from_bytes(&read("slope_3m.bin"), w, h, 1.0).unwrap();
        let poly: Vec<(f32, f32)> = route["polyline_en"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32)).collect();
        let s = panel::profile(&poly, &heights, &slope, route["assumptions"]["bias_deg"].as_f64().unwrap() as f32);
        // the bundle's walk-back field (Python reference, u16 s, 65535 = no way back)
        let wb: Vec<f32> = read("walk_back_s.bin").chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).map(|v| if v == 65535 { f32::INFINITY } else { v as f32 }).collect();
        let field = hud::WalkBack { w, h, cell: 1.0, s: wb };
        let t0 = std::time::Instant::now();
        let r = plan::relay::suggest(&heights, &s, &|e, n| field.at(e, n)).expect("a relay point for the published route");
        let ms = t0.elapsed().as_millis();
        println!("relay at ({:.1}, {:.1}): in sight {:.1}% -> {:.1}%, walk-back {:.0} s, {:.0} m from the exit, {ms} ms native",
                 r.e, r.n, r.before * 100.0, r.after * 100.0, r.back_s, r.from_exit_m);
        assert!(r.after > r.before && r.back_s <= plan::relay::RELAY_MAX_BACK_S);
    }

    #[test]
    fn panel_profile_and_summary_agree_with_the_route() {
        let m = manifest();
        let (w, h) = (m["grid"]["width"].as_u64().unwrap() as usize, m["grid"]["height"].as_u64().unwrap() as usize);
        let route: serde_json::Value = serde_json::from_slice(&read("route.json")).unwrap();
        let stops: serde_json::Value = serde_json::from_slice(&read("stops.json")).unwrap();
        let heights = terrain::Grid::from_bytes(&read("heightmap.bin"), w, h, 1.0).unwrap();
        let slope = terrain::Grid::from_bytes(&read("slope_3m.bin"), w, h, 1.0).unwrap();
        let poly: Vec<(f32, f32)> = route["polyline_en"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32)).collect();
        let bias = route["assumptions"]["bias_deg"].as_f64().unwrap() as f32;
        let s = panel::profile(&poly, &heights, &slope, bias);
        assert!((s.last().unwrap().d as f64 - route["total_length_m"].as_f64().unwrap()).abs() < 0.2);
        for (i, l) in route["legs"].as_array().unwrap().iter().enumerate() {
            let (a, b) = (l["poly_start"].as_u64().unwrap() as usize, l["poly_end"].as_u64().unwrap() as usize);
            let mx = s[a..=b].iter().map(|x| x.s).fold(0.0f32, f32::max) as f64;
            assert!((mx - l["max_corrected_slope_deg"].as_f64().unwrap()).abs() <= 0.06, "leg {i}: {mx}");
        }
        let sm = panel::summary(&route, stops["stops"].as_array().unwrap());
        assert_eq!(sm.length_m, route["total_length_m"].as_f64().unwrap());
        assert_eq!(sm.feasible, route["all_legs_possible"].as_bool().unwrap());
        let backs: Vec<f64> = stops["stops"].as_array().unwrap().iter().map(|x| x["walk_back_s"].as_f64().unwrap()).collect();
        assert_eq!(sm.longest_back_s, backs.iter().cloned().fold(0.0, f64::max));
    }

    /// The panel model built from the real bundle: every leg has a row, stops are numbered, measured values filled.
    #[test]
    fn panel_model_from_the_real_bundle() {
        let route: serde_json::Value = serde_json::from_slice(&read("route.json")).unwrap();
        let stops: serde_json::Value = serde_json::from_slice(&read("stops.json")).unwrap();
        let st = stops["stops"].as_array().unwrap();
        let m = panel::model(&route, st, &manifest(), Vec::new(), model::Controls::default());
        assert_eq!(m.legs.len(), route["legs"].as_array().unwrap().len());
        assert_eq!(m.stops.len(), st.len());
        assert!(m.stops[1].starts_with("2 "), "numbered like the map labels: {}", m.stops[1]);
        assert!(m.vals.iter().all(|(_, v)| !v.is_empty() && v != "NaN"), "every template value filled: {:?}", m.vals);
        assert!(!m.rover_counts.0.is_empty());
    }

    #[test]
    fn stop_cards_from_the_real_bundle() {
        let doc: serde_json::Value = serde_json::from_slice(&read("stops.json")).unwrap();
        let ens: serde_json::Value = serde_json::from_slice(&read("route_ensemble.json")).unwrap();
        let st: Vec<walk::Stop> = doc["stops"].as_array().unwrap().iter().map(|x| walk::Stop {
            id: x["id"].as_str().unwrap().into(), name: x["name"].as_str().unwrap().into(),
            e: x["e_m"].as_f64().unwrap() as f32, n: x["n_m"].as_f64().unwrap() as f32 }).collect();
        let plan: Vec<serde_json::Value> = doc["stops"].as_array().unwrap().clone();
        for i in 0..st.len() {
            let c = card::build(i, &st, &doc, &plan, Some(&ens), true, 3.4, &Default::default());
            assert!(c.open && !c.title.is_empty() && c.kicker.starts_with(&format!("Stop {} of {}", i + 1, st.len())), "{c:?}");
            let p = c.photo.as_ref().expect("every published stop has its Navcam photo");
            assert!(std::path::Path::new(&format!("../{}", p.src)).exists(), "photo file missing: {}", p.src);
            assert!(p.caption.contains("NASA/JPL-Caltech") && p.raw_url.starts_with("https://"), "{p:?}");
            assert!(c.facts.iter().all(|f| f.url.starts_with("https://") && !f.title.is_empty() && !f.meta.is_empty()), "{:?}", c.facts);
            assert!(c.stats.len() == 3, "walk back, map slope, rover check: {:?}", c.stats);
            assert!(!c.about.is_empty());
            if i > 0 {
                assert!(!c.facts.is_empty(), "{} has no sourced fact", c.title);
                assert!(c.stats[0].note.contains("1000 runs"), "ensemble range on the published plan: {}", c.stats[0].note);
            }
        }
        // the mineral pairings are labelled as our inference, nothing else is
        let inferred: Vec<String> = (0..st.len()).flat_map(|i| card::build(i, &st, &doc, &plan, None, true, 3.4, &Default::default()).facts)
            .filter(|f| f.inference).map(|f| f.text).collect();
        assert_eq!(inferred.len(), 2, "{inferred:?}");
        assert!(inferred.iter().all(|t| t.contains("inference")));
        // a stop added on the map gets no facts, and points to the nearest Curiosity stop
        let mut st2 = st.clone();
        st2.push(walk::Stop { id: "custom-1".into(), name: "Custom stop 1".into(), e: st[1].e + 3.0, n: st[1].n + 4.0 });
        let c = card::build(st2.len() - 1, &st2, &doc, &plan, None, false, 3.4, &Default::default());
        assert!(c.custom && c.facts.is_empty() && c.custom_note.contains("Bococo") && c.custom_note.contains("5 m"), "{c:?}");
        // ... and shows the nearest Curiosity photo, marked as taken there
        let p = c.photo.as_ref().expect("nearest stop photo");
        assert!(p.borrowed && p.caption.contains("Bococo") && p.caption.contains("not here"), "{p:?}");
    }

    #[test]
    fn rover_checks_parse_with_known_labels() {
        let v: serde_json::Value = serde_json::from_slice(&read("rover_check.json")).unwrap();
        let c = checks::parse(&v);
        assert_eq!(c.len(), v["stops"].as_array().unwrap().len(), "every rover-check stop parses");
        assert!(c.iter().all(|x| ["agrees", "partial", "disagrees"].contains(&x.label.as_str())));
    }

    #[test]
    fn missing_file_reports_an_error() {
        let inbox = Inbox::default();
        inbox.request("no_such_file.bin");
        assert!(inbox.take().remove(0).1.is_err());
    }
}
