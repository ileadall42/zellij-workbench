use anyhow::{Result, bail};
use chrono::Utc;
use rusqlite::{Connection, params};

use crate::{
    config::data_path,
    model::{GitInfo, Pane, Workspace},
    util::shell_quote,
};

pub fn open_db() -> Result<Connection> {
    let path = data_path()?.join("workspaces.db");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Connection::open(path).map_err(Into::into)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        create table if not exists workspaces (
          id text primary key,
          name text not null,
          alias text,
          server text not null,
          session text not null,
          root_path text not null,
          agent text not null,
          note text not null default '',
          status text not null default 'active',
          presence text not null default 'seen',
          resurrectable integer not null default 0,
          tags text not null default '',
          last_seen text not null,
          last_attached_at text,
          attach_count integer not null default 0,
          git_branch text,
          git_head text,
          git_remote text,
          git_dirty integer,
          git_ahead integer,
          git_behind integer
        );

        create table if not exists panes (
          workspace_id text not null,
          pane_id text not null default '',
          tab_name text not null default '',
          tab_position integer not null default 0,
          pane integer not null,
          active integer not null,
          is_floating integer not null default 0,
          command text not null,
          path text not null,
          title text not null,
          foreign key(workspace_id) references workspaces(id)
        );
        ",
    )?;
    add_column_if_missing(
        conn,
        "workspaces",
        "resurrectable",
        "alter table workspaces add column resurrectable integer not null default 0",
    )?;
    add_column_if_missing(
        conn,
        "panes",
        "pane_id",
        "alter table panes add column pane_id text not null default ''",
    )?;
    add_column_if_missing(
        conn,
        "panes",
        "tab_name",
        "alter table panes add column tab_name text not null default ''",
    )?;
    add_column_if_missing(
        conn,
        "panes",
        "tab_position",
        "alter table panes add column tab_position integer not null default 0",
    )?;
    add_column_if_missing(
        conn,
        "panes",
        "is_floating",
        "alter table panes add column is_floating integer not null default 0",
    )?;
    conn.pragma_update(None, "user_version", 1)?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<()> {
    let sql = format!(
        "select count(*) from pragma_table_info({}) where name = ?1",
        shell_quote(table)
    );
    let exists: i64 = conn.query_row(&sql, params![column], |row| row.get(0))?;
    if exists == 0 {
        conn.execute(alter_sql, [])?;
    }
    Ok(())
}

pub fn upsert_workspace(conn: &Connection, ws: &Workspace) -> Result<()> {
    conn.execute(
        "insert into workspaces (id, name, alias, server, session, root_path, agent, note, status, presence, resurrectable, tags, last_seen, last_attached_at, attach_count, git_branch, git_head, git_remote, git_dirty, git_ahead, git_behind)
         values (?1, ?2, (select alias from workspaces where id = ?1), ?3, ?4, ?5, ?6, coalesce((select note from workspaces where id = ?1), ''), coalesce((select status from workspaces where id = ?1), 'active'), 'seen', ?14, coalesce((select tags from workspaces where id = ?1), ''), ?7, (select last_attached_at from workspaces where id = ?1), coalesce((select attach_count from workspaces where id = ?1), 0), ?8, ?9, ?10, ?11, ?12, ?13)
         on conflict(id) do update set
           name = excluded.name,
           server = excluded.server,
           session = excluded.session,
           root_path = excluded.root_path,
           agent = excluded.agent,
           last_seen = excluded.last_seen,
           alias = workspaces.alias,
           note = workspaces.note,
           status = workspaces.status,
           presence = excluded.presence,
           resurrectable = excluded.resurrectable,
           tags = workspaces.tags,
           last_attached_at = workspaces.last_attached_at,
           attach_count = workspaces.attach_count,
           git_branch = excluded.git_branch,
           git_head = excluded.git_head,
           git_remote = excluded.git_remote,
           git_dirty = excluded.git_dirty,
           git_ahead = excluded.git_ahead,
           git_behind = excluded.git_behind",
        params![
            ws.id,
            ws.name,
            ws.server,
            ws.session,
            ws.root_path,
            ws.agent,
            ws.last_seen,
            ws.git.as_ref().and_then(|git| git.branch.as_deref()),
            ws.git.as_ref().and_then(|git| git.head.as_deref()),
            ws.git.as_ref().and_then(|git| git.remote.as_deref()),
            ws.git.as_ref().map(|git| git.dirty as i64),
            ws.git.as_ref().map(|git| git.ahead),
            ws.git.as_ref().map(|git| git.behind),
            ws.resurrectable as i64,
        ],
    )?;
    conn.execute("delete from panes where workspace_id = ?1", params![ws.id])?;
    for pane in &ws.panes {
        conn.execute(
            "insert into panes (workspace_id, pane_id, tab_name, tab_position, pane, active, is_floating, command, path, title)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                ws.id,
                pane.pane_id,
                pane.tab_name,
                pane.tab_position,
                pane.pane,
                pane.active as i64,
                pane.is_floating as i64,
                pane.command,
                pane.path,
                pane.title
            ],
        )?;
    }
    Ok(())
}

