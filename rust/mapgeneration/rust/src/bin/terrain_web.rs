use axum::{
    extract::Path,
    extract::Query,
    extract::State,
    http::header,
    http::HeaderValue,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use postgres::{Client, NoTls};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct AppState {
    map_cache: Arc<tokio::sync::RwLock<HashMap<String, CachedMap>>>,
    map_cache_ttl: Duration,
    icons_cache: Arc<tokio::sync::RwLock<HashMap<String, CachedIcons>>>,
    icons_cache_ttl: Duration,
}

struct CachedMap {
    png: Vec<u8>,
    created_at: Instant,
}

struct CachedIcons {
    json: Vec<u8>,
    created_at: Instant,
}

#[derive(Clone)]
struct TerrainRaster {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    min_chunk_x: i32,
    max_chunk_y: i32,
}

#[derive(Clone)]
struct CaveClaim {
    name: String,
    north: i32,
    east: i32,
}

#[derive(Clone)]
struct WatchtowerClaim {
    name: String,
    north: i32,
    east: i32,
}

#[derive(Clone)]
struct TempleClaim {
    name: String,
    north: i32,
    east: i32,
}

#[derive(Clone)]
struct DungeonClaim {
    name: String,
    north: i32,
    east: i32,
}

#[derive(Serialize)]
struct Marker {
    name: String,
    marker_type: String,
    icon_key: String,
    x: f32,
    y: f32,
}

#[derive(Serialize)]
struct MarkerResponse {
    width: usize,
    height: usize,
    markers: Vec<Marker>,
}

#[derive(Deserialize)]
struct TerrainQuery {
    dimension: Option<i32>,
    format: Option<String>,
    quality: Option<u8>,
    resource_id: Option<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MapFormat {
    Png,
    Webp,
}

fn parse_map_format(value: Option<&str>) -> MapFormat {
    match value {
        Some(v) if v.eq_ignore_ascii_case("webp") => MapFormat::Webp,
        _ => MapFormat::Png,
    }
}

fn clamp_jpeg_quality(value: Option<u8>) -> u8 {
    value.unwrap_or(82).clamp(30, 95)
}

fn json_array_to_u32_vec(value: &Value) -> Vec<u32> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.as_u64()
                    .map(|num| num as u32)
                    .or_else(|| item.as_i64().map(|num| (num as i32) as u32))
                    .or_else(|| item.as_f64().map(|num| num as i64 as u32))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn normalize_biome_id(raw_biome_id: u32) -> u8 {
    const MAX_KNOWN_BIOME_ID: u8 = 14;

    let low_byte = (raw_biome_id & 0xFF) as u8;
    if low_byte <= MAX_KNOWN_BIOME_ID {
        return low_byte;
    }

    for shift in [8_u32, 16_u32, 24_u32] {
        let candidate = ((raw_biome_id >> shift) & 0xFF) as u8;
        if candidate <= MAX_KNOWN_BIOME_ID {
            return candidate;
        }
    }

    low_byte
}

fn water_color(water_type: u32) -> [u8; 3] {
    let water_type_u8: u8 = (water_type & 0xFF) as u8;
    match water_type_u8 {
        0 => [50, 137, 220],
        1 => [70, 130, 180],
        2 => [30, 144, 255],
        3 => [0, 132, 186],
        4 => [0, 105, 148],
        5 => [47, 79, 79],
        _ => [25, 25, 112],
    }
}

fn biome_color(biome_id: u32) -> [u8; 3] {
    let biome_id_u8 = normalize_biome_id(biome_id);
    match biome_id_u8 {
        0 => [255, 25, 0],
        1 => [34, 139, 34],
        2 => [12, 48, 11],
        3 => [1, 1, 10],
        4 => [130, 230, 130],
        5 => [210, 105, 30],
        6 => [100, 40, 60],
        7 => [237, 201, 175],
        8 => [55, 110, 87],
        9 => [112, 128, 144],
        10 => [255, 240, 200],
        11 => [220, 255, 110],
        12 => [105, 105, 105],
        13 => [60, 100, 30],
        14 => [100, 90, 108],
        _ => [0, 0, 0],
    }
}

fn brighten(color: [u8; 3], amount: u8) -> [u8; 3] {
    [
        color[0].saturating_add(amount),
        color[1].saturating_add(amount),
        color[2].saturating_add(amount),
    ]
}

fn set_pixel_rgba(buffer: &mut [u8], width: usize, x: usize, y: usize, color: [u8; 3]) {
    let index = (y * width + x) * 4;
    if index + 3 >= buffer.len() {
        return;
    }

    buffer[index] = color[0];
    buffer[index + 1] = color[1];
    buffer[index + 2] = color[2];
    buffer[index + 3] = 255;
}

fn parse_claim_coord(name: &str, key: char) -> Option<i32> {
    let marker = format!("{key}:");
    let marker_index = name.find(&marker)?;
    let after_marker = &name[marker_index + marker.len()..];
    let trimmed = after_marker.trim_start();

    let mut end = 0usize;
    for (index, character) in trimmed.char_indices() {
        if !(character.is_ascii_digit() || (index == 0 && (character == '-' || character == '+'))) {
            break;
        }
        end = index + character.len_utf8();
    }

    if end == 0 {
        return None;
    }

    trimmed[..end].parse::<i32>().ok()
}

fn parse_cave_claim(name: &str) -> Option<CaveClaim> {
    let north = parse_claim_coord(name, 'N')?;
    let east = parse_claim_coord(name, 'E')?;

    Some(CaveClaim {
        name: name.to_string(),
        north,
        east,
    })
}

fn parse_watchtower_claim(name: &str) -> Option<WatchtowerClaim> {
    let parts = name.split("|~").collect::<Vec<_>>();
    if parts.len() < 4 || !parts[1].to_lowercase().contains("watchtower") {
        return None;
    }

    Some(WatchtowerClaim {
        name: parts[0].trim().to_string(),
        north: parts[2].trim().parse().ok()?,
        east: parts[3].trim().parse().ok()?,
    })
}

fn parse_temple_claim(name: &str) -> Option<TempleClaim> {
    let lowered = name.to_lowercase();
    if !lowered.contains("temple") {
        return None;
    }

    let parts = name.split("|~").collect::<Vec<_>>();
    if parts.len() >= 4 && parts[1].to_lowercase().contains("temple") {
        return Some(TempleClaim {
            name: parts[0].trim().to_string(),
            north: parts[2].trim().parse().ok()?,
            east: parts[3].trim().parse().ok()?,
        });
    }

    let north = parse_claim_coord(name, 'N')?;
    let east = parse_claim_coord(name, 'E')?;

    Some(TempleClaim {
        name: name.to_string(),
        north,
        east,
    })
}

fn parse_dungeon_claim(name: &str) -> Option<DungeonClaim> {
    let lowered = name.to_lowercase();
    if !lowered.contains("dungeon") {
        return None;
    }

    let parts = name.split("|~").collect::<Vec<_>>();
    if parts.len() >= 4 && parts[1].to_lowercase().contains("dungeon") {
        return Some(DungeonClaim {
            name: parts[0].trim().to_string(),
            north: parts[2].trim().parse().ok()?,
            east: parts[3].trim().parse().ok()?,
        });
    }

    let north = parse_claim_coord(name, 'N')?;
    let east = parse_claim_coord(name, 'E')?;

    Some(DungeonClaim {
        name: name.to_string(),
        north,
        east,
    })
}

const SPECIAL_CAVE_RESOURCES: [&str; 8] = [
    "emarium",
    "pyrelite",
    "rathium",
    "ferralith",
    "celestium",
    "aurumite",
    "luminite",
    "elenvar",
];

fn cave_resource_in_name(name: &str) -> Option<&'static str> {
    let lowered = name.to_lowercase();
    SPECIAL_CAVE_RESOURCES
        .iter()
        .copied()
        .find(|resource| lowered.contains(resource))
}

fn world_to_map(east: i32, north: i32, raster: &TerrainRaster) -> (f32, f32) {
    let map_x = east as f32 - (raster.min_chunk_x as f32 * 32.0);
    let max_world_y = raster.max_chunk_y as f32 * 32.0 + 31.0;
    let map_y = max_world_y - north as f32;

    (
        map_x.clamp(0.0, raster.width as f32),
        map_y.clamp(0.0, raster.height as f32),
    )
}

fn get_database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bitcraft:bitcraft_pw@localhost:5431/bitcraft_hub".to_string())
}

