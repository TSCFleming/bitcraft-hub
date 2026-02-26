use eframe::egui::{self, ColorImage, PointerButton, Pos2, Rect, Sense, TextureHandle, TextureOptions, Vec2};
use postgres::{Client, NoTls};
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;

struct TerrainRaster {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    min_chunk_x: i32,
    max_chunk_y: i32,
}

struct MapTile {
    texture: TextureHandle,
    position: Pos2,
    size: Vec2,
}

struct CaveClaim {
    name: String,
    north: i32,
    east: i32,
}

struct WatchtowerClaim {
    name: String,
    north: i32,
    east: i32,
}

struct TempleClaim {
    name: String,
    north: i32,
    east: i32,
}

struct DungeonClaim {
    name: String,
    north: i32,
    east: i32,
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
    if parts.len() < 4 {
        return None;
    }

    if !parts[1].to_lowercase().contains("watchtower") {
        return None;
    }

    let north = parts[2].trim().parse::<i32>().ok()?;
    let east = parts[3].trim().parse::<i32>().ok()?;

    Some(WatchtowerClaim {
        name: parts[0].trim().to_string(),
        north,
        east,
    })
}

fn parse_temple_claim(name: &str) -> Option<TempleClaim> {
    let parts = name.split("|~").collect::<Vec<_>>();
    if parts.len() < 4 {
        return None;
    }

    if !parts[1].to_lowercase().contains("temple") {
        return None;
    }

    let north = parts[2].trim().parse::<i32>().ok()?;
    let east = parts[3].trim().parse::<i32>().ok()?;

    Some(TempleClaim {
        name: parts[0].trim().to_string(),
        north,
        east,
    })
}

fn parse_dungeon_claim(name: &str) -> Option<DungeonClaim> {
    let parts = name.split("|~").collect::<Vec<_>>();
    if parts.len() < 4 {
        return None;
    }

    if !parts[1].to_lowercase().contains("dungeon") {
        return None;
    }

    let north = parts[2].trim().parse::<i32>().ok()?;
    let east = parts[3].trim().parse::<i32>().ok()?;

    Some(DungeonClaim {
        name: parts[0].trim().to_string(),
        north,
        east,
    })
}

fn query_cave_claims(client: &mut Client) -> Result<Vec<CaveClaim>, Box<dyn Error>> {
    let rows = client.query(
        "SELECT name FROM claim_state WHERE name ILIKE '%cave%'",
        &[],
    )?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name: String = row.get(0);
            parse_cave_claim(&name)
        })
        .collect())
}

fn query_watchtower_claims(client: &mut Client) -> Result<Vec<WatchtowerClaim>, Box<dyn Error>> {
    let rows = client.query(
        "SELECT name FROM claim_state WHERE name ILIKE '%watchtower%'",
        &[],
    )?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name: String = row.get(0);
            parse_watchtower_claim(&name)
        })
        .collect())
}

fn query_temple_claims(client: &mut Client) -> Result<Vec<TempleClaim>, Box<dyn Error>> {
    let rows = client.query(
        "SELECT name FROM claim_state WHERE name ILIKE '%temple%'",
        &[],
    )?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name: String = row.get(0);
            parse_temple_claim(&name)
        })
        .collect())
}

fn query_dungeon_claims(client: &mut Client) -> Result<Vec<DungeonClaim>, Box<dyn Error>> {
    let rows = client.query(
        "SELECT name FROM claim_state WHERE name ILIKE '%dungeon%'",
        &[],
    )?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let name: String = row.get(0);
            parse_dungeon_claim(&name)
        })
        .collect())
}


fn world_to_map_position(cave: &CaveClaim, raster: &TerrainRaster) -> Pos2 {
    let map_x = cave.east as f32 - (raster.min_chunk_x as f32 * 32.0);
    let max_world_y = raster.max_chunk_y as f32 * 32.0 + 31.0;
    let map_y = max_world_y - cave.north as f32;

    Pos2::new(
        map_x.clamp(0.0, raster.width as f32),
        map_y.clamp(0.0, raster.height as f32),
    )
}

