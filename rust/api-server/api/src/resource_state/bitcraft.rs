use crate::AppState;
use crate::websocket::SpacetimeUpdateMessages;
use entity::resource_state;
use game_module::module_bindings::ResourceState;
use migration::{OnConflict, sea_query};
use sea_orm::{ColumnTrait, EntityTrait, IntoActiveModel, ModelTrait, QueryFilter};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::sleep;

pub(crate) fn start_worker_resource_state(
    global_app_state: AppState,
    mut rx: UnboundedReceiver<SpacetimeUpdateMessages<ResourceState>>,
    batch_size: usize,
    time_limit: Duration,
) {
    tokio::spawn(async move {
        let on_conflict = sea_query::OnConflict::column(resource_state::Column::EntityId)
            .update_columns([
                resource_state::Column::ResourceId,
                resource_state::Column::DirectionIndex,
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
                                let mut local_messages = Vec::with_capacity(batch_size + 10);
                                let mut currently_known = ::entity::resource_state::Entity::find()
                                    .all(&global_app_state.conn)
                                    .await
                                    .map_or(vec![], |rows| rows)
                                    .into_iter()
                                    .map(|value| (value.entity_id, value))
                                    .collect::<HashMap<_, _>>();

                                for model in data.into_iter().map(|value| {
                                    let model: ::entity::resource_state::Model = value.into();
                                    model
                                }) {
                                    use std::collections::hash_map::Entry;
                                    match currently_known.entry(model.entity_id) {
                                        Entry::Occupied(entry) => {
                                            if entry.get() != &model {
                                                local_messages.push(model.into_active_model());
                                            }
                                            entry.remove();
                                        }
                                        Entry::Vacant(_) => {
                                            local_messages.push(model.into_active_model());
                                        }
                                    }

                                    if local_messages.len() >= batch_size {
                                        insert_multiple_resource_state(
                                            &global_app_state,
                                            &on_conflict,
                                            &mut local_messages,
                                        )
                                        .await;
                                    }
                                }

                                if !local_messages.is_empty() {
                                    insert_multiple_resource_state(
                                        &global_app_state,
                                        &on_conflict,
                                        &mut local_messages,
                                    )
                                    .await;
                                }

                                for entity_ids in currently_known.into_keys().collect::<Vec<_>>().chunks(1000) {
                                    let entity_ids = entity_ids.to_vec();
                                    if let Err(error) = ::entity::resource_state::Entity::delete_many()
                                        .filter(::entity::resource_state::Column::EntityId.is_in(entity_ids.clone()))
                                        .exec(&global_app_state.conn)
                                        .await
                                    {
                                        let ids_str: Vec<String> =
                                            entity_ids.iter().map(|id| id.to_string()).collect();
                                        tracing::error!(
                                            ResourceState = ids_str.join(","),
                                            error = error.to_string(),
                                            "Could not delete ResourceState"
                                        );
                                    }
                                }
                            }
                            SpacetimeUpdateMessages::Insert { new, .. } => {
                                let model: ::entity::resource_state::Model = new.into();
                                messages.push(model.into_active_model());
                                if messages.len() >= batch_size {
                                    break;
                                }
                            }
                            SpacetimeUpdateMessages::Update { new, .. } => {
                                let model: ::entity::resource_state::Model = new.into();
                                if let Some(index) = messages
                                    .iter()
                                    .position(|value| value.entity_id.as_ref() == &model.entity_id)
                                {
                                    messages.remove(index);
                                }
                                messages.push(model.into_active_model());
                                if messages.len() >= batch_size {
                                    break;
                                }
                            }
                            SpacetimeUpdateMessages::Remove { delete, .. } => {
                                let model: ::entity::resource_state::Model = delete.into();
                                if let Some(index) = messages
                                    .iter()
                                    .position(|value| value.entity_id.as_ref() == &model.entity_id)
                                {
                                    messages.remove(index);
                                }

                                if let Err(error) = ::entity::resource_state::Entity::delete_by_id(model.entity_id)
                                    .exec(&global_app_state.conn)
                                    .await
                                {
                                    tracing::error!(
                                        ResourceState = model.entity_id,
                                        error = error.to_string(),
                                        "Could not delete ResourceState"
                                    );
                                }
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
                insert_multiple_resource_state(&global_app_state, &on_conflict, &mut messages).await;
            }

            if messages.is_empty() && rx.is_closed() {
                break;
            }
        }
    });
}

async fn insert_multiple_resource_state(
    global_app_state: &AppState,
    on_conflict: &OnConflict,
    messages: &mut Vec<::entity::resource_state::ActiveModel>,
) {
    let insert = ::entity::resource_state::Entity::insert_many(messages.clone())
        .on_conflict(on_conflict.clone())
        .exec(&global_app_state.conn)
        .await;

    if let Err(error) = insert {
        tracing::error!("Error inserting ResourceState: {}", error);
    }

    messages.clear();
}