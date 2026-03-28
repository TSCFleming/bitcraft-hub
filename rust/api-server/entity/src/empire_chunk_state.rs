use game_module::module_bindings::EmpireChunkState;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "empire_chunk_state")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub chunk_index: i64,
    pub empire_entity_id: i64,
    pub watchtower_entity_id: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<EmpireChunkState> for Model {
    fn from(value: EmpireChunkState) -> Self {
        Self {
            chunk_index: value.chunk_index as i64,
            empire_entity_id: value.empire_entity_id as i64,
            watchtower_entity_id: value.watchtower_entity_id as i64,
        }
    }
}