fn world_to_map_position_watchtower(watchtower: &WatchtowerClaim, raster: &TerrainRaster) -> Pos2 {
    let map_x = watchtower.east as f32 - (raster.min_chunk_x as f32 * 32.0);
    let max_world_y = raster.max_chunk_y as f32 * 32.0 + 31.0;
    let map_y = max_world_y - watchtower.north as f32;

    Pos2::new(
        map_x.clamp(0.0, raster.width as f32),
        map_y.clamp(0.0, raster.height as f32),
    )
}

fn world_to_map_position_temple(temple: &TempleClaim, raster: &TerrainRaster) -> Pos2 {
    let map_x = temple.east as f32 - (raster.min_chunk_x as f32 * 32.0);
    let max_world_y = raster.max_chunk_y as f32 * 32.0 + 31.0;
    let map_y = max_world_y - temple.north as f32;

    Pos2::new(
        map_x.clamp(0.0, raster.width as f32),
        map_y.clamp(0.0, raster.height as f32),
    )
}

fn world_to_map_position_dungeon(dungeon: &DungeonClaim, raster: &TerrainRaster) -> Pos2 {
    let map_x = dungeon.east as f32 - (raster.min_chunk_x as f32 * 32.0);
    let max_world_y = raster.max_chunk_y as f32 * 32.0 + 31.0;
    let map_y = max_world_y - dungeon.north as f32;

    Pos2::new(
        map_x.clamp(0.0, raster.width as f32),
        map_y.clamp(0.0, raster.height as f32),
    )
}

fn create_cave_icon_texture(ctx: &egui::Context) -> TextureHandle {
    let side = 32usize;
    let mut pixels = vec![0_u8; side * side * 4];
    let center = (side as f32 - 1.0) * 0.5;
    let radius = side as f32 * 0.42;
    let border_radius = side as f32 * 0.48;

    for y in 0..side {
        for x in 0..side {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let index = (y * side + x) * 4;

            if distance <= border_radius {
                pixels[index] = 210;
                pixels[index + 1] = 170;
                pixels[index + 2] = 70;
                pixels[index + 3] = 255;
            }

            if distance <= radius {
                pixels[index] = 30;
                pixels[index + 1] = 30;
                pixels[index + 2] = 30;
                pixels[index + 3] = 255;
            }
        }
    }

    let image = ColorImage::from_rgba_unmultiplied([side, side], &pixels);
    ctx.load_texture("builtin-cave-icon", image, TextureOptions::LINEAR)
}

fn create_watchtower_icon_texture(ctx: &egui::Context) -> TextureHandle {
    let side = 32usize;
    let mut pixels = vec![0_u8; side * side * 4];

    for y in 0..side {
        for x in 0..side {
            let index = (y * side + x) * 4;
            let center_x = (side / 2) as i32;
            let center_y = (side / 2) as i32;
            let dx = x as i32 - center_x;
            let dy = y as i32 - center_y;

            let in_circle = dx * dx + dy * dy <= 13 * 13;
            let tower_body = x >= 12 && x <= 19 && y >= 11 && y <= 25;
            let tower_top = y >= 7 && y <= 12 && x >= 9 && x <= 22;

            if in_circle {
                pixels[index] = 195;
                pixels[index + 1] = 165;
                pixels[index + 2] = 90;
                pixels[index + 3] = 255;
            }

            if tower_body || tower_top {
                pixels[index] = 40;
                pixels[index + 1] = 40;
                pixels[index + 2] = 40;
                pixels[index + 3] = 255;
            }
        }
    }

    let image = ColorImage::from_rgba_unmultiplied([side, side], &pixels);
    ctx.load_texture("builtin-watchtower-icon", image, TextureOptions::LINEAR)
}

