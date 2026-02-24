use crate::sea_orm::Statement;
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let stmt = Statement::from_string(
            manager.get_database_backend(),
            "DO $$\n"
                .to_string()
                + "DECLARE pk_name text;\n"
                + "BEGIN\n"
                + "  SELECT conname INTO pk_name\n"
                + "  FROM pg_constraint\n"
                + "  WHERE conrelid = 'inventory_changelog'::regclass\n"
                + "    AND contype = 'p';\n"
                + "  IF pk_name IS NOT NULL THEN\n"
                + "    EXECUTE format('ALTER TABLE inventory_changelog DROP CONSTRAINT %I', pk_name);\n"
                + "  END IF;\n"
                + "END $$;",
        );

        db.execute(stmt).await?;

        let stmt = Statement::from_string(
            manager.get_database_backend(),
            "ALTER TABLE inventory_changelog ADD PRIMARY KEY (id, timestamp);".to_string(),
        );

        db.execute(stmt).await?;

        let stmt = Statement::from_string(
            manager.get_database_backend(),
            "SELECT create_hypertable('inventory_changelog', by_range('timestamp', INTERVAL '1 day'), migrate_data => true);".to_string(),
        );

        db.execute(stmt).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
