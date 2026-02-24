use crate::AppState;
use crate::websocket::{SpacetimeUpdateMessages, WebSocketMessages};
use entity::terrain_chunk_state;
use game_module::module_bindings::TerrainChunkState;
use migration::{OnConflict, sea_query};
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, ModelTrait, QueryFilter};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::sleep;

pub(crate) fn start_worker_terrain_chunk_state(
    global_app_state: AppState,
    mut rx: UnboundedReceiver<SpacetimeUpdateMessages<TerrainChunkState>>,
    batch_size: usize,
    time_limit: Duration,
) {
    tokio::spawn(async move {
        let on_conflict = sea_query::OnConflict::column(terrain_chunk_state::Column::ChunkIndex)
            .update_columns([
                terrain_chunk_state::Column::ChunkX,
                terrain_chunk_state::Column::ChunkZ,
                terrain_chunk_state::Column::Dimension,
                terrain_chunk_state::Column::Biomes,
                terrain_chunk_state::Column::BiomeDensity,
                terrain_chunk_state::Column::Elevations,
                terrain_chunk_state::Column::WaterLevels,
                terrain_chunk_state::Column::WaterBodyTypes,
                terrain_chunk_state::Column::ZoningTypes,
                terrain_chunk_state::Column::OriginalElevations,
            ])
            .to_owned();

        loop {
            let mut messages = Vec::with_capacity(batch_size + 10);
            let timer = sleep(time_limit);
            tokio::pin!(timer);

            loop {
                tokio::select! {
                    Some(msg) = rx.recv() => {
                        match msg {
                            SpacetimeUpdateMessages::Initial { data, .. } => {
                                tracing::info!(
                                    "TerrainChunkState initial batch size: {}",
                                    data.len()
                                );
                                let mut local_messages = Vec::with_capacity(batch_size + 10);
                                let mut currently_known_chunks = ::entity::terrain_chunk_state::Entity::find()
                                    .all(&global_app_state.conn)
                                    .await
                                    .map_or(vec![], |rows| rows)
                                    .into_iter()
                                    .map(|value| (value.chunk_index, value))
                                    .collect::<HashMap<_, _>>();

                                tracing::info!(
                                    "TerrainChunkState existing rows in DB: {}",
                                    currently_known_chunks.len()
                                );

                                for model in data.into_iter().map(|value| {
                                    let model: ::entity::terrain_chunk_state::Model = value.into();
                                    model
                                }) {
                                    let _ = global_app_state
                                        .tx
                                        .send(WebSocketMessages::TerrainChunkState(model.clone()));

                                    use std::collections::hash_map::Entry;
                                    match currently_known_chunks.entry(model.chunk_index) {
                                        Entry::Occupied(entry) => {
                                            let existing_model = entry.get();
                                            if &model != existing_model {
                                                local_messages.push(model.into_active_model());
                                            }
                                            entry.remove();
                                        }
                                        Entry::Vacant(_entry) => {
                                            local_messages.push(model.into_active_model());
                                        }
                                    }

                                    if local_messages.len() >= batch_size {
                                        insert_multiple_terrain_chunk_state(
                                            &global_app_state,
                                            &on_conflict,
                                            &mut local_messages,
                                        )
                                        .await;
                                    }
                                }

                                tracing::info!(
                                    "TerrainChunkState pending inserts after initial pass: {}",
                                    local_messages.len()
                                );
                                if !local_messages.is_empty() {
                                    insert_multiple_terrain_chunk_state(
                                        &global_app_state,
                                        &on_conflict,
                                        &mut local_messages,
                                    )
                                    .await;
                                }

                                for chunk_ids in currently_known_chunks
                                    .into_keys()
                                    .collect::<Vec<_>>()
                                    .chunks(1000)
                                {
                                    let chunk_ids = chunk_ids.to_vec();
                                    if let Err(error) = ::entity::terrain_chunk_state::Entity::delete_many()
                                        .filter(::entity::terrain_chunk_state::Column::ChunkIndex.is_in(chunk_ids.clone()))
                                        .exec(&global_app_state.conn)
                                        .await
                                    {
                                        let chunk_ids_str: Vec<String> =
                                            chunk_ids.iter().map(|id| id.to_string()).collect();
                                        tracing::error!(
                                            TerrainChunkState = chunk_ids_str.join(","),
                                            error = error.to_string(),
                                            "Could not delete TerrainChunkState"
                                        );
                                    }
                                }
                            }
                            SpacetimeUpdateMessages::Insert { new, .. } => {
                                let model: ::entity::terrain_chunk_state::Model = new.into();

                                let _ = global_app_state
                                    .tx
                                    .send(WebSocketMessages::TerrainChunkState(model.clone()));

                                messages.push(model.into_active_model());
                                if messages.len() >= batch_size {
                                    break;
                                }
                            }
                            SpacetimeUpdateMessages::Update { new, .. } => {
                                let model: ::entity::terrain_chunk_state::Model = new.into();

                                if let Some(index) = messages
                                    .iter()
                                    .position(|value| value.chunk_index.as_ref() == &model.chunk_index)
                                {
                                    messages.remove(index);
                                }

                                let _ = global_app_state
                                    .tx
                                    .send(WebSocketMessages::TerrainChunkState(model.clone()));

                                messages.push(model.into_active_model());
                                if messages.len() >= batch_size {
                                    break;
                                }
                            }
                            SpacetimeUpdateMessages::Remove { delete, .. } => {
                                let model: ::entity::terrain_chunk_state::Model = delete.into();
                                let id = model.chunk_index;

                                if let Some(index) = messages
                                    .iter()
                                    .position(|value| value.chunk_index.as_ref() == &model.chunk_index)
                                {
                                    messages.remove(index);
                                }

                                if let Err(error) = ::entity::terrain_chunk_state::Entity::delete_by_id(id)
                                    .exec(&global_app_state.conn)
                                    .await
                                {
                                    tracing::error!(
                                        TerrainChunkState = id,
                                        error = error.to_string(),
                                        "Could not delete TerrainChunkState"
                                    );
                                }

                                tracing::debug!("TerrainChunkState::Remove");
                            }
                        }
                    }
                    _ = &mut timer => {
                        break;
                    }
                    else => {
                        break;
                    }
                }
            }

            if !messages.is_empty() {
                insert_multiple_terrain_chunk_state(&global_app_state, &on_conflict, &mut messages)
                    .await;
            }

            if messages.is_empty() && rx.is_closed() {
                break;
            }
        }
    });
}

async fn insert_multiple_terrain_chunk_state(
    global_app_state: &AppState,
    on_conflict: &OnConflict,
    messages: &mut Vec<::entity::terrain_chunk_state::ActiveModel>,
) {
    tracing::info!(
        "Inserting TerrainChunkState batch size: {}",
        messages.len()
    );
    let insert = ::entity::terrain_chunk_state::Entity::insert_many(messages.clone())
        .on_conflict(on_conflict.clone())
        .exec(&global_app_state.conn)
        .await;

    match insert {
        Ok(_) => {
            tracing::info!(
                "Inserted TerrainChunkState batch size: {}",
                messages.len()
            );
        }
        Err(error) => {
            tracing::error!("Error inserting TerrainChunkState: {}", error);
        }
    }

    messages.clear();
}
