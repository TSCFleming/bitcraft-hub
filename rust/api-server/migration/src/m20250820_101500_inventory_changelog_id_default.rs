use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let backend = manager.get_database_backend();
        let stmt = Statement::from_string(
            backend,
            r#"
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'inventory_changelog'
          AND column_name = 'id'
          AND column_default IS NOT NULL
    ) THEN
        IF NOT EXISTS (
            SELECT 1 FROM pg_class WHERE relname = 'inventory_changelog_id_seq'
        ) THEN
            EXECUTE 'CREATE SEQUENCE inventory_changelog_id_seq';
        END IF;

        EXECUTE 'ALTER TABLE inventory_changelog ALTER COLUMN id SET DEFAULT nextval(''inventory_changelog_id_seq'')';
        EXECUTE 'SELECT setval(''inventory_changelog_id_seq'', COALESCE((SELECT MAX(id) FROM inventory_changelog), 0) + 1, false)';
        EXECUTE 'ALTER SEQUENCE inventory_changelog_id_seq OWNED BY inventory_changelog.id';
    END IF;
END$$;
"#
            .to_string(),
        );
        db.execute(stmt).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let backend = manager.get_database_backend();
        let stmt = Statement::from_string(
            backend,
            r#"
DO $$
BEGIN
    EXECUTE 'ALTER TABLE inventory_changelog ALTER COLUMN id DROP DEFAULT';
    IF EXISTS (SELECT 1 FROM pg_class WHERE relname = 'inventory_changelog_id_seq') THEN
        EXECUTE 'DROP SEQUENCE inventory_changelog_id_seq';
    END IF;
END$$;
"#
            .to_string(),
        );
        db.execute(stmt).await?;
        Ok(())
    }
}