pub fn load_workspaces(conn: &Connection) -> Result<Vec<Workspace>> {
    let mut stmt = conn.prepare(
        "select id, name, alias, server, session, root_path, agent, note, status, presence, resurrectable, tags, last_seen, last_attached_at, attach_count, git_branch, git_head, git_remote, git_dirty, git_ahead, git_behind
         from workspaces order by coalesce(last_attached_at, last_seen) desc, name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Workspace {
            id: row.get(0)?,
            name: row.get(1)?,
            alias: row.get(2)?,
            server: row.get(3)?,
            session: row.get(4)?,
            root_path: row.get(5)?,
            agent: row.get(6)?,
            note: row.get(7)?,
            status: row.get(8)?,
            presence: row.get(9)?,
            resurrectable: row.get::<_, i64>(10)? != 0,
            tags: parse_tags(&row.get::<_, String>(11)?),
            last_seen: row.get(12)?,
            last_attached_at: row.get(13)?,
            attach_count: row.get(14)?,
            git: git_info_from_row(
                row.get(15)?,
                row.get(16)?,
                row.get(17)?,
                row.get(18)?,
                row.get(19)?,
                row.get(20)?,
            ),
            panes: Vec::new(),
        })
    })?;

    let mut workspaces = Vec::new();
    for row in rows {
        let mut ws = row?;
        ws.panes = load_panes(conn, &ws.id)?;
        workspaces.push(ws);
    }
    Ok(workspaces)
}

pub fn mark_server_missing(conn: &Connection, server: &str) -> Result<usize> {
    conn.execute(
        "update workspaces set presence = 'missing' where server = ?1",
        params![server],
    )
    .map_err(Into::into)
}

fn git_info_from_row(
    branch: Option<String>,
    head: Option<String>,
    remote: Option<String>,
    dirty: Option<i64>,
    ahead: Option<i64>,
    behind: Option<i64>,
) -> Option<GitInfo> {
    if branch.is_none()
        && head.is_none()
        && remote.is_none()
        && dirty.is_none()
        && ahead.is_none()
        && behind.is_none()
    {
        return None;
    }
    Some(GitInfo {
        branch,
        head,
        remote,
        dirty: dirty.unwrap_or(0) != 0,
        ahead: ahead.unwrap_or(0),
        behind: behind.unwrap_or(0),
    })
}