fn get_map_cache_ttl() -> Duration {
    let seconds = std::env::var("TERRAIN_CACHE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(3600);
    Duration::from_secs(seconds)
}

fn get_icons_cache_ttl() -> Duration {
    let seconds = std::env::var("ICONS_CACHE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(3600);
    Duration::from_secs(seconds)
}

fn build_raster(dimension: i32) -> Result<TerrainRaster, Box<dyn Error>> {
    let mut client = Client::connect(&get_database_url(), NoTls)?;
    let rows = client.query(
        r#"SELECT chunk_x,
   chunk_z,
   biomes::text,
   water_levels::text,
   water_body_types::text,
   original_elevations::text
FROM terrain_chunk_state
WHERE "dimension" = $1"#,
        &[&dimension],
    )?;

    let mut data: Vec<(i32, i32, Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>)> = Vec::new();

    for row in rows {
        let x: i32 = row.get(0);
        let y: i32 = row.get(1);
        let biomes_json: String = row.get(2);
        let water_levels_json: String = row.get(3);
        let water_types_json: String = row.get(4);
        let original_elevations_json: String = row.get(5);

        data.push((
            x,
            y,
            json_array_to_u32_vec(&serde_json::from_str::<Value>(&biomes_json)?),
            json_array_to_u32_vec(&serde_json::from_str::<Value>(&water_levels_json)?),
            json_array_to_u32_vec(&serde_json::from_str::<Value>(&water_types_json)?),
            json_array_to_u32_vec(&serde_json::from_str::<Value>(&original_elevations_json)?),
        ));
    }

    if data.is_empty() {
        return Err(format!("No terrain chunks returned (dimension={dimension})").into());
    }

    let min_x = data.iter().map(|(x, _, _, _, _, _)| *x).min().unwrap_or(0);
    let max_y = data.iter().map(|(_, y, _, _, _, _)| *y).max().unwrap_or(0);
    let max_x = data.iter().map(|(x, _, _, _, _, _)| *x).max().unwrap_or(0);
    let min_y = data.iter().map(|(_, y, _, _, _, _)| *y).min().unwrap_or(0);

    let width = ((max_x - min_x + 1) as usize) * 32;
    let height = ((max_y - min_y + 1) as usize) * 32;
    let mut pixels = vec![0_u8; width * height * 4];

    for (x, y, biomes, water_levels, water_types, original_elevations) in data {
        for i in 0usize..1024 {
            let chunk_x = i % 32;
            let chunk_y = i / 32;

            let is_land = match (original_elevations.get(i), water_levels.get(i)) {
                (Some(&elev), Some(&water)) => elev >= water,
                _ => false,
            };

            let land_color = biome_color(*biomes.get(i).unwrap_or(&0));
            let elevation = *original_elevations.get(i).unwrap_or(&0);
            let brightened_land = brighten(land_color, elevation.saturating_sub(25).min(255) as u8);
            let color = if is_land {
                brightened_land
            } else {
                water_color(*water_types.get(i).unwrap_or(&0))
            };

            let x_pos = ((x - min_x) as usize) * 32 + chunk_x;
            let y_pos = ((max_y - y) as usize) * 32 + (31 - chunk_y);
            set_pixel_rgba(&mut pixels, width, x_pos, y_pos, color);
        }
    }

    Ok(TerrainRaster {
        width,
        height,
        pixels,
        min_chunk_x: min_x,
        max_chunk_y: max_y,
    })
}

fn build_markers(
    dimension: i32,
    raster: &TerrainRaster,
    resource_id_filter: Option<i32>,
) -> Result<Vec<Marker>, Box<dyn Error>> {
    let mut client = Client::connect(&get_database_url(), NoTls)?;
    let mut markers = Vec::new();

    for row in client.query("SELECT name FROM claim_state WHERE name ILIKE '%cave%'", &[])? {
        let name: String = row.get(0);
        if let Some(cave) = parse_cave_claim(&name) {
            let (x, y) = world_to_map(cave.east, cave.north, raster);
            let icon_key = if let Some(resource) = cave_resource_in_name(&cave.name) {
                format!("{}_cave", resource)
            } else {
                "cave".to_string()
            };
            markers.push(Marker {
                name: cave.name,
                marker_type: "cave".to_string(),
                icon_key,
                x,
                y,
            });
        }
    }

    for row in client.query("SELECT name FROM claim_state WHERE name ILIKE '%watchtower%'", &[])? {
        let name: String = row.get(0);
        if let Some(watchtower) = parse_watchtower_claim(&name) {
            let (x, y) = world_to_map(watchtower.east, watchtower.north, raster);
            markers.push(Marker {
                name: watchtower.name,
                marker_type: "watchtower".to_string(),
                icon_key: "watchtower".to_string(),
                x,
                y,
            });
        }
    }

    for row in client.query("SELECT name FROM claim_state WHERE name ILIKE '%temple%'", &[])? {
        let name: String = row.get(0);
        if let Some(temple) = parse_temple_claim(&name) {
            let (x, y) = world_to_map(temple.east, temple.north, raster);
            markers.push(Marker {
                name: temple.name,
                marker_type: "temple".to_string(),
                icon_key: "temple".to_string(),
                x,
                y,
            });
        }
    }

    for row in client.query("SELECT name FROM claim_state WHERE name ILIKE '%dungeon%'", &[])? {
        let name: String = row.get(0);
        if let Some(dungeon) = parse_dungeon_claim(&name) {
            let (x, y) = world_to_map(dungeon.east, dungeon.north, raster);
            markers.push(Marker {
                name: dungeon.name,
                marker_type: "dungeon".to_string(),
                icon_key: "dungeon".to_string(),
                x,
                y,
            });
        }
    }

    if let Some(resource_id) = resource_id_filter {
        let dimension_i64 = dimension as i64;
        for row in client.query(
            r#"SELECT rs.entity_id, ls.x, ls.z
FROM resource_state rs
INNER JOIN location_state ls ON ls.entity_id = rs.entity_id
WHERE rs.resource_id = $1
  AND ls.dimension = $2"#,
            &[&resource_id, &dimension_i64],
        )? {
            let entity_id: i64 = row.get(0);
            let east_i64: i64 = row.get(1);
            let north_i64: i64 = row.get(2);

            if east_i64 < i32::MIN as i64 || east_i64 > i32::MAX as i64 {
                continue;
            }
            if north_i64 < i32::MIN as i64 || north_i64 > i32::MAX as i64 {
                continue;
            }

            let east = (east_i64 / 3) as i32;
            let north = (north_i64 / 3) as i32;

            let (x, y) = world_to_map(east, north, raster);
            markers.push(Marker {
                name: format!("Resource {} Entity {}", resource_id, entity_id),
                marker_type: "resource".to_string(),
                icon_key: "resource".to_string(),
                x,
                y,
            });
        }
    }

    let _ = dimension;
    Ok(markers)
}

fn encode_png(raster: &TerrainRaster) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    image::ImageEncoder::write_image(
        encoder,
        &raster.pixels,
        raster.width as u32,
        raster.height as u32,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(bytes)
}

fn encode_jpeg(raster: &TerrainRaster, quality: u8) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut rgb = Vec::with_capacity((raster.width * raster.height) * 3);
    for pixel in raster.pixels.chunks_exact(4) {
        rgb.push(pixel[0]);
        rgb.push(pixel[1]);
        rgb.push(pixel[2]);
    }

    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
    encoder.encode(
        &rgb,
        raster.width as u32,
        raster.height as u32,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(bytes)
}

fn encode_webp(raster: &TerrainRaster) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut bytes);
    encoder.encode(
        &raster.pixels,
        raster.width as u32,
        raster.height as u32,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(bytes)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn map_png(
    State(state): State<AppState>,
    Query(query): Query<TerrainQuery>,
) -> Response {
    let dimension = query.dimension.unwrap_or(1);
    let map_format = parse_map_format(query.format.as_deref());
    let jpeg_quality = clamp_jpeg_quality(query.quality);
    let cache_key = format!(
        "dim:{dimension}:fmt:{}:q:{jpeg_quality}",
        if map_format == MapFormat::Webp {
            "webp"
        } else {
            "png"
        }
    );
    let content_type = if map_format == MapFormat::Webp {
        "image/webp"
    } else {
        "image/png"
    };

    {
        let cache = state.map_cache.read().await;
        if let Some(cached) = cache.get(&cache_key) {
            if cached.created_at.elapsed() < state.map_cache_ttl {
                let mut response = Response::new(cached.png.clone().into());
                response
                    .headers_mut()
                    .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
                );
                response
                    .headers_mut()
                    .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
                response
                    .headers_mut()
                    .insert(header::EXPIRES, HeaderValue::from_static("0"));
                response
                    .headers_mut()
                    .insert("x-map-cache", HeaderValue::from_static("HIT"));
                return response;
            }
        }
    }

    match tokio::task::spawn_blocking(move || {
        let raster = build_raster(dimension).map_err(|error| error.to_string())?;
        let encoded = if map_format == MapFormat::Webp {
            let _ = jpeg_quality;
            encode_webp(&raster).map_err(|error| error.to_string())?
        } else {
            encode_png(&raster).map_err(|error| error.to_string())?
        };
        Ok::<Vec<u8>, String>(encoded)
    })
    .await
    {
        Ok(Ok(bytes)) => {
            {
                let mut cache = state.map_cache.write().await;
                cache.insert(
                    cache_key,
                    CachedMap {
                        png: bytes.clone(),
                        created_at: Instant::now(),
                    },
                );
            }

            let mut response = Response::new(bytes.into());
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
            );
            response
                .headers_mut()
                .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
            response
                .headers_mut()
                .insert(header::EXPIRES, HeaderValue::from_static("0"));
            response
                .headers_mut()
                .insert("x-map-cache", HeaderValue::from_static("MISS"));
            response
        }
        Ok(Err(error)) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to build map image: {error}"),
        )
            .into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {error}"),
        )
            .into_response(),
    }
}

