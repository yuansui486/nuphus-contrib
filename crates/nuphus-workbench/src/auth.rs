use crate::{ApiError, Result, WorkbenchStore};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CAPABILITIES: &[&str] = &["read", "edit", "run", "automation", "projects", "respond"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Client {
    pub client_id: String,
    pub name: String,
    pub projects: Vec<String>,
    pub capabilities: Vec<String>,
    pub revoked: bool,
}

/// Constructed by the host or authentication middleware, never deserialized
/// from an external API request. External clients cannot claim to be the UI.
#[derive(Clone)]
pub enum Principal {
    LocalUi,
    Client(Client),
}

impl Principal {
    pub fn id(&self) -> &str {
        match self {
            Self::LocalUi => "local-ui",
            Self::Client(c) => &c.client_id,
        }
    }

    pub fn authorize(&self, capability: &str, project: Option<&str>) -> Result<()> {
        match self {
            Self::LocalUi => Ok(()),
            Self::Client(c)
                if !c.revoked
                    && c.capabilities.iter().any(|s| s == capability)
                    && project.is_none_or(|id| c.projects.iter().any(|s| s == id || s == "*")) =>
            {
                Ok(())
            }
            _ => Err(ApiError::new(
                "permission_denied",
                "This client is not authorized for the requested project and capability",
            )),
        }
    }
}

impl WorkbenchStore {
    /// The plaintext token is returned once. Only its digest is persisted.
    pub fn create_client(
        &self,
        name: &str,
        projects: Vec<String>,
        capabilities: Vec<String>,
    ) -> Result<(Client, String)> {
        if name.trim().is_empty()
            || capabilities
                .iter()
                .any(|s| !CAPABILITIES.contains(&s.as_str()))
        {
            return Err(ApiError::new(
                "invalid_params",
                "Client name and known capabilities are required",
            ));
        }
        for project in &projects {
            if project != "*" {
                self.project(project)?;
            }
        }
        let client = Client {
            client_id: uuid::Uuid::new_v4().to_string(),
            name: name.trim().into(),
            projects,
            capabilities,
            revoked: false,
        };
        let token = format!(
            "nw_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.registry()?.execute("INSERT INTO clients(client_id,name,token_hash,projects,capabilities) VALUES(?1,?2,?3,?4,?5)",
            params![client.client_id,client.name,token_hash(&token),serde_json::to_string(&client.projects)?,serde_json::to_string(&client.capabilities)?])?;
        Ok((client, token))
    }

    pub fn clients(&self) -> Result<Vec<Client>> {
        let conn = self.registry()?;
        let mut stmt = conn.prepare(
            "SELECT client_id,name,projects,capabilities,revoked FROM clients ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(client_id, name, projects, capabilities, revoked)| {
                Ok(Client {
                    client_id,
                    name,
                    projects: serde_json::from_str(&projects)?,
                    capabilities: serde_json::from_str(&capabilities)?,
                    revoked,
                })
            })
            .collect()
    }

    pub fn revoke_client(&self, id: &str) -> Result<()> {
        if self
            .registry()?
            .execute("UPDATE clients SET revoked=1 WHERE client_id=?1", [id])?
            == 0
        {
            return Err(ApiError::new("not_found", "Client not found"));
        }
        Ok(())
    }

    pub fn authenticate(&self, token: &str) -> Result<Principal> {
        let id: Option<String> = self
            .registry()?
            .query_row(
                "SELECT client_id FROM clients WHERE token_hash=?1 AND revoked=0",
                [token_hash(token)],
                |r| r.get(0),
            )
            .optional()?;
        let id =
            id.ok_or_else(|| ApiError::new("unauthorized", "Invalid or revoked client token"))?;
        self.clients()?
            .into_iter()
            .find(|c| c.client_id == id)
            .map(Principal::Client)
            .ok_or_else(|| ApiError::new("unauthorized", "Client not found"))
    }
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
