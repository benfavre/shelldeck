//! Building the assistant's `@` directory from live application state.
//!
//! The Workspace is the only place that can see every source at once —
//! connections, requests, tickets, terminals, scripts, sites, tunnels, open
//! files, the fleet — and the only place that knows who is asking. So it is the
//! only place allowed to decide what may be mentioned.
//!
//! Both gates from `docs/ai-mentions.md` § 5 are applied **here, at build
//! time**, not in the view: a candidate that a caller may not see is never
//! handed to the picker in the first place. The assistant re-checks the
//! surviving references against the directory again at send time, because a
//! draft can outlive a site switch.

use gpui::{App, Context};
use shelldeck_core::ai::{
    scoped_candidates, MentionCandidate, MentionKind, MentionScope, PersonRelation,
};
use shelldeck_core::config::manage_directory;
use shelldeck_core::models::script::ScriptTarget;
use std::rc::Rc;

use super::Workspace;
use crate::t;

impl Workspace {
    /// Who is asking, for both mention gates.
    pub(super) fn mention_scope(&self) -> MentionScope {
        MentionScope {
            signed_in: self.signed_in(),
            mode: self.effective_mode(),
            is_superadmin: self.is_superadmin(),
            is_inklura_support: self.is_inklura_support(),
            active_site_id: self.app_config.cloud_sync.active_site_id.clone(),
        }
    }

    /// Rebuild the directory and publish it to both assistant hosts.
    ///
    /// Cheap enough to run whenever the picker opens: it walks in-memory
    /// collections and bounds every payload. Nothing here touches the network —
    /// the one remote source (people) is a cache refreshed separately.
    pub(super) fn refresh_mention_directory(&mut self, cx: &mut Context<Self>) {
        let directory = Rc::new(self.build_mention_directory(cx));
        self.ai_assistant.update(cx, |assistant, cx| {
            assistant.set_mention_directory(directory.clone(), cx);
        });
        self.ai_dock_assistant.update(cx, |assistant, cx| {
            assistant.set_mention_directory(directory.clone(), cx);
        });
    }

    /// Every candidate the caller is allowed to reference right now.
    pub(super) fn build_mention_directory(&self, cx: &App) -> Vec<MentionCandidate> {
        let scope = self.mention_scope();
        if !scope.signed_in {
            return Vec::new();
        }
        let mut candidates: Vec<MentionCandidate> = Vec::new();
        if scope.allows_kind(MentionKind::Host) {
            candidates.extend(self.host_candidates());
        }
        if scope.allows_kind(MentionKind::Request) {
            candidates.extend(self.request_candidates());
        }
        if scope.allows_kind(MentionKind::Ticket) {
            candidates.extend(self.ticket_candidates(cx));
        }
        if scope.allows_kind(MentionKind::Terminal) {
            candidates.extend(self.terminal_candidates(cx));
        }
        if scope.allows_kind(MentionKind::Script) {
            candidates.extend(self.script_candidates(cx));
        }
        if scope.allows_kind(MentionKind::Site) {
            candidates.extend(self.site_candidates());
        }
        if scope.allows_kind(MentionKind::Tunnel) {
            candidates.extend(self.tunnel_candidates());
        }
        if scope.allows_kind(MentionKind::File) {
            candidates.extend(self.file_candidates(cx));
        }
        if scope.allows_kind(MentionKind::Instance) {
            candidates.extend(self.fleet_instance_candidates());
        }
        if scope.allows_kind(MentionKind::Job) {
            candidates.extend(self.fleet_job_candidates());
        }
        if scope.allows_kind(MentionKind::Person) {
            candidates.extend(self.person_candidates());
        }
        scoped_candidates(&scope, candidates)
    }

    // --------------------------------------------------------------- sources

    fn host_candidates(&self) -> Vec<MentionCandidate> {
        self.connections
            .iter()
            .map(|connection| {
                MentionCandidate::new(
                    MentionKind::Host,
                    connection.id.to_string(),
                    connection.display_name(),
                    connection.connection_string(),
                    serde_json::json!({
                        "alias": connection.alias,
                        "hostname": connection.hostname,
                        "port": connection.port,
                        "user": connection.user,
                        "group": connection.group,
                        "tags": connection.tags,
                        "source": format!("{:?}", connection.source),
                        "status": format!("{:?}", connection.status),
                        "site": connection.site_label,
                        "proxy_jump": connection.proxy_jump,
                        "forward_agent": connection.forward_agent,
                    }),
                )
                .site(
                    connection.site_id.map(|id| id.to_string()),
                    connection.site_label.clone(),
                )
                .keywords(format!(
                    "{} {} {}",
                    connection.hostname,
                    connection.user,
                    connection.tags.join(" ")
                ))
            })
            .collect()
    }

