#!/usr/bin/env bash
# Seeds a throwaway config + SQLite index with realistic-looking (but fake)
# workspaces, for recording demo GIFs without needing real remote hosts.
#
# Usage: scripts/demo-fixture.sh <config-dir> <data-dir>
set -euo pipefail

config_dir="${1:?usage: demo-fixture.sh <config-dir> <data-dir>}"
data_dir="${2:?usage: demo-fixture.sh <config-dir> <data-dir>}"

mkdir -p "$config_dir" "$data_dir"

cat >"$config_dir/config.yaml" <<'YAML'
servers:
  - name: local
    ssh: ""
    term: xterm-256color
    local: true
  - name: prod
    ssh: ssh prod
    term: xterm-256color
    local: false
  - name: research
    ssh: ssh research
    term: xterm-256color
    local: false
YAML

sqlite3 "$data_dir/workspaces.db" <<'SQL'
create table workspaces (
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

create table panes (
  workspace_id text not null,
  pane_id text not null default '',
  tab_name text not null default '',
  tab_position integer not null default 0,
  pane integer not null,
  active integer not null,
  is_floating integer not null default 0,
  command text not null,
  path text not null,
  title text not null
);

insert into workspaces values
('prod/api','api','api','prod','api','/srv/api','codex','Backend uses uv. Check worker before deploy.','active','seen',0,'backend,prod','2026-07-01T10:55:00Z','2026-07-01T10:58:00Z',8,'main','d43063f','https://github.com/example/api',1,1,0),
('prod/worker','worker',null,'prod','worker','/srv/worker','bash','Runs queue consumers and btop.','active','seen',0,'backend,prod','2026-07-01T09:40:00Z','2026-07-01T09:55:00Z',4,'release','a81f222','https://github.com/example/worker',1,0,0),
('research/neuroplay','neuroplay','neuro','research','neuroplay','/data/code/neuroplay','claude','Frontend in ./web. Dataset notes in docs/. Exited overnight, attach to resurrect.','active','seen',1,'research,frontend','2026-07-01T02:10:00Z',null,6,'main','91c2f04','https://github.com/example/neuroplay',0,0,0),
('local/zellij-workbench','zellij-workbench','zw','local','zellij-workbench','~/code/zellij-workbench','zsh','Port of tmux-workbench to zellij. Tests + docs done.','active','seen',0,'oss,rust','2026-07-01T10:45:00Z','2026-07-01T10:50:00Z',15,'main','acade4e','https://github.com/ileadall42/zellij-workbench',1,0,0),
('prod/old-dashboard','old-dashboard',null,'prod','old-dashboard','/srv/dashboard','node','Archived after migration to admin-v2.','archived','missing',0,'frontend,legacy','2026-06-28T12:00:00Z',null,1,'legacy','0ac91be','https://github.com/example/dashboard',0,0,3);

insert into panes values
('prod/api','terminal_0','Tab #1',0,0,1,0,'codex','/srv/api','api agent'),
('prod/api','terminal_1','Tab #1',0,1,0,0,'zsh','/srv/api','shell'),
('prod/worker','terminal_0','Tab #1',0,0,1,0,'bash','/srv/worker','worker shell'),
('prod/worker','terminal_1','Tab #2',1,0,0,0,'btop','/srv/worker','monitor'),
('research/neuroplay','terminal_0','Tab #1',0,0,1,0,'claude','/data/code/neuroplay','claude'),
('research/neuroplay','terminal_1','Tab #2',1,0,0,0,'npm','/data/code/neuroplay/web','web'),
('local/zellij-workbench','terminal_0','Tab #1',0,0,1,0,'zsh','~/code/zellij-workbench','local shell'),
('local/zellij-workbench','terminal_1','Tab #2',1,0,0,0,'cargo','~/code/zellij-workbench','tests'),
('prod/old-dashboard','terminal_0','Tab #1',0,0,0,0,'node','/srv/dashboard','legacy');

pragma user_version = 1;
SQL