pub fn find_workspace(conn: &Connection, name: &str) -> Result<Option<Workspace>> {
    let matches: Vec<Workspace> = load_workspaces(conn)?
        .into_iter()
        .filter(|ws| {
            ws.id == name
                || ws.name == name
                || ws.session == name
                || ws.alias.as_deref() == Some(name)
        })
        .collect();
    if matches.len() > 1 {
        let ids = matches
            .iter()
            .map(|ws| ws.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("ambiguous workspace `{name}`; use one of: {ids}");
    }
    Ok(matches.into_iter().next())
}

fn load_panes(conn: &Connection, workspace_id: &str) -> Result<Vec<Pane>> {
    let mut stmt = conn.prepare(
        "select pane_id, tab_name, tab_position, pane, active, is_floating, command, path, title
         from panes where workspace_id = ?1 order by tab_position, pane",
    )?;
    let rows = stmt.query_map(params![workspace_id], |row| {
        Ok(Pane {
            pane_id: row.get(0)?,
            tab_name: row.get(1)?,
            tab_position: row.get(2)?,
            pane: row.get(3)?,
            active: row.get::<_, i64>(4)? == 1,
            is_floating: row.get::<_, i64>(5)? == 1,
            command: row.get(6)?,
            path: row.get(7)?,
            title: row.get(8)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn set_note_by_id(conn: &Connection, id: &str, note: &str) -> Result<usize> {
    conn.execute(
        "update workspaces set note = ?1 where id = ?2",
        params![note, id],
    )
    .map_err(Into::into)
}

pub fn set_status_by_id(conn: &Connection, id: &str, status: &str) -> Result<usize> {
    conn.execute(
        "update workspaces set status = ?1 where id = ?2",
        params![status, id],
    )
    .map_err(Into::into)
}

pub fn set_alias_by_id(conn: &Connection, id: &str, alias: Option<&str>) -> Result<usize> {
    conn.execute(
        "update workspaces set alias = ?1 where id = ?2",
        params![alias, id],
    )
    .map_err(Into::into)
}

pub fn set_tags_by_id(conn: &Connection, id: &str, tags: &[String]) -> Result<usize> {
    conn.execute(
        "update workspaces set tags = ?1 where id = ?2",
        params![format_tags(tags), id],
    )
    .map_err(Into::into)
}

pub fn record_attach(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "update workspaces
         set last_attached_at = ?1,
             attach_count = attach_count + 1
         where id = ?2",
        params![Utc::now().to_rfc3339(), id],
    )?;
    Ok(())
}

fn parse_tags(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn format_tags(tags: &[String]) -> String {
    tags.iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::{
        load_workspaces, migrate, record_attach, set_alias_by_id, set_note_by_id, set_status_by_id,
        set_tags_by_id, upsert_workspace,
    };
    use crate::model::{Pane, Workspace};

    #[test]
    fn upsert_preserves_user_metadata_and_attach_history() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let first = test_workspace("server/session", "/repo");
        upsert_workspace(&conn, &first).unwrap();
        set_alias_by_id(&conn, &first.id, Some("alias")).unwrap();
        set_note_by_id(&conn, &first.id, "note").unwrap();
        set_status_by_id(&conn, &first.id, "paused").unwrap();
        set_tags_by_id(&conn, &first.id, &["tag".to_string()]).unwrap();
        record_attach(&conn, &first.id).unwrap();

        let second = test_workspace("server/session", "/repo/subdir");
        upsert_workspace(&conn, &second).unwrap();

        let workspace = load_workspaces(&conn).unwrap().remove(0);
        assert_eq!(workspace.alias.as_deref(), Some("alias"));
        assert_eq!(workspace.note, "note");
        assert_eq!(workspace.status, "paused");
        assert_eq!(workspace.presence, "seen");
        assert_eq!(workspace.tags, vec!["tag"]);
        assert!(workspace.last_attached_at.is_some());
        assert_eq!(workspace.attach_count, 1);
        assert_eq!(workspace.root_path, "/repo/subdir");
    }

    #[test]
    fn same_session_name_on_different_servers_does_not_collide() {
        // Two different machines can each have a zellij session named "api";
        // the workspace id is namespaced by server, so both must be retained
        // independently rather than overwriting each other.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let on_host_a = test_workspace("host-a/api", "/srv/api");
        let on_host_b = test_workspace("host-b/api", "/srv/api");
        upsert_workspace(&conn, &on_host_a).unwrap();
        upsert_workspace(&conn, &on_host_b).unwrap();

        let mut workspaces = load_workspaces(&conn).unwrap();
        workspaces.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(workspaces.len(), 2);
        assert_eq!(workspaces[0].id, "host-a/api");
        assert_eq!(workspaces[0].server, "host-a");
        assert_eq!(workspaces[1].id, "host-b/api");
        assert_eq!(workspaces[1].server, "host-b");
    }

    fn test_workspace(id: &str, root_path: &str) -> Workspace {
        let (server, session) = id.split_once('/').unwrap();
        Workspace {
            id: id.to_string(),
            name: session.to_string(),
            alias: None,
            server: server.to_string(),
            session: session.to_string(),
            root_path: root_path.to_string(),
            agent: "bash".to_string(),
            panes: vec![Pane {
                pane_id: "terminal_0".to_string(),
                tab_name: "Tab #1".to_string(),
                tab_position: 0,
                pane: 0,
                active: true,
                is_floating: false,
                command: "bash".to_string(),
                path: root_path.to_string(),
                title: String::new(),
            }],
            note: String::new(),
            status: "active".to_string(),
            presence: "seen".to_string(),
            resurrectable: false,
            tags: Vec::new(),
            last_seen: "now".to_string(),
            last_attached_at: None,
            attach_count: 0,
            git: None,
        }
    }
}