fn create_temple_icon_texture(ctx: &egui::Context) -> TextureHandle {
    let side = 32usize;
    let mut pixels = vec![0_u8; side * side * 4];

    for y in 0..side {
        for x in 0..side {
            let index = (y * side + x) * 4;
            let base = y >= 15 && y <= 26 && x >= 8 && x <= 23;
            let roof = y >= 9 && y <= 15 && x + y >= 21 && x + y <= 41;
            let door = y >= 20 && y <= 26 && x >= 14 && x <= 17;

            if base || roof {
                pixels[index] = 200;
                pixels[index + 1] = 175;
                pixels[index + 2] = 95;
                pixels[index + 3] = 255;
            }

            if door {
                pixels[index] = 55;
                pixels[index + 1] = 40;
                pixels[index + 2] = 30;
                pixels[index + 3] = 255;
            }
        }
    }

    let image = ColorImage::from_rgba_unmultiplied([side, side], &pixels);
    ctx.load_texture("builtin-temple-icon", image, TextureOptions::LINEAR)
}

fn create_dungeon_icon_texture(ctx: &egui::Context) -> TextureHandle {
    let side = 32usize;
    let mut pixels = vec![0_u8; side * side * 4];

    for y in 0..side {
        for x in 0..side {
            let index = (y * side + x) * 4;
            let ring = x >= 8 && x <= 23 && y >= 8 && y <= 23;
            let hollow = x >= 11 && x <= 20 && y >= 11 && y <= 20;
            let gate = x >= 14 && x <= 17 && y >= 18 && y <= 23;

            if ring {
                pixels[index] = 120;
                pixels[index + 1] = 120;
                pixels[index + 2] = 130;
                pixels[index + 3] = 255;
            }

            if hollow || gate {
                pixels[index] = 25;
                pixels[index + 1] = 25;
                pixels[index + 2] = 30;
                pixels[index + 3] = 255;
            }
        }
    }

    let image = ColorImage::from_rgba_unmultiplied([side, side], &pixels);
    ctx.load_texture("builtin-dungeon-icon", image, TextureOptions::LINEAR)
}

fn build_terrain_raster(
) -> Result<
    (
        TerrainRaster,
        Vec<CaveClaim>,
        Vec<WatchtowerClaim>,
        Vec<TempleClaim>,
        Vec<DungeonClaim>,
    ),
    Box<dyn Error>,
> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bitcraft:bitcraft_pw@localhost:5431/bitcraft_hub".to_string());

    let mut client = Client::connect(&database_url, NoTls)?;

    let dimension: i32 = std::env::var("DIMENSION")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);

    let rows = client.query(
        r#"SELECT chunk_x,
   chunk_z,
   "dimension",
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
        let biomes_json: String = row.get(3);
        let water_levels_json: String = row.get(4);
        let water_types_json: String = row.get(5);
        let original_elevations_json: String = row.get(6);

        let biomes: Value = serde_json::from_str(&biomes_json)?;
        let water_levels: Value = serde_json::from_str(&water_levels_json)?;
        let water_types: Value = serde_json::from_str(&water_types_json)?;
        let original_elevations: Value = serde_json::from_str(&original_elevations_json)?;

        data.push((
            x,
            y,
            json_array_to_u32_vec(&biomes),
            json_array_to_u32_vec(&water_levels),
            json_array_to_u32_vec(&water_types),
            json_array_to_u32_vec(&original_elevations),
        ));
    }

    if data.is_empty() {
        return Err(format!(
            "No terrain chunks returned from Postgres (dimension={dimension})"
        )
        .into());
    }

    let cave_claims = query_cave_claims(&mut client)?;
    let watchtower_claims = query_watchtower_claims(&mut client)?;
    let temple_claims = query_temple_claims(&mut client)?;
    let dungeon_claims = query_dungeon_claims(&mut client)?;

    let min_x = data.iter().map(|(x, _, _, _, _, _)| *x).min().unwrap_or(0);
    let min_y = data.iter().map(|(_, y, _, _, _, _)| *y).min().unwrap_or(0);
    let max_x = data.iter().map(|(x, _, _, _, _, _)| *x).max().unwrap_or(0);
    let max_y = data.iter().map(|(_, y, _, _, _, _)| *y).max().unwrap_or(0);

    let image_width = ((max_x - min_x + 1) as usize) * 32;
    let image_height = ((max_y - min_y + 1) as usize) * 32;

    let mut pixels = vec![0_u8; image_width * image_height * 4];

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
            let brighten_amount = elevation.saturating_sub(25).min(255) as u8;
            let brightened_land = brighten(land_color, brighten_amount);

            let color = if is_land {
                brightened_land
            } else {
                water_color(*water_types.get(i).unwrap_or(&0))
            };

            let x_pos = ((x - min_x) as usize) * 32 + chunk_x;
            let y_pos = ((max_y - y) as usize) * 32 + (31 - chunk_y);

            set_pixel_rgba(&mut pixels, image_width, x_pos, y_pos, color);
        }
    }

    Ok((
        TerrainRaster {
            width: image_width,
            height: image_height,
            pixels,
            min_chunk_x: min_x,
            max_chunk_y: max_y,
        },
        cave_claims,
        watchtower_claims,
        temple_claims,
        dungeon_claims,
    ))
}