async fn icons(
    State(state): State<AppState>,
    Query(query): Query<TerrainQuery>,
) -> Response {
    let dimension = query.dimension.unwrap_or(1);
    let resource_id = query.resource_id;
    let cache_key = format!(
        "dim:{dimension}:resource:{}",
        resource_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    );

    {
        let cache = state.icons_cache.read().await;
        if let Some(cached) = cache.get(&cache_key) {
            if cached.created_at.elapsed() < state.icons_cache_ttl {
                let mut response = Response::new(cached.json.clone().into());
                response
                    .headers_mut()
                    .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
                );
                response
                    .headers_mut()
                    .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
                response
                    .headers_mut()
                    .insert(header::EXPIRES, HeaderValue::from_static("0"));
                response
                    .headers_mut()
                    .insert("x-icons-cache", HeaderValue::from_static("HIT"));
                return response;
            }
        }
    }

    match tokio::task::spawn_blocking(move || {
        let raster = build_raster(dimension).map_err(|e| e.to_string())?;
        let markers = build_markers(dimension, &raster, resource_id).map_err(|e| e.to_string())?;
        Ok::<MarkerResponse, String>(MarkerResponse {
            width: raster.width,
            height: raster.height,
            markers,
        })
    })
    .await
    {
        Ok(Ok(payload)) => {
            let body = match serde_json::to_vec(&payload) {
                Ok(body) => body,
                Err(error) => {
                    return (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to serialize icons payload: {error}"),
                    )
                        .into_response();
                }
            };

            {
                let mut cache = state.icons_cache.write().await;
                cache.insert(
                    cache_key,
                    CachedIcons {
                        json: body.clone(),
                        created_at: Instant::now(),
                    },
                );
            }

            let mut response = Response::new(body.into());
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, no-cache, must-revalidate, max-age=0"),
            );
            response
                .headers_mut()
                .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
            response
                .headers_mut()
                .insert(header::EXPIRES, HeaderValue::from_static("0"));
            response
                .headers_mut()
                .insert("x-icons-cache", HeaderValue::from_static("MISS"));
            response
        }
        Ok(Err(error)) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to build icons payload: {error}"),
        )
            .into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Task join error: {error}"),
        )
            .into_response(),
    }
}