    fn request_candidates(&self) -> Vec<MentionCandidate> {
        self.issues_list
            .iter()
            .map(|issue| {
                MentionCandidate::new(
                    MentionKind::Request,
                    issue.id.clone(),
                    if issue.title.trim().is_empty() {
                        issue.id.clone()
                    } else {
                        issue.title.clone()
                    },
                    format!("{} · {}", issue.status, issue.priority),
                    serde_json::json!({
                        "id": issue.id,
                        "status": issue.status,
                        "priority": issue.priority,
                        "source": issue.source,
                        "requested_by": issue.requested_by,
                        "assignee": issue.assignee,
                        "site": issue.site_label,
                        "tenant": issue.tenant_name,
                        "comments": issue.comment_count,
                        "attachments": issue.attachment_count,
                        "created_at": issue.created_at,
                        "updated_at": issue.updated_at,
                        "body": issue.body,
                    }),
                )
                .site(issue.site_id.clone(), issue.site_label.clone())
                .keywords(format!("{} {}", issue.requested_by, issue.assignee))
            })
            .collect()
    }

    fn ticket_candidates(&self, cx: &App) -> Vec<MentionCandidate> {
        self.support
            .read(cx)
            .tickets()
            .iter()
            .map(|ticket| {
                MentionCandidate::new(
                    MentionKind::Ticket,
                    ticket.id.clone(),
                    if ticket.subject.trim().is_empty() {
                        ticket.contact.display()
                    } else {
                        ticket.subject.clone()
                    },
                    format!("{} · {}", ticket.contact.display(), ticket.status),
                    serde_json::json!({
                        "id": ticket.id,
                        "channel": ticket.channel,
                        "status": ticket.status,
                        "priority": ticket.priority,
                        "assignee": ticket.assignee,
                        "contact": ticket.contact.display(),
                        "tags": ticket.tags,
                        "messages": ticket.msg_count,
                        "last_at": ticket.last_at,
                        "sla_breaching": ticket.sla.breaching,
                        "last_preview": ticket.last_preview,
                    }),
                )
                .keywords(format!("{} {}", ticket.assignee, ticket.tags.join(" ")))
            })
            .collect()
    }

    fn terminal_candidates(&self, cx: &App) -> Vec<MentionCandidate> {
        self.terminal
            .read(cx)
            .mention_sessions()
            .into_iter()
            .map(|session| {
                let connection = session.connection_id.and_then(|id| {
                    self.connections
                        .iter()
                        .find(|connection| connection.id == id)
                });
                MentionCandidate::new(
                    MentionKind::Terminal,
                    session.id.to_string(),
                    session.title.clone(),
                    match connection {
                        Some(connection) => connection.connection_string(),
                        None => session.cwd.clone().unwrap_or_default(),
                    },
                    serde_json::json!({
                        "title": session.title,
                        "state": session.state,
                        "cwd": session.cwd,
                        "connection": connection.map(|connection| connection.display_name()),
                        // The tail is a snapshot, not a live read: the model is
                        // told when it was taken so a stale tail is legible as
                        // one.
                        "captured_at": session.captured_at,
                        "output_tail": session.tail,
                    }),
                )
                .site(
                    connection.and_then(|connection| connection.site_id.map(|id| id.to_string())),
                    connection.and_then(|connection| connection.site_label.clone()),
                )
            })
            .collect()
    }

    fn script_candidates(&self, cx: &App) -> Vec<MentionCandidate> {
        self.scripts
            .read(cx)
            .scripts
            .iter()
            .map(|script| {
                let target = match script.target {
                    ScriptTarget::Local => "local".to_string(),
                    ScriptTarget::AskOnRun => "ask_on_run".to_string(),
                    ScriptTarget::Remote(id) => self
                        .connections
                        .iter()
                        .find(|connection| connection.id == id)
                        .map(|connection| connection.display_name().to_string())
                        .unwrap_or_else(|| id.to_string()),
                };
                MentionCandidate::new(
                    MentionKind::Script,
                    script.id.to_string(),
                    script.name.clone(),
                    format!("{:?} · {}", script.language, target),
                    serde_json::json!({
                        "description": script.description,
                        "language": format!("{:?}", script.language),
                        "category": format!("{:?}", script.category),
                        "target": target,
                        "tags": script.tags,
                        "variables": script
                            .variables
                            .iter()
                            .map(|variable| variable.name.clone())
                            .collect::<Vec<_>>(),
                        "run_count": script.run_count,
                        "body": script.body,
                    }),
                )
                .keywords(script.tags.join(" "))
            })
            .collect()
    }