fn create_map_tiles(ctx: &egui::Context, raster: &TerrainRaster, max_texture_side: usize) -> Vec<MapTile> {
    let tile_side = max_texture_side.max(1);
    let mut tiles = Vec::new();

    let mut tile_index = 0usize;
    for tile_y in (0..raster.height).step_by(tile_side) {
        for tile_x in (0..raster.width).step_by(tile_side) {
            let tile_width = (raster.width - tile_x).min(tile_side);
            let tile_height = (raster.height - tile_y).min(tile_side);

            let mut tile_pixels = vec![0_u8; tile_width * tile_height * 4];

            for row in 0..tile_height {
                let src_start = ((tile_y + row) * raster.width + tile_x) * 4;
                let src_end = src_start + tile_width * 4;
                let dst_start = row * tile_width * 4;
                let dst_end = dst_start + tile_width * 4;
                tile_pixels[dst_start..dst_end]
                    .copy_from_slice(&raster.pixels[src_start..src_end]);
            }

            let tile_image = ColorImage::from_rgba_unmultiplied([tile_width, tile_height], &tile_pixels);
            let texture = ctx.load_texture(
                format!("terrain-map-tile-{tile_index}"),
                tile_image,
                TextureOptions::NEAREST,
            );
            tile_index += 1;

            tiles.push(MapTile {
                texture,
                position: Pos2::new(tile_x as f32, tile_y as f32),
                size: Vec2::new(tile_width as f32, tile_height as f32),
            });
        }
    }

    tiles
}

fn load_texture_from_path(
    ctx: &egui::Context,
    path: &str,
    texture_name: &str,
) -> Result<TextureHandle, String> {
    let image = image::open(path)
        .map_err(|error| format!("Failed to open icon '{path}': {error}"))?
        .to_rgba8();

    let width = image.width() as usize;
    let height = image.height() as usize;

    if width == 0 || height == 0 {
        return Err("Icon image has zero width or height".to_string());
    }

    let color_image = ColorImage::from_rgba_unmultiplied([width, height], image.as_raw());
    Ok(ctx.load_texture(
        texture_name.to_string(),
        color_image,
        TextureOptions::LINEAR,
    ))
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

fn try_load_resource_cave_icon(ctx: &egui::Context, resource: &str) -> Option<TextureHandle> {
    let png = format!("icons/{}_cave.png", resource);
    let webp = format!("icons/{}_cave.webp", resource);
    let candidates = [png, webp];

    candidates.iter().find_map(|path| {
        load_texture_from_path(ctx, path, &format!("{resource}-cave-icon"))
            .ok()
    })
}

fn try_load_watchtower_icon(ctx: &egui::Context) -> Option<TextureHandle> {
    let candidates = ["icons/watchtower.png", "icons/watchtower.webp"];

    candidates.iter().find_map(|path| {
        load_texture_from_path(ctx, path, "watchtower-icon")
            .ok()
    })
}

fn try_load_temple_icon(ctx: &egui::Context) -> Option<TextureHandle> {
    let candidates = ["icons/temple.png", "icons/temple.webp"];

    candidates.iter().find_map(|path| {
        load_texture_from_path(ctx, path, "temple-icon")
            .ok()
    })
}

fn try_load_dungeon_icon(ctx: &egui::Context) -> Option<TextureHandle> {
    let candidates = ["icons/dungeon.png", "icons/dungeon.webp"];

    candidates.iter().find_map(|path| {
        load_texture_from_path(ctx, path, "dungeon-icon")
            .ok()
    })
}

fn load_special_cave_icons(
    ctx: &egui::Context,
) -> (HashMap<String, TextureHandle>, Vec<String>) {
    let mut loaded = HashMap::new();
    let mut missing = Vec::new();

    for resource in SPECIAL_CAVE_RESOURCES {
        if let Some(texture) = try_load_resource_cave_icon(ctx, resource) {
            loaded.insert(resource.to_string(), texture);
        } else {
            missing.push(resource.to_string());
        }
    }

    (loaded, missing)
}

fn cave_resource_in_name(name: &str) -> Option<&'static str> {
    let lowered = name.to_lowercase();
    SPECIAL_CAVE_RESOURCES
        .iter()
        .copied()
        .find(|resource| lowered.contains(resource))
}