async fn icon_file(Path(file_name): Path<String>) -> Response {
    if file_name.contains("..") || file_name.contains('/') || file_name.contains('\\') {
        return (axum::http::StatusCode::BAD_REQUEST, "invalid icon path").into_response();
    }

    let path = format!("icons/{file_name}");
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let content_type = if file_name.ends_with(".webp") {
                "image/webp"
            } else {
                "image/png"
            };
            let mut response = Response::new(bytes.into());
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_str(content_type)
                    .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
            );
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            );
            response
        }
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "icon not found").into_response(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let state = AppState {
        map_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        map_cache_ttl: get_map_cache_ttl(),
        icons_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        icons_cache_ttl: get_icons_cache_ttl(),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/api/map.png", get(map_png))
        .route("/api/icons", get(icons))
        .route("/icon/{file}", get(icon_file))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("terrain_web listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang='en'>
<head>
  <meta charset='UTF-8' />
  <meta name='viewport' content='width=device-width, initial-scale=1.0' />
  <title>BitCraft Terrain Web Viewer</title>
  <style>
    html, body { margin: 0; width: 100%; height: 100%; background: #10131a; color: #e5e7eb; font-family: Arial, sans-serif; }
    #app { display: grid; grid-template-columns: 320px 1fr; height: 100%; }
    #sidebar { border-right: 1px solid #2b3340; padding: 12px; overflow: auto; }
    #canvasWrap { position: relative; overflow: hidden; }
    #mapCanvas { width: 100%; height: 100%; display: block; cursor: default; }
    .row { margin-bottom: 8px; }
    .small { color: #93a2b7; font-size: 12px; }
    input, button { background: #1a2230; color: #e5e7eb; border: 1px solid #364256; padding: 6px; }
    button { cursor: pointer; }
  </style>
</head>
<body>
<div id='app'>
  <aside id='sidebar'>
    <h3 style='margin-top:0'>Terrain Web Viewer</h3>
    <div class='row'>
      <label>Dimension: <input id='dimension' type='number' value='1' style='width:90px' /></label>
      <button id='reload'>Reload</button>
    </div>
        <div class='row'>
            <label>Resource ID: <input id='resourceId' type='number' placeholder='e.g. 12' style='width:90px' /></label>
            <button id='loadResource'>Load Resources</button>
        </div>
    <div class='row small'>Wheel: zoom · Right-drag: pan · Left-click marker: name</div>
    <div class='row' id='status'>Loading…</div>
  </aside>
  <main id='canvasWrap'><canvas id='mapCanvas'></canvas></main>
</div>
<script>
const canvas = document.getElementById('mapCanvas');
const ctx = canvas.getContext('2d');
const statusEl = document.getElementById('status');
const dimInput = document.getElementById('dimension');
const reloadBtn = document.getElementById('reload');
const resourceIdInput = document.getElementById('resourceId');
const loadResourceBtn = document.getElementById('loadResource');

let mapImage = new Image();
let markers = [];
const loadedIcons = {};
let mapW = 0;
let mapH = 0;
let zoom = 1;
let panX = 0;
let panY = 0;
let rightDown = false;
let lastX = 0;
let lastY = 0;

function markerColor(type) {
  if (type === 'cave') return '#f59e0b';
  if (type === 'watchtower') return '#22c55e';
  if (type === 'temple') return '#60a5fa';
  if (type === 'dungeon') return '#a78bfa';
    if (type === 'resource') return '#10b981';
  return '#ef4444';
}

async function loadMarkerIcons() {
    const iconCandidates = {
        cave: ['cave.png', 'cave.webp'],
        watchtower: ['watchtower.png', 'watchtower.webp'],
        temple: ['temple.png', 'temple.webp'],
        dungeon: ['dungeon.png', 'dungeon.webp'],
        emarium_cave: ['emarium_cave.png', 'emarium_cave.webp'],
        pyrelite_cave: ['pyrelite_cave.png', 'pyrelite_cave.webp'],
        rathium_cave: ['rathium_cave.png', 'rathium_cave.webp'],
        ferralith_cave: ['ferralith_cave.png', 'ferralith_cave.webp'],
        celestium_cave: ['celestium_cave.png', 'celestium_cave.webp'],
        aurumite_cave: ['aurumite_cave.png', 'aurumite_cave.webp'],
        luminite_cave: ['luminite_cave.png', 'luminite_cave.webp'],
        elenvar_cave: ['elenvar_cave.png', 'elenvar_cave.webp'],
        resource: ['resource.png', 'resource.webp'],
    };

    for (const [key, candidates] of Object.entries(iconCandidates)) {
        let loaded = false;
        for (const fileName of candidates) {
            try {
                const url = `/icon/${encodeURIComponent(fileName)}`;
                const response = await fetch(url);
                if (!response.ok) continue;

                const blob = await response.blob();
                const imageUrl = URL.createObjectURL(blob);
                const image = new Image();
                await new Promise((resolve, reject) => {
                    image.onload = resolve;
                    image.onerror = reject;
                    image.src = imageUrl;
                });

                loadedIcons[key] = image;
                loaded = true;
                break;
            } catch (_) {}
        }

        if (!loaded) {
            loadedIcons[key] = null;
        }
    }
}

function resizeCanvas() {
  const rect = canvas.parentElement.getBoundingClientRect();
  canvas.width = Math.max(1, Math.floor(rect.width));
  canvas.height = Math.max(1, Math.floor(rect.height));
  draw();
}

function draw() {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (!mapImage.complete || !mapW || !mapH) return;

  const drawW = mapW * zoom;
  const drawH = mapH * zoom;
  ctx.drawImage(mapImage, panX, panY, drawW, drawH);

  for (const marker of markers) {
    const x = panX + marker.x * zoom;
    const y = panY + marker.y * zoom;
    if (x < -12 || y < -12 || x > canvas.width + 12 || y > canvas.height + 12) continue;

        const iconKey = marker.icon_key || marker.marker_type;
        const icon = loadedIcons[iconKey] || loadedIcons[marker.marker_type];
        const iconSide = Math.max(10, Math.min(48, 22 * zoom));

        if (icon) {
            ctx.drawImage(icon, x - iconSide / 2, y - iconSide / 2, iconSide, iconSide);
        } else {
            ctx.beginPath();
            ctx.arc(x, y, Math.max(4, 5 * zoom), 0, Math.PI * 2);
            ctx.fillStyle = markerColor(marker.marker_type);
            ctx.fill();
            ctx.strokeStyle = '#111827';
            ctx.lineWidth = 1;
            ctx.stroke();
        }
  }
}

function pickMarker(px, py) {
  for (let i = markers.length - 1; i >= 0; i--) {
    const marker = markers[i];
    const x = panX + marker.x * zoom;
    const y = panY + marker.y * zoom;
    const r = Math.max(4, 5 * zoom) + 2;
    const dx = px - x;
    const dy = py - y;
    if (dx * dx + dy * dy <= r * r) return marker;
  }
  return null;
}

async function loadData() {
  const dimension = parseInt(dimInput.value || '1', 10) || 1;
    const resourceIdValue = (resourceIdInput.value || '').trim();
    const parsedResourceId = resourceIdValue === '' ? null : parseInt(resourceIdValue, 10);
    const useResourceId = Number.isInteger(parsedResourceId) ? parsedResourceId : null;
  statusEl.textContent = 'Loading map and markers…';
    const cacheBust = Date.now();

    const mapUrl = `/api/map.png?dimension=${dimension}&format=webp&t=${cacheBust}`;
        const markerUrl = useResourceId === null
            ? `/api/icons?dimension=${dimension}&t=${cacheBust}`
            : `/api/icons?dimension=${dimension}&resource_id=${useResourceId}&t=${cacheBust}`;

  const markerRes = await fetch(markerUrl);
  if (!markerRes.ok) {
    statusEl.textContent = await markerRes.text();
    return;
  }
  const markerData = await markerRes.json();
  markers = markerData.markers || [];
  mapW = markerData.width || 0;
  mapH = markerData.height || 0;

    await loadMarkerIcons();

  await new Promise((resolve, reject) => {
    mapImage = new Image();
    mapImage.onload = resolve;
    mapImage.onerror = reject;
    mapImage.src = mapUrl;
  });

    mapW = mapImage.naturalWidth || mapW;
    mapH = mapImage.naturalHeight || mapH;

  zoom = 1;
  panX = (canvas.width - mapW) / 2;
  panY = (canvas.height - mapH) / 2;

        statusEl.textContent = useResourceId === null
            ? `Loaded ${markers.length} markers (${mapW}x${mapH}).`
            : `Loaded ${markers.length} markers for resource_id=${useResourceId} (${mapW}x${mapH}).`;
  draw();
}

canvas.addEventListener('contextmenu', (event) => event.preventDefault());
canvas.addEventListener('mousedown', (event) => {
  if (event.button === 2) {
    rightDown = true;
    lastX = event.clientX;
    lastY = event.clientY;
  }
});
window.addEventListener('mouseup', () => {
  rightDown = false;
});
window.addEventListener('mousemove', (event) => {
  if (!rightDown) return;
  panX += event.clientX - lastX;
  panY += event.clientY - lastY;
  lastX = event.clientX;
  lastY = event.clientY;
  draw();
});

canvas.addEventListener('wheel', (event) => {
  event.preventDefault();
  const zoomFactor = Math.exp(-event.deltaY * 0.001);
  const oldZoom = zoom;
  const newZoom = Math.min(6, Math.max(0.2, oldZoom * zoomFactor));
  if (newZoom === oldZoom) return;

  const rect = canvas.getBoundingClientRect();
  const mx = event.clientX - rect.left;
  const my = event.clientY - rect.top;

  const worldX = (mx - panX) / oldZoom;
  const worldY = (my - panY) / oldZoom;
  zoom = newZoom;
  panX = mx - worldX * zoom;
  panY = my - worldY * zoom;
  draw();
}, { passive: false });

canvas.addEventListener('click', (event) => {
  if (event.button !== 0) return;
  const rect = canvas.getBoundingClientRect();
  const marker = pickMarker(event.clientX - rect.left, event.clientY - rect.top);
  if (marker) {
    statusEl.textContent = `Selected: ${marker.name} (${Math.round(marker.x)}, ${Math.round(marker.y)})`;
  }
});

reloadBtn.addEventListener('click', () => { loadData().catch((e) => { statusEl.textContent = e.message; }); });
loadResourceBtn.addEventListener('click', () => { loadData().catch((e) => { statusEl.textContent = e.message; }); });
window.addEventListener('resize', resizeCanvas);
resizeCanvas();
loadData().catch((e) => { statusEl.textContent = e.message; });
</script>
</body>
</html>
"#;
