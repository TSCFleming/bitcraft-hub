use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ResourceState::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ResourceState::EntityId)
                            .big_integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ResourceState::ResourceId).integer().not_null())
                    .col(
                        ColumnDef::new(ResourceState::DirectionIndex)
                            .integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-resource-state-resource-id")
                    .table(ResourceState::Table)
                    .col(ResourceState::ResourceId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(EmpireChunkState::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EmpireChunkState::ChunkIndex)
                            .big_integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(EmpireChunkState::EmpireEntityId)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(EmpireChunkState::WatchtowerEntityId)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-empire-chunk-state-empire-entity-id")
                    .table(EmpireChunkState::Table)
                    .col(EmpireChunkState::EmpireEntityId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx-empire-chunk-state-watchtower-entity-id")
                    .table(EmpireChunkState::Table)
                    .col(EmpireChunkState::WatchtowerEntityId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("idx-resource-state-resource-id")
                    .table(ResourceState::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx-empire-chunk-state-empire-entity-id")
                    .table(EmpireChunkState::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("idx-empire-chunk-state-watchtower-entity-id")
                    .table(EmpireChunkState::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(Table::drop().table(ResourceState::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(EmpireChunkState::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum ResourceState {
    Table,
    EntityId,
    ResourceId,
    DirectionIndex,
}

#[derive(DeriveIden)]
enum EmpireChunkState {
    Table,
    ChunkIndex,
    EmpireEntityId,
    WatchtowerEntityId,
}