fn load_icon_texture(ctx: &egui::Context, path: &str, id: usize) -> Result<TextureHandle, String> {
    load_texture_from_path(ctx, path, &format!("map-icon-{id}"))
}

#[derive(Clone)]
struct MapIcon {
    name: String,
    path: String,
    position: Pos2,
    size: f32,
    visible: bool,
    texture: TextureHandle,
}

struct TerrainViewerApp {
    map_tiles: Vec<MapTile>,
    map_size: Vec2,
    zoom: f32,
    pan_offset: Vec2,
    icons: Vec<MapIcon>,
    selected_icon: Option<usize>,
    new_icon_path: String,
    status: String,
}

impl TerrainViewerApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        terrain_raster: TerrainRaster,
        cave_claims: Vec<CaveClaim>,
        watchtower_claims: Vec<WatchtowerClaim>,
        temple_claims: Vec<TempleClaim>,
        dungeon_claims: Vec<DungeonClaim>,
    ) -> Self {
        let max_texture_side = cc.egui_ctx.input(|input| input.max_texture_side);
        let map_tiles = create_map_tiles(&cc.egui_ctx, &terrain_raster, max_texture_side);
        let cave_texture = create_cave_icon_texture(&cc.egui_ctx);
        let watchtower_texture = create_watchtower_icon_texture(&cc.egui_ctx);
        let temple_texture = create_temple_icon_texture(&cc.egui_ctx);
        let dungeon_texture = create_dungeon_icon_texture(&cc.egui_ctx);
        let watchtower_texture_override = try_load_watchtower_icon(&cc.egui_ctx);
        let temple_texture_override = try_load_temple_icon(&cc.egui_ctx);
        let dungeon_texture_override = try_load_dungeon_icon(&cc.egui_ctx);
        let (special_icons, missing_special_icons) = load_special_cave_icons(&cc.egui_ctx);

        let cave_icons = cave_claims
            .iter()
            .map(|cave| MapIcon {
                name: cave.name.clone(),
                path: if let Some(resource) = cave_resource_in_name(&cave.name) {
                    format!("claim_state:cave:{resource}")
                } else {
                    "claim_state:cave".to_string()
                },
                position: world_to_map_position(cave, &terrain_raster),
                size: 22.0,
                visible: true,
                texture: if let Some(resource) = cave_resource_in_name(&cave.name) {
                    special_icons
                        .get(resource)
                        .cloned()
                        .unwrap_or_else(|| cave_texture.clone())
                } else {
                    cave_texture.clone()
                },
            })
            .collect::<Vec<_>>();

        let cave_count = cave_icons.len();

        let watchtower_icons = watchtower_claims
            .iter()
            .map(|watchtower| MapIcon {
                name: format!("{} (Watchtower)", watchtower.name),
                path: "claim_state:watchtower".to_string(),
                position: world_to_map_position_watchtower(watchtower, &terrain_raster),
                size: 20.0,
                visible: true,
                texture: watchtower_texture_override
                    .clone()
                    .unwrap_or_else(|| watchtower_texture.clone()),
            })
            .collect::<Vec<_>>();

        let watchtower_count = watchtower_icons.len();

        let temple_icons = temple_claims
            .iter()
            .map(|temple| MapIcon {
                name: format!("{} (Temple)", temple.name),
                path: "claim_state:temple".to_string(),
                position: world_to_map_position_temple(temple, &terrain_raster),
                size: 22.0,
                visible: true,
                texture: temple_texture_override
                    .clone()
                    .unwrap_or_else(|| temple_texture.clone()),
            })
            .collect::<Vec<_>>();

        let temple_count = temple_icons.len();

        let dungeon_icons = dungeon_claims
            .iter()
            .map(|dungeon| MapIcon {
                name: format!("{} (Dungeon)", dungeon.name),
                path: "claim_state:dungeon".to_string(),
                position: world_to_map_position_dungeon(dungeon, &terrain_raster),
                size: 22.0,
                visible: true,
                texture: dungeon_texture_override
                    .clone()
                    .unwrap_or_else(|| dungeon_texture.clone()),
            })
            .collect::<Vec<_>>();

        let dungeon_count = dungeon_icons.len();

        let mut icons = cave_icons;
        icons.extend(watchtower_icons);
        icons.extend(temple_icons);
        icons.extend(dungeon_icons);

        let selected_icon = if icons.is_empty() { None } else { Some(0) };
        let special_status = if missing_special_icons.is_empty() {
            "Special cave icons: all loaded".to_string()
        } else {
            format!(
                "Missing special cave icons for: {} (use icons/<name>_cave.png or .webp)",
                missing_special_icons.join(", ")
            )
        };

        let watchtower_status = if watchtower_texture_override.is_some() {
            "Watchtower icon: loaded"
        } else {
            "Watchtower icon: not found (use icons/watchtower.png or .webp)"
        };

        let temple_status = if temple_texture_override.is_some() {
            "Temple icon: loaded"
        } else {
            "Temple icon: not found (use icons/temple.png or .webp)"
        };

        let dungeon_status = if dungeon_texture_override.is_some() {
            "Dungeon icon: loaded"
        } else {
            "Dungeon icon: not found (use icons/dungeon.png or .webp)"
        };

        let status = format!(
            "Loaded {cave_count} cave icons, {watchtower_count} watchtowers, {temple_count} temples, and {dungeon_count} dungeons from claim_state. {special_status}. {watchtower_status}. {temple_status}. {dungeon_status}. Click an icon on the map to see its name"
        );

        Self {
            map_tiles,
            map_size: Vec2::new(terrain_raster.width as f32, terrain_raster.height as f32),
            zoom: 1.0,
            pan_offset: Vec2::ZERO,
            icons,
            selected_icon,
            new_icon_path: String::new(),
            status,
        }
    }

    fn add_icon(&mut self, ctx: &egui::Context) {
        let path = self.new_icon_path.trim().to_string();
        if path.is_empty() {
            self.status = "Enter an icon file path first".to_string();
            return;
        }

        let icon_id = self.icons.len();
        match load_icon_texture(ctx, &path, icon_id) {
            Ok(texture) => {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("icon")
                    .to_string();

                self.icons.push(MapIcon {
                    name,
                    path,
                    position: Pos2::new(self.map_size.x * 0.5, self.map_size.y * 0.5),
                    size: 32.0,
                    visible: true,
                    texture,
                });
                self.selected_icon = Some(icon_id);
                self.status = "Icon added at map center. Click an icon on the map to see its name".to_string();
            }
            Err(error) => {
                self.status = error;
            }
        }
    }
}

