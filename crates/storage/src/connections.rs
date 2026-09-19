use anyhow::Result;
use dbstudio_core::models::{ConnectionConfig, DatabaseType};
use sqlx::{Pool, Sqlite};

const KEYRING_SERVICE: &str = dbstudio_core::NAMESPACE;
const LEGACY_KEYRING_SERVICE: &str = dbstudio_core::LEGACY_NAMESPACE;

/// Column list shared by every `connections` read query — keep in sync with
/// [`Row`] and the `INSERT` statement in [`ConnectionsRepository::save`].
const SELECT_COLUMNS: &str = "id, name, db_type, host, port, database, username,
                              ssh_enabled, ssh_host, ssh_port, ssh_username, ssh_auth_type,
                              ssh_key_path, extra_params, color, created_at, updated_at";

pub struct ConnectionsRepository<'a> {
    pool: &'a Pool<Sqlite>,
}

impl<'a> ConnectionsRepository<'a> {
    pub fn new(pool: &'a Pool<Sqlite>) -> Self {
        Self { pool }
    }

    fn keyring_entry(id: &str) -> keyring::Entry {
        Self::keyring_entry_for(KEYRING_SERVICE, id)
    }

    fn legacy_keyring_entry(id: &str) -> keyring::Entry {
        Self::keyring_entry_for(LEGACY_KEYRING_SERVICE, id)
    }

    fn keyring_entry_for(service: &str, id: &str) -> keyring::Entry {
        keyring::Entry::new(service, id).unwrap_or_else(|_| {
            keyring::Entry::new_with_target(id, service, "")
                .expect("Failed to create keyring entry")
        })
    }

    pub async fn load_all(&self) -> Result<Vec<ConnectionConfig>> {
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM connections ORDER BY name ASC"
        ))
        .fetch_all(self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into_info()).collect())
    }

    pub async fn load_by_id(&self, id: &str) -> Result<Option<ConnectionConfig>> {
        let row: Option<Row> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM connections WHERE id = ?1"
        ))
        .bind(id)
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| r.into_info()))
    }

    pub fn get_password(&self, connection_id: &str) -> String {
        if let Ok(password) = Self::keyring_entry(connection_id).get_password() {
            if !password.is_empty() {
                return password;
            }
        }

        // One-time migration: pre-rename installs stored passwords under the
        // `dbclient` keyring service. Re-export the credential into the current
        // namespace on first read; the legacy entry is kept as a backup.
        if let Ok(legacy) = Self::legacy_keyring_entry(connection_id).get_password() {
            if !legacy.is_empty() {
                let _ = Self::keyring_entry(connection_id).set_password(&legacy);
                return legacy;
            }
        }

        String::new()
    }

    pub fn set_password(&self, connection_id: &str, password: &str) -> Result<()> {
        if password.is_empty() {
            let _ = Self::keyring_entry(connection_id).delete_credential();
            // Also clear the legacy entry so an old credential cannot resurface
            // through the migration fallback after the user saved an empty one.
            let _ = Self::legacy_keyring_entry(connection_id).delete_credential();
        } else {
            Self::keyring_entry(connection_id).set_password(password)?;
        }
        Ok(())
    }

    pub fn get_ssh_password(&self, connection_id: &str) -> String {
        Self::keyring_entry(&format!("ssh:{connection_id}"))
            .get_password()
            .unwrap_or_default()
    }

    pub fn set_ssh_password(&self, connection_id: &str, password: &str) -> Result<()> {
        let entry = Self::keyring_entry(&format!("ssh:{connection_id}"));
        if password.is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(password)?;
        }
        Ok(())
    }

    pub fn get_ssh_passphrase(&self, connection_id: &str) -> String {
        Self::keyring_entry(&format!("sshpass:{connection_id}"))
            .get_password()
            .unwrap_or_default()
    }

    pub fn set_ssh_passphrase(&self, connection_id: &str, passphrase: &str) -> Result<()> {
        let entry = Self::keyring_entry(&format!("sshpass:{connection_id}"));
        if passphrase.is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(passphrase)?;
        }
        Ok(())
    }

    pub async fn save(&self, info: &ConnectionConfig, password: &str) -> Result<()> {
        if !password.is_empty() {
            self.set_password(&info.id, password)?;
        }

        sqlx::query(
            "INSERT OR REPLACE INTO connections
             (id, name, db_type, host, port, database, username,
              ssh_enabled, ssh_host, ssh_port, ssh_username, ssh_auth_type,
              ssh_key_path, extra_params, color, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )
        .bind(&info.id)
        .bind(&info.name)
        .bind(info.db_type.as_str())
        .bind(&info.host)
        .bind(info.port)
        .bind(&info.database)
        .bind(&info.username)
        .bind(info.ssh_enabled as i32)
        .bind(&info.ssh_host)
        .bind(info.ssh_port.map(|p| p as i32))
        .bind(&info.ssh_username)
        .bind(&info.ssh_auth_type)
        .bind(&info.ssh_key_path)
        .bind(&info.extra_params)
        .bind(&info.color)
        .bind(&info.created_at)
        .bind(&info.updated_at)
        .execute(self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM connections WHERE id = ?1")
            .bind(id)
            .execute(self.pool)
            .await?;
        let _ = Self::keyring_entry(id).delete_credential();
        let _ = Self::legacy_keyring_entry(id).delete_credential();
        let _ = Self::keyring_entry(&format!("ssh:{id}")).delete_credential();
        let _ = Self::keyring_entry(&format!("sshpass:{id}")).delete_credential();
        Ok(())
    }

    pub async fn count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM connections")
            .fetch_one(self.pool)
            .await?;
        Ok(row.0)
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    db_type: String,
    host: String,
    port: i32,
    database: String,
    username: String,
    ssh_enabled: i32,
    ssh_host: Option<String>,
    ssh_port: Option<i32>,
    ssh_username: Option<String>,
    ssh_auth_type: Option<String>,
    ssh_key_path: Option<String>,
    extra_params: Option<String>,
    color: Option<String>,
    created_at: String,
    updated_at: String,
}

impl Row {
    fn into_info(self) -> ConnectionConfig {
        let db_type = self
            .db_type
            .parse::<DatabaseType>()
            .unwrap_or(DatabaseType::PostgreSQL);

        ConnectionConfig {
            id: self.id,
            name: self.name,
            db_type,
            host: self.host,
            port: self.port as u16,
            database: self.database,
            username: self.username,
            color: self.color,
            ssh_enabled: self.ssh_enabled != 0,
            ssh_host: self.ssh_host,
            ssh_port: self.ssh_port.map(|p| p as u16),
            ssh_username: self.ssh_username,
            ssh_auth_type: self.ssh_auth_type,
            ssh_key_path: self.ssh_key_path,
            extra_params: self.extra_params,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}
