use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub servers: Vec<ServerConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub ssh: String,
    pub term: Option<String>,
    #[serde(default)]
    pub local: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub alias: Option<String>,
    pub server: String,
    pub session: String,
    pub root_path: String,
    pub agent: String,
    pub panes: Vec<Pane>,
    pub note: String,
    pub status: String,
    pub presence: String,
    pub resurrectable: bool,
    pub tags: Vec<String>,
    pub last_seen: String,
    pub last_attached_at: Option<String>,
    pub attach_count: i64,
    pub git: Option<GitInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub remote: Option<String>,
    pub dirty: bool,
    pub ahead: i64,
    pub behind: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pane {
    pub pane_id: String,
    pub tab_name: String,
    pub tab_position: i64,
    pub pane: i64,
    pub active: bool,
    pub is_floating: bool,
    pub command: String,
    pub path: String,
    pub title: String,
}

#[derive(Debug)]
pub struct DoctorReport {
    pub hostname: String,
    pub zellij_available: bool,
    pub zellij_version: Option<String>,
    pub sessions: Vec<String>,
}