impl eframe::App for TerrainViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::left("controls")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading("Terrain Map Viewer");
                ui.label("Load icons and resize them. Scroll wheel zooms the map. Click an icon on the map to see its name.");
                ui.separator();

                ui.add(egui::Slider::new(&mut self.zoom, 0.2..=6.0).text("Zoom"));

                ui.horizontal(|ui| {
                    ui.label("Icon path:");
                    ui.text_edit_singleline(&mut self.new_icon_path);
                });

                if ui.button("Add icon").clicked() {
                    self.add_icon(ctx);
                }

                ui.separator();
                ui.label(&self.status);
                ui.separator();

                let current_selected = self.selected_icon;
                let mut pending_selected = None;

                ui.label(format!("Icons: {}", self.icons.len()));
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (index, icon) in self.icons.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let is_selected = current_selected == Some(index);
                            if ui.selectable_label(is_selected, &icon.name).clicked() {
                                pending_selected = Some(index);
                            }
                            ui.checkbox(&mut icon.visible, "Visible");
                        });
                    }
                });

                if let Some(selected) = current_selected.and_then(|index| self.icons.get_mut(index)) {
                    ui.separator();
                    ui.label(format!("Selected: {}", selected.name));
                    ui.label(format!("Path: {}", selected.path));
                    ui.add(egui::Slider::new(&mut selected.size, 8.0..=256.0).text("Size"));
                    ui.horizontal(|ui| {
                        ui.label(format!("X: {:.0}", selected.position.x));
                        ui.label(format!("Y: {:.0}", selected.position.y));
                    });
                }

                if let Some(index) = pending_selected {
                    self.selected_icon = Some(index);
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let viewport_size = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(viewport_size, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            let viewport_rect = rect;
            let map_rect = Rect::from_min_size(rect.min + self.pan_offset, self.map_size * self.zoom);

            if response.hovered() {
                let scroll_delta_y = ui.ctx().input(|input| input.raw_scroll_delta.y);
                if scroll_delta_y.abs() > f32::EPSILON {
                    let zoom_factor = (scroll_delta_y * 0.001).exp();
                    self.zoom = (self.zoom * zoom_factor).clamp(0.2, 6.0);
                }
            }

            let right_dragging = ui
                .ctx()
                .input(|input| input.pointer.button_down(PointerButton::Secondary));
            if right_dragging && response.hovered() {
                let drag_delta = ui.ctx().input(|input| input.pointer.delta());
                self.pan_offset += drag_delta;
            }

            if response.clicked_by(PointerButton::Primary) {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    let mut clicked_icon = None;

                    for (index, icon) in self.icons.iter().enumerate().rev() {
                        if !icon.visible {
                            continue;
                        }

                        let center = map_rect.min + icon.position.to_vec2() * self.zoom;
                        let icon_side = icon.size * self.zoom;
                        let icon_rect = Rect::from_center_size(center, Vec2::splat(icon_side));

                        if !icon_rect.intersects(viewport_rect) {
                            continue;
                        }

                        if icon_rect.contains(pointer_pos) {
                            clicked_icon = Some(index);
                            break;
                        }
                    }

                    if let Some(index) = clicked_icon {
                        self.selected_icon = Some(index);
                        if let Some(icon) = self.icons.get(index) {
                            self.status = format!(
                                "Selected icon: {} ({:.0}, {:.0})",
                                icon.name, icon.position.x, icon.position.y
                            );
                        }
                    }
                }
            }

            for tile in &self.map_tiles {
                let tile_min = map_rect.min + tile.position.to_vec2() * self.zoom;
                let tile_rect = Rect::from_min_size(tile_min, tile.size * self.zoom);

                if !tile_rect.intersects(viewport_rect) {
                    continue;
                }

                painter.image(
                    tile.texture.id(),
                    tile_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            for icon in self.icons.iter().filter(|icon| icon.visible) {
                let center = map_rect.min + icon.position.to_vec2() * self.zoom;
                let icon_side = icon.size * self.zoom;
                let icon_rect = Rect::from_center_size(center, Vec2::splat(icon_side));

                if !icon_rect.intersects(viewport_rect) {
                    continue;
                }

                painter.image(
                    icon.texture.id(),
                    icon_rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let (terrain_raster, cave_claims, watchtower_claims, temple_claims, dungeon_claims) =
        match build_terrain_raster() {
        Ok(result) => result,
        Err(error) => {
            eprintln!("Failed to load terrain map: {error}");
            return Ok(());
        }
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1500.0, 900.0]),
        ..Default::default()
    };

    eframe::run_native(
        "BitCraft Terrain Viewer",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(TerrainViewerApp::new(
                cc,
                terrain_raster,
                cave_claims,
                watchtower_claims,
                temple_claims,
                dungeon_claims,
            )))
        }),
    )
}