    fn site_candidates(&self) -> Vec<MentionCandidate> {
        let active = self.app_config.cloud_sync.active_site_id.clone();
        self.site_directory
            .as_ref()
            .map(|directory| {
                directory
                    .sites
                    .iter()
                    .map(|site| {
                        MentionCandidate::new(
                            MentionKind::Site,
                            site.site_id.clone(),
                            site.display_label(),
                            site.host.clone(),
                            serde_json::json!({
                                "site_id": site.site_id,
                                "tenant": site.tenant_name,
                                "host": site.host,
                                "is_wordpress": site.is_wordpress,
                                "wp_admin_url": site.wp_admin_url,
                                "active": active.as_deref() == Some(site.site_id.as_str()),
                            }),
                        )
                        // A site is in scope for itself: a caller who may see
                        // the site row may reference it.
                        .site(Some(site.site_id.clone()), Some(site.display_label()))
                        .keywords(format!("{} {}", site.tenant_name, site.host))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn tunnel_candidates(&self) -> Vec<MentionCandidate> {
        self.store
            .port_forwards
            .iter()
            .map(|forward| {
                let connection = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == forward.connection_id);
                let route = format!(
                    "{}:{} → {}:{}",
                    forward.local_host,
                    forward.local_port,
                    forward.remote_host,
                    forward.remote_port
                );
                MentionCandidate::new(
                    MentionKind::Tunnel,
                    forward.id.to_string(),
                    forward
                        .label
                        .clone()
                        .filter(|label| !label.trim().is_empty())
                        .unwrap_or_else(|| route.clone()),
                    route.clone(),
                    serde_json::json!({
                        "direction": format!("{:?}", forward.direction),
                        "local": format!("{}:{}", forward.local_host, forward.local_port),
                        "remote": format!("{}:{}", forward.remote_host, forward.remote_port),
                        "auto_start": forward.auto_start,
                        "active": self.active_tunnels.contains_key(&forward.id),
                        "connection": connection.map(|connection| connection.display_name()),
                    }),
                )
                .site(
                    connection.and_then(|connection| connection.site_id.map(|id| id.to_string())),
                    connection.and_then(|connection| connection.site_label.clone()),
                )
                .keywords(route)
            })
            .collect()
    }

    fn file_candidates(&self, cx: &App) -> Vec<MentionCandidate> {
        use crate::file_editor::view::TabContent;
        self.file_editor
            .read(cx)
            .tabs
            .iter()
            .filter_map(|tab| {
                let path = tab.path.as_ref()?;
                let (language, lines, dirty, excerpt) = match &tab.content {
                    TabContent::Text {
                        buffer, language, ..
                    } => (
                        format!("{language:?}"),
                        buffer.len_lines(),
                        buffer.is_dirty(),
                        buffer.text(),
                    ),
                    TabContent::Image { .. } => ("image".to_string(), 0, false, String::new()),
                    TabContent::Pdf { .. } => ("pdf".to_string(), 0, false, String::new()),
                    TabContent::Binary { .. } => ("binary".to_string(), 0, false, String::new()),
                };
                Some(
                    MentionCandidate::new(
                        MentionKind::File,
                        path.to_string_lossy().to_string(),
                        tab.filename.clone(),
                        path.parent()
                            .map(|parent| parent.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        serde_json::json!({
                            "path": path.to_string_lossy(),
                            "language": language,
                            "lines": lines,
                            "unsaved_changes": dirty,
                            // Bounded by `sanitize_detail`; a mention points at
                            // a file, it does not ship the whole repository.
                            "excerpt": excerpt,
                        }),
                    )
                    .keywords(path.to_string_lossy().to_string()),
                )
            })
            .collect()
    }

    fn fleet_instance_candidates(&self) -> Vec<MentionCandidate> {
        let Some(snapshot) = self.fleet_snapshot.as_ref() else {
            return Vec::new();
        };
        snapshot
            .resources
            .iter()
            .filter(|resource| {
                resource.resource.kind == shelldeck_core::config::platform::ResourceKind::Node
            })
            .map(|resource| {
                let id = resource.resource.id.as_str();
                MentionCandidate::new(
                    MentionKind::Instance,
                    id.to_owned(),
                    resource.summary.as_str().to_owned(),
                    format!(
                        "{} · {}",
                        resource.resource.authority.as_str(),
                        resource.freshness.state.as_str()
                    ),
                    serde_json::json!({
                        "id": id,
                        "authority": resource.resource.authority.as_str(),
                        "kind": resource.resource.kind.as_str(),
                        "freshness": resource.freshness.state.as_str(),
                        "observed_at": resource.freshness.observed_at.as_millis(),
                        "revision": resource.freshness.revision.get(),
                        "summary": resource.summary.as_str(),
                    }),
                )
                .keywords(resource.resource.authority.as_str())
            })
            .collect()
    }

    /// Fleet jobs. A job has no label of its own, so the first line of its
    /// prompt becomes one — which is also what the Fleet view shows.
    fn fleet_job_candidates(&self) -> Vec<MentionCandidate> {
        let Some(snapshot) = self.fleet_snapshot.as_ref() else {
            return Vec::new();
        };
        snapshot
            .resources
            .iter()
            .filter(|resource| {
                resource.resource.kind == shelldeck_core::config::platform::ResourceKind::Job
            })
            .map(|resource| {
                let id = resource.resource.id.as_str();
                MentionCandidate::new(
                    MentionKind::Job,
                    id.to_owned(),
                    resource.summary.as_str().to_owned(),
                    format!(
                        "{} · {}",
                        resource.resource.authority.as_str(),
                        resource.freshness.state.as_str()
                    ),
                    serde_json::json!({
                        "id": id,
                        "authority": resource.resource.authority.as_str(),
                        "freshness": resource.freshness.state.as_str(),
                        "observed_at": resource.freshness.observed_at.as_millis(),
                        "revision": resource.freshness.revision.get(),
                        "summary": resource.summary.as_str(),
                    }),
                )
                .keywords(resource.resource.authority.as_str())
            })
            .collect()
    }

    /// People.
    ///
    /// Only two sources carry the role information rule 1 needs
    /// (`docs/ai-mentions.md` § 5.3): the signed-in account itself, and the
    /// Manage directory. Participants scraped out of requests and tickets are
    /// deliberately **not** offered — an assignee could be a super-admin and
    /// nothing in that row would say so, and a picker that cannot prove a
    /// person is addressable must not offer them.
    fn person_candidates(&self) -> Vec<MentionCandidate> {
        let mut candidates: Vec<MentionCandidate> = Vec::new();
        if let Some(account) = self.app_config.account.as_ref() {
            let label = if account.name.trim().is_empty() {
                account.email.clone()
            } else {
                account.name.clone()
            };
            if !account.email.trim().is_empty() {
                candidates.push(
                    MentionCandidate::new(
                        MentionKind::Person,
                        account.email.to_lowercase(),
                        label,
                        t!(PersonRelation::SelfAccount.label_key()).to_string(),
                        serde_json::json!({
                            "email": account.email,
                            "name": account.name,
                            "relation": PersonRelation::SelfAccount,
                            "roles": account.roles,
                        }),
                    )
                    .keywords(account.email.clone()),
                );
            }
        }
        for person in &self.mention_people {
            let email = person.email.to_lowercase();
            if candidates
                .iter()
                .any(|candidate| candidate.id == email && candidate.kind == MentionKind::Person)
            {
                continue;
            }
            let relation = if person.is_support_agent() {
                PersonRelation::SupportAgent
            } else {
                PersonRelation::Member
            };
            candidates.push(
                MentionCandidate::new(
                    MentionKind::Person,
                    email,
                    person.display_name(),
                    t!(relation.label_key()).to_string(),
                    serde_json::json!({
                        "email": person.email,
                        "name": person.name,
                        "relation": relation,
                        "roles": person.roles,
                        "site": person.site_label,
                    }),
                )
                .site(person.site_id.clone(), person.site_label.clone())
                .keywords(person.email.clone()),
            );
        }
        candidates
    }

    // ---------------------------------------------------------------- people

    /// Refresh the people cache from Manage.
    ///
    /// Best-effort and quiet: the endpoint ships in a separate `bext` PR, so a
    /// 404 is the expected answer today and must not produce a toast in a
    /// picker the user just opened. Only a rejected token is worth surfacing,
    /// and that path already exists.
    pub(super) fn refresh_mention_people(&mut self, cx: &mut Context<Self>) {
        if !self.signed_in() {
            self.mention_people.clear();
            return;
        }
        let base = self.app_config.cloud_sync.base_url.clone();
        let token = self.app_config.cloud_sync.token.clone();
        let site = self.app_config.cloud_sync.active_site_id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(
                    async move { manage_directory::fetch_people(&base, &token, site.as_deref()) },
                )
                .await;
            let _ = this.update(cx, |workspace, cx| match result {
                Ok(people) => {
                    let changed = workspace.mention_people != people;
                    workspace.mention_people = people;
                    if changed {
                        workspace.refresh_mention_directory(cx);
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, "mentionable people unavailable");
                }
            });
        })
        .detach();
    }
}
