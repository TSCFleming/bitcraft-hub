use game_module::module_bindings::ResourceState;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "resource_state")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub entity_id: i64,
    pub resource_id: i32,
    pub direction_index: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<ResourceState> for Model {
    fn from(value: ResourceState) -> Self {
        Self {
            entity_id: value.entity_id as i64,
            resource_id: value.resource_id,
            direction_index: value.direction_index,
        }
    }
}