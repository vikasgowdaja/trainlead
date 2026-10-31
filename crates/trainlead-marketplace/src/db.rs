use std::str::FromStr;

use sqlx::{SqlitePool, sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions}};

use crate::Result;

pub async fn connect(url: &str) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?.create_if_missing(true)
        .foreign_keys(true).journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new().max_connections(if url.contains(":memory:") { 1 } else { 5 })
        .connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_seed_roles_and_make_audit_append_only() -> Result<()> {
        let pool = connect("sqlite::memory:").await?;
        let roles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles").fetch_one(&pool).await?;
        assert_eq!(roles, 6);
        sqlx::query("INSERT INTO audit_log(id,action,object_type,object_id,occurred_at) VALUES('test','test','test','test',0)").execute(&pool).await?;
        assert!(sqlx::query("UPDATE audit_log SET action='changed' WHERE id='test'").execute(&pool).await.is_err());
        assert!(sqlx::query("DELETE FROM audit_log WHERE id='test'").execute(&pool).await.is_err());
        Ok(())
    }
}