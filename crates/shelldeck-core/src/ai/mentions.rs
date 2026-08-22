//! `@` mentions — typed references to ShellDeck entities carried by a turn.
//!
//! A mention is not an attachment. Attachments are bytes and are not portable
//! across backends; a mention is structured text and therefore reaches every
//! provider identically, including the CLI backends invoked with no tools.
//! That is why the `@` path — not the `+` path — is the one that carries
//! application meaning.
//!
//! Everything here is pure: candidate scoping, fuzzy matching and draft
//! reconciliation are decided without a GPUI context so they can be tested
//! directly. Building the candidate list from live application state is the
//! Workspace's job (`shelldeck_ui::workspace::mentions`).
//!
//! Full contract: `docs/ai-mentions.md`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::cloud_account::AppMode;

/// Longest excerpt any single mention payload may carry, in characters.
/// Deliberately small: a mention identifies a record, it does not replace
/// fetching it.
pub const MENTION_DETAIL_MAX_CHARS: usize = 1_500;

/// How many candidates the picker will ever show at once. The directory itself
/// may be much larger; this bounds the rendered list, not the search.
pub const MENTION_PICKER_LIMIT: usize = 40;

/// What kind of ShellDeck entity a mention points at.
///
/// The catalogue is closed on purpose. Adding a variant means answering, in
/// `docs/ai-mentions.md`, what its payload is and who may see it — a kind with
/// no scoping rule is a leak waiting to happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MentionKind {
    /// An SSH connection profile.
    Host,
    /// An Inklura Manage site.
    Site,
    /// A port forward.
    Tunnel,
    /// A saved script.
    Script,
    /// A hosted request (issue).
    Request,
    /// A support ticket.
    Ticket,
    /// An open terminal session.
    Terminal,
    /// A file open in the editor.
    File,
    /// A Monique fleet runtime instance.
    Instance,
    /// A Monique fleet job.
    Job,
    /// A person the caller is allowed to address.
    Person,
}

impl MentionKind {
    /// Every kind, in picker section order: the entities a user reasons about
    /// most often come first.
    pub fn all() -> &'static [MentionKind] {
        &[
            MentionKind::Host,
            MentionKind::Request,
            MentionKind::Ticket,
            MentionKind::Terminal,
            MentionKind::Script,
            MentionKind::Site,
            MentionKind::Tunnel,
            MentionKind::File,
            MentionKind::Instance,
            MentionKind::Job,
            MentionKind::Person,
        ]
    }

    /// Stable machine token. Part of the wire format and of the searchable
    /// text, so `@host` narrows the picker to connections.
    pub fn token(self) -> &'static str {
        match self {
            MentionKind::Host => "host",
            MentionKind::Site => "site",
            MentionKind::Tunnel => "tunnel",
            MentionKind::Script => "script",
            MentionKind::Request => "request",
            MentionKind::Ticket => "ticket",
            MentionKind::Terminal => "terminal",
            MentionKind::File => "file",
            MentionKind::Instance => "instance",
            MentionKind::Job => "job",
            MentionKind::Person => "person",
        }
    }

    /// Bundled Lucide slug (`.agents/icons.md` — the subset is ~80 files, not
    /// all of Lucide; every slug here is verified present).
    pub fn icon(self) -> &'static str {
        match self {
            MentionKind::Host => "server",
            MentionKind::Site => "globe",
            MentionKind::Tunnel => "arrow-left-right",
            MentionKind::Script => "scroll-text",
            MentionKind::Request => "inbox",
            MentionKind::Ticket => "life-buoy",
            MentionKind::Terminal => "terminal",
            MentionKind::File => "file-text",
            MentionKind::Instance => "cpu",
            MentionKind::Job => "list-checks",
            MentionKind::Person => "user",
        }
    }

    /// i18n key for the picker section header.
    pub fn label_key(self) -> &'static str {
        match self {
            MentionKind::Host => "ai.mention.kind.host",
            MentionKind::Site => "ai.mention.kind.site",
            MentionKind::Tunnel => "ai.mention.kind.tunnel",
            MentionKind::Script => "ai.mention.kind.script",
            MentionKind::Request => "ai.mention.kind.request",
            MentionKind::Ticket => "ai.mention.kind.ticket",
            MentionKind::Terminal => "ai.mention.kind.terminal",
            MentionKind::File => "ai.mention.kind.file",
            MentionKind::Instance => "ai.mention.kind.instance",
            MentionKind::Job => "ai.mention.kind.job",
            MentionKind::Person => "ai.mention.kind.person",
        }
    }

    /// Lowest app mode that may reference this kind.
    ///
    /// Gating on the *effective mode* rather than the raw role bag is
    /// deliberate: the mode is the hat the user is currently wearing, and the
    /// assistant must not offer a surface the current mode does not show
    /// (`.agents/roles.md`).
    pub fn required_mode(self) -> AppMode {
        match self {
            MentionKind::Site | MentionKind::Request | MentionKind::Person => AppMode::User,
            MentionKind::Ticket => AppMode::Support,
            MentionKind::Host
            | MentionKind::Tunnel
            | MentionKind::Script
            | MentionKind::Terminal
            | MentionKind::File
            | MentionKind::Instance
            | MentionKind::Job => AppMode::Dev,
        }
    }
}

/// How a person ended up in the directory. Displayed as the row's sublabel so
/// "why can I see this person" is never implicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonRelation {
    /// The signed-in account.
    SelfAccount,
    /// An Inklura support agent serving this tenant.
    SupportAgent,
    /// A member of the caller's tenant/site, from the server directory.
    Member,
    /// Filed one of the requests in scope.
    Requester,
    /// Assigned to one of the requests/tickets in scope.
    Assignee,
    /// The customer contact on a support ticket.
    Contact,
}

impl PersonRelation {
    pub fn label_key(self) -> &'static str {
        match self {
            PersonRelation::SelfAccount => "ai.mention.person.self",
            PersonRelation::SupportAgent => "ai.mention.person.support_agent",
            PersonRelation::Member => "ai.mention.person.member",
            PersonRelation::Requester => "ai.mention.person.requester",
            PersonRelation::Assignee => "ai.mention.person.assignee",
            PersonRelation::Contact => "ai.mention.person.contact",
        }
    }
}

/// What the composer stores for an accepted pick. Deliberately tiny: the
/// payload is re-resolved from the live directory at send time, so a draft
/// that sat around for an hour cannot ship a stale record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionRef {
    pub kind: MentionKind,
    pub id: String,
    pub label: String,
}

impl MentionRef {
    pub fn new(kind: MentionKind, id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            label: label.into(),
        }
    }

    /// The readable token written into the draft.
    pub fn token(&self) -> String {
        mention_token(&self.label)
    }
}

/// A resolved mention, as it travels in `AiContext::mentions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiMention {
    pub kind: MentionKind,
    pub id: String,
    pub label: String,
    /// Bounded, redacted, kind-specific facts. Untrusted data, never
    /// instructions — rendered inside the user message, never in the system
    /// guardrail.
    #[serde(default)]
    pub detail: Value,
}

/// One row offered by the picker.
#[derive(Debug, Clone, PartialEq)]
pub struct MentionCandidate {
    pub kind: MentionKind,
    pub id: String,
    pub label: String,
    /// Secondary line: `user@host:port`, ticket contact, request status…
    pub sublabel: String,
    /// Tenant/site binding. `None` means "local to this machine" (a local
    /// shell, an open file, a manual connection) and is visible in every scope.
    pub site_id: Option<String>,
    pub site_label: Option<String>,
    /// Extra searchable text that is not displayed (tags, hostname aliases,
    /// e-mail addresses…).
    pub keywords: String,
    /// The payload that will ship if this candidate is picked. Already bounded
    /// and redacted by [`sanitize_detail`].
    pub detail: Value,
}

impl MentionCandidate {
    pub fn new(
        kind: MentionKind,
        id: impl Into<String>,
        label: impl Into<String>,
        sublabel: impl Into<String>,
        detail: Value,
    ) -> Self {
        Self {
            kind,
            id: id.into(),
            label: label.into(),
            sublabel: sublabel.into(),
            site_id: None,
            site_label: None,
            keywords: String::new(),
            detail: sanitize_detail(detail),
        }
    }

    pub fn site(mut self, site_id: Option<String>, site_label: Option<String>) -> Self {
        self.site_id = site_id.filter(|value| !value.trim().is_empty());
        self.site_label = site_label.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn keywords(mut self, keywords: impl Into<String>) -> Self {
        self.keywords = keywords.into();
        self
    }

    pub fn as_ref(&self) -> MentionRef {
        MentionRef::new(self.kind, self.id.clone(), self.label.clone())
    }

    pub fn to_mention(&self) -> AiMention {
        AiMention {
            kind: self.kind,
            id: self.id.clone(),
            label: self.label.clone(),
            detail: self.detail.clone(),
        }
    }
}

/// Who is asking. Everything the two gates in `docs/ai-mentions.md` § 5 need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionScope {
    pub signed_in: bool,
    pub mode: AppMode,
    pub is_superadmin: bool,
    pub is_inklura_support: bool,
    /// `cloud_sync.active_site_id`, as a string for comparison with the
    /// server's string ids.
    pub active_site_id: Option<String>,
}

impl Default for MentionScope {
    fn default() -> Self {
        Self {
            signed_in: false,
            mode: AppMode::User,
            is_superadmin: false,
            is_inklura_support: false,
            active_site_id: None,
        }
    }
}

impl MentionScope {
    /// Inklura staff. Staff work cross-site by definition, so they are the
    /// only callers who ever see a row outside the active site.
    pub fn is_staff(&self) -> bool {
        self.is_superadmin || self.is_inklura_support
    }

    /// Kind gate — `docs/ai-mentions.md` § 5.1.
    pub fn allows_kind(&self, kind: MentionKind) -> bool {
        if !self.signed_in {
            return false;
        }
        match kind.required_mode() {
            AppMode::User => true,
            AppMode::Support => matches!(self.mode, AppMode::Support | AppMode::Dev),
            AppMode::Dev => self.mode == AppMode::Dev,
        }
    }

    /// Row gate — `docs/ai-mentions.md` § 5.2.
    ///
    /// An unbound candidate is local to this machine and belongs to no tenant,
    /// so it is always in scope. A bound candidate must match the active site
    /// unless the caller is staff.
    pub fn allows_candidate(&self, candidate: &MentionCandidate) -> bool {
        if !self.allows_kind(candidate.kind) {
            return false;
        }
        let Some(site) = candidate.site_id.as_deref() else {
            return true;
        };
        if self.is_staff() {
            return true;
        }
        self.active_site_id
            .as_deref()
            .is_some_and(|active| active.eq_ignore_ascii_case(site))
    }
}

/// Apply both gates. The Workspace calls this when the directory is built, and
/// the assistant calls it again at send time — a draft can outlive a site
/// switch, and a mention that left the caller's scope must not travel.
pub fn scoped_candidates(
    scope: &MentionScope,
    candidates: impl IntoIterator<Item = MentionCandidate>,
) -> Vec<MentionCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| scope.allows_candidate(candidate))
        .collect()
}

/// Role names that must never be addressable from the composer, in any of the
/// spellings CM has used.
const NEVER_MENTIONABLE_ROLES: [&str; 3] = ["superadmin", "super_admin", "super-admin"];

/// Whether a person may be offered at all.
///
/// `server_mentionable` is the directory's own verdict. It is honoured, but it
/// is not trusted alone: a client that relies on one flag from one server is
/// one deploy bug away from leaking platform-staff identities into a
/// customer-facing picker, so the role bag is re-checked here.
pub fn person_is_mentionable(roles: &[String], server_mentionable: Option<bool>) -> bool {
    if server_mentionable == Some(false) {
        return false;
    }
    !roles.iter().any(|role| {
        let normalized = role.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        NEVER_MENTIONABLE_ROLES
            .iter()
            .any(|denied| normalized == denied.replace('-', "_"))
    })
}

/// The readable token written into the draft for `label`.
pub fn mention_token(label: &str) -> String {
    format!("@{}", label.trim())
}

/// Fuzzy subsequence match, same spirit as the command palette's matcher:
/// every character of `needle` must appear in order in `haystack`.
fn subsequence_match(haystack: &str, needle: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|needle_char| chars.any(|hay_char| hay_char == needle_char))
}

/// Rank: lower is better. `None` when the candidate does not match at all.
fn match_rank(candidate: &MentionCandidate, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(3);
    }
    let label = candidate.label.to_lowercase();
    let sublabel = candidate.sublabel.to_lowercase();
    let keywords = candidate.keywords.to_lowercase();
    let token = candidate.kind.token();
    if label.starts_with(query) || token.starts_with(query) {
        return Some(0);
    }
    if label.contains(query) {
        return Some(1);
    }
    if sublabel.contains(query) || keywords.contains(query) {
        return Some(2);
    }
    if subsequence_match(&label, query) {
        return Some(3);
    }
    None
}

/// Filter and rank the directory for a picker query.
///
/// Returned in (rank, kind order, label) order so the list is stable between
/// keystrokes — a picker whose rows reshuffle under the cursor is a picker
/// that gets the wrong entity selected.
pub fn filter_mention_candidates<'a>(
    candidates: &'a [MentionCandidate],
    query: &str,
    limit: usize,
) -> Vec<&'a MentionCandidate> {
    let query = query.trim().trim_start_matches('@').to_lowercase();
    let mut scored: Vec<(u8, usize, &'a MentionCandidate)> = candidates
        .iter()
        .filter_map(|candidate| {
            let rank = match_rank(candidate, &query)?;
            let kind_order = MentionKind::all()
                .iter()
                .position(|kind| *kind == candidate.kind)
                .unwrap_or(usize::MAX);
            Some((rank, kind_order, candidate))
        })
        .collect();
    scored.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(left.1.cmp(&right.1))
            .then_with(|| {
                left.2
                    .label
                    .to_lowercase()
                    .cmp(&right.2.label.to_lowercase())
            })
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, candidate)| candidate)
        .collect()
}

/// Insert `label`'s token at `caret`, replacing an in-progress `@query` when
/// the caret sits right after one.
///
/// Returns the new draft and the new caret offset, both in bytes.
pub fn insert_mention(draft: &str, caret: usize, label: &str) -> (String, usize) {
    let caret = clamp_to_char_boundary(draft, caret);
    let start = match mention_query_at_caret(draft, caret) {
        Some((start, _)) => start,
        None => caret,
    };
    let token = mention_token(label);
    let needs_leading_space = start > 0
        && !draft[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
    let trailing = &draft[caret..];
    let needs_trailing_space = !trailing.starts_with(' ');

    let mut output = String::with_capacity(draft.len() + token.len() + 2);
    output.push_str(&draft[..start]);
    if needs_leading_space {
        output.push(' ');
    }
    output.push_str(&token);
    if needs_trailing_space {
        output.push(' ');
    }
    let new_caret = output.len();
    output.push_str(trailing);
    (output, new_caret)
}

/// The `@query` the caret is currently inside, if any.
///
/// Returns the byte offset of the `@` and the query typed after it. Used to
/// open the picker while typing and to replace the partial token on accept.
/// A query is only recognised when the `@` starts a word — `user@host` is an
/// e-mail address, not a mention.
pub fn mention_query_at_caret(draft: &str, caret: usize) -> Option<(usize, String)> {
    let caret = clamp_to_char_boundary(draft, caret);
    let head = &draft[..caret];
    let at = head.rfind('@')?;
    if at > 0 {
        let previous = head[..at].chars().next_back()?;
        if !previous.is_whitespace() {
            return None;
        }
    }
    let query = &head[at + 1..];
    // A mention query stops at a newline; a label may contain spaces, so a
    // space alone does not end it — the picker keeps matching until the user
    // types something that matches nothing.
    if query.contains('\n') {
        return None;
    }
    Some((at, query.to_string()))
}

/// Drop refs whose token no longer appears in the draft.
///
/// The draft is the source of truth: deleting the text of a mention deletes the
/// mention. Repeated labels are matched by occurrence count, so two hosts with
/// the same alias behave predictably.
pub fn reconcile_mentions(draft: &str, refs: &[MentionRef]) -> Vec<MentionRef> {
    let mut kept: Vec<MentionRef> = Vec::with_capacity(refs.len());
    for candidate in refs {
        let token = candidate.token();
        if token.len() <= 1 {
            continue;
        }
        let available = draft.matches(token.as_str()).count();
        let already = kept
            .iter()
            .filter(|existing| existing.label == candidate.label)
            .count();
        if already < available {
            kept.push(candidate.clone());
        }
    }
    kept
}

/// Remove one occurrence of `label`'s token from the draft, collapsing the
/// space it leaves behind. Used when a chip's `×` is clicked.
pub fn remove_mention_token(draft: &str, label: &str) -> String {
    let token = mention_token(label);
    let Some(start) = draft.find(token.as_str()) else {
        return draft.to_string();
    };
    let mut end = start + token.len();
    if draft[end..].starts_with(' ') {
        end += 1;
    } else if start > 0 && draft[..start].ends_with(' ') {
        return format!("{}{}", &draft[..start - 1], &draft[end..]);
    }
    format!("{}{}", &draft[..start], &draft[end..])
}

/// Byte ranges of the `@Label` tokens present in `text`, for the given labels.
///
/// This is what lets a mention be *seen* — coloured in the composer while it is
/// typed, and in the thread once it is sent. Three rules make it predictable:
///
/// * **Longest label first.** With both `web` and `web-01` in play, `@web-01`
///   must colour as one mention, not as `@web` followed by loose text.
/// * **No overlaps.** A byte belongs to at most one mention.
/// * **Word boundary before the `@`**, exactly like [`mention_query_at_caret`],
///   so an e-mail address is never painted as a mention.
///
/// Ranges come back sorted by position, which is what a run-splitter needs.
pub fn mention_spans(text: &str, labels: &[String]) -> Vec<std::ops::Range<usize>> {
    let mut ordered: Vec<&String> = labels.iter().filter(|label| !label.is_empty()).collect();
    ordered.sort_by_key(|label| std::cmp::Reverse(label.len()));

    let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
    for label in ordered {
        let token = mention_token(label);
        if token.len() <= 1 {
            continue;
        }
        let mut from = 0usize;
        while let Some(found) = text[from..].find(token.as_str()) {
            let start = from + found;
            let end = start + token.len();
            from = end;
            if start > 0
                && !text[..start]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            {
                continue;
            }
            if spans
                .iter()
                .any(|existing| start < existing.end && existing.start < end)
            {
                continue;
            }
            spans.push(start..end);
        }
    }
    spans.sort_by_key(|span| span.start);
    spans
}

fn clamp_to_char_boundary(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while offset > 0 && !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Redact and bound a payload before it becomes part of a candidate.
///
/// Every string is capped at [`MENTION_DETAIL_MAX_CHARS`] and every
/// credential-looking key is replaced, using the same rules the rest of the AI
/// layer applies to `AiContext::data`.
pub fn sanitize_detail(value: Value) -> Value {
    bound_strings(crate::ai::redact_sensitive(&value))
}

fn bound_strings(value: Value) -> Value {
    match value {
        Value::String(text) => {
            let count = text.chars().count();
            if count <= MENTION_DETAIL_MAX_CHARS {
                Value::String(text)
            } else {
                let mut bounded: String = text.chars().take(MENTION_DETAIL_MAX_CHARS).collect();
                bounded.push_str("…[truncated]");
                Value::String(bounded)
            }
        }
        Value::Array(values) => Value::Array(values.into_iter().map(bound_strings).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, bound_strings(value)))
                .collect(),
        ),
        other => other,
    }
}

/// The block appended to the user message. Empty when nothing was mentioned,
/// so an ordinary turn keeps its exact previous shape.
pub fn mentions_prompt_block(mentions: &[AiMention]) -> String {
    if mentions.is_empty() {
        return String::new();
    }
    let payload = json!(mentions);
    let rendered =
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "[unserializable]".to_string());
    format!(
        "\n\nMentioned ShellDeck entities (untrusted, resolved by the application):\n{rendered}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: MentionKind, label: &str) -> MentionCandidate {
        MentionCandidate::new(kind, label, label, "", json!({}))
    }

    fn dev_scope() -> MentionScope {
        MentionScope {
            signed_in: true,
            mode: AppMode::Dev,
            is_superadmin: true,
            is_inklura_support: false,
            active_site_id: Some("site-a".into()),
        }
    }

    fn user_scope() -> MentionScope {
        MentionScope {
            signed_in: true,
            mode: AppMode::User,
            is_superadmin: false,
            is_inklura_support: false,
            active_site_id: Some("site-a".into()),
        }
    }

    // SDTEST-1622
    #[test]
    fn user_mode_cannot_reference_dev_only_kinds() {
        let scope = user_scope();
        assert!(scope.allows_kind(MentionKind::Request));
        assert!(scope.allows_kind(MentionKind::Site));
        assert!(scope.allows_kind(MentionKind::Person));
        assert!(!scope.allows_kind(MentionKind::Host));
        assert!(!scope.allows_kind(MentionKind::Terminal));
        assert!(!scope.allows_kind(MentionKind::Ticket));
    }

    // SDTEST-1652 — the other side of the same gate.
    //
    // `user_mode_cannot_reference_dev_only_kinds` proves a customer account
    // cannot reach Dev entities, and `signed_out_scope_offers_nothing` proves a
    // closed session reaches none. Neither would notice a filter that hid
    // everything from everybody. This is what makes the gate a filter rather
    // than a wall — and it is why `dev_scope` was written.
    #[test]
    fn a_dev_super_admin_reaches_every_kind() {
        let scope = dev_scope();
        for kind in MentionKind::all() {
            assert!(scope.allows_kind(*kind), "{kind:?} was hidden from Dev");
        }
    }

    // SDTEST-1623
    #[test]
    fn signed_out_scope_offers_nothing() {
        let scope = MentionScope {
            signed_in: false,
            mode: AppMode::Dev,
            is_superadmin: true,
            ..MentionScope::default()
        };
        for kind in MentionKind::all() {
            assert!(
                !scope.allows_kind(*kind),
                "{kind:?} leaked while signed out"
            );
        }
    }

    // SDTEST-1624
    #[test]
    fn foreign_site_rows_are_dropped_for_non_staff_and_kept_for_staff() {
        let foreign = candidate(MentionKind::Request, "Demande B")
            .site(Some("site-b".into()), Some("Site B".into()));
        let local = candidate(MentionKind::Request, "Demande A")
            .site(Some("site-a".into()), Some("Site A".into()));
        let unbound = candidate(MentionKind::Request, "Demande locale");

        let user = user_scope();
        assert!(!user.allows_candidate(&foreign));
        assert!(user.allows_candidate(&local));
        assert!(user.allows_candidate(&unbound));

        let staff = MentionScope {
            is_inklura_support: true,
            mode: AppMode::Support,
            ..user_scope()
        };
        assert!(staff.allows_candidate(&foreign));
    }

    // SDTEST-1625
    #[test]
    fn scoped_candidates_filters_the_whole_directory() {
        let scope = user_scope();
        let kept = scoped_candidates(
            &scope,
            vec![
                candidate(MentionKind::Host, "prod-web-01"),
                candidate(MentionKind::Request, "Demande A")
                    .site(Some("site-a".into()), Some("Site A".into())),
                candidate(MentionKind::Request, "Demande B")
                    .site(Some("site-b".into()), Some("Site B".into())),
            ],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].label, "Demande A");
    }

    // SDTEST-1626
    #[test]
    fn super_admins_are_never_mentionable() {
        assert!(!person_is_mentionable(
            &["admin".into(), "superadmin".into()],
            Some(true)
        ));
        assert!(!person_is_mentionable(&["Super-Admin".into()], None));
        assert!(!person_is_mentionable(&["super_admin".into()], Some(true)));
        assert!(person_is_mentionable(&["inklura_support".into()], None));
        assert!(!person_is_mentionable(
            &["inklura_support".into()],
            Some(false)
        ));
    }

    // SDTEST-1627
    #[test]
    fn caret_query_ignores_email_addresses() {
        assert_eq!(
            mention_query_at_caret("redémarre @pro", "redémarre @pro".len()),
            Some(("redémarre ".len(), "pro".to_string()))
        );
        assert_eq!(mention_query_at_caret("karim@webdesign29.net", 20), None);
        assert_eq!(mention_query_at_caret("rien ici", 8), None);
    }

    // SDTEST-1628
    #[test]
    fn inserting_replaces_the_partial_query_and_spaces_the_token() {
        let (draft, caret) =
            insert_mention("redémarre @pro", "redémarre @pro".len(), "prod-web-01");
        assert_eq!(draft, "redémarre @prod-web-01 ");
        assert_eq!(caret, draft.len());

        let (draft, caret) = insert_mention("redémarre", "redémarre".len(), "prod-web-01");
        assert_eq!(draft, "redémarre @prod-web-01 ");
        assert_eq!(caret, draft.len());

        // The caret sits after "a ", and the text already continues with a
        // space — the token must not add a second one.
        let (draft, _) = insert_mention("a  b", 2, "hôte");
        assert_eq!(draft, "a @hôte b");
    }

    // SDTEST-1629
    #[test]
    fn deleting_the_text_deletes_the_mention() {
        let refs = vec![
            MentionRef::new(MentionKind::Host, "1", "prod-web-01"),
            MentionRef::new(MentionKind::Host, "2", "prod-db-01"),
        ];
        let kept = reconcile_mentions("compare @prod-web-01 et @prod-db-01", &refs);
        assert_eq!(kept.len(), 2);

        let kept = reconcile_mentions("compare @prod-web-01 et rien", &refs);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "1");

        let kept = reconcile_mentions("plus aucune référence", &refs);
        assert!(kept.is_empty());
    }

    // SDTEST-1630
    #[test]
    fn repeated_labels_are_matched_by_occurrence_count() {
        let refs = vec![
            MentionRef::new(MentionKind::Host, "1", "web"),
            MentionRef::new(MentionKind::Host, "2", "web"),
        ];
        assert_eq!(reconcile_mentions("@web une fois", &refs).len(), 1);
        assert_eq!(reconcile_mentions("@web et @web", &refs).len(), 2);
    }

    // SDTEST-1631
    #[test]
    fn removing_a_chip_removes_one_token_and_its_space() {
        assert_eq!(
            remove_mention_token("compare @web et @db", "web"),
            "compare et @db"
        );
        assert_eq!(remove_mention_token("fin @web", "web"), "fin");
        assert_eq!(remove_mention_token("aucun token", "web"), "aucun token");
    }

    // SDTEST-1632
    #[test]
    fn picker_ranks_prefix_matches_first_and_is_stable() {
        let candidates = vec![
            MentionCandidate::new(MentionKind::Host, "1", "staging-web", "u@s", json!({})),
            MentionCandidate::new(MentionKind::Host, "2", "web-prod", "u@w", json!({})),
            MentionCandidate::new(MentionKind::Request, "3", "Panne web", "open", json!({}))
                .keywords("incident"),
        ];
        let matches = filter_mention_candidates(&candidates, "web", 10);
        assert_eq!(matches[0].label, "web-prod");
        assert_eq!(
            filter_mention_candidates(&candidates, "web", 10)[0].label,
            "web-prod"
        );
        assert_eq!(matches.len(), 3);

        let by_kind = filter_mention_candidates(&candidates, "host", 10);
        assert_eq!(by_kind.len(), 2);
        assert!(by_kind.iter().all(|row| row.kind == MentionKind::Host));
    }

    // SDTEST-1633
    #[test]
    fn candidate_payloads_are_redacted_and_bounded() {
        let long = "x".repeat(MENTION_DETAIL_MAX_CHARS + 500);
        let candidate = MentionCandidate::new(
            MentionKind::Host,
            "1",
            "prod",
            "",
            json!({ "password": "hunter2", "body": long }),
        );
        assert_eq!(candidate.detail["password"], "[REDACTED]");
        let body = candidate.detail["body"].as_str().unwrap();
        assert!(body.ends_with("…[truncated]"));
        assert!(body.chars().count() <= MENTION_DETAIL_MAX_CHARS + 12);
    }

    // SDTEST-1634
    #[test]
    fn prompt_block_is_empty_without_mentions() {
        assert!(mentions_prompt_block(&[]).is_empty());
        let block = mentions_prompt_block(&[AiMention {
            kind: MentionKind::Host,
            id: "1".into(),
            label: "prod".into(),
            detail: json!({ "hostname": "10.0.0.1" }),
        }]);
        assert!(block.contains("untrusted"));
        assert!(block.contains("10.0.0.1"));
    }

    // SDTEST-1635
    #[test]
    fn every_kind_has_a_bundled_icon_and_a_distinct_token() {
        let mut tokens: Vec<&str> = MentionKind::all().iter().map(|kind| kind.token()).collect();
        tokens.sort_unstable();
        let count = tokens.len();
        tokens.dedup();
        assert_eq!(tokens.len(), count, "mention tokens must be unique");
        assert_eq!(MentionKind::all().len(), 11);
        for kind in MentionKind::all() {
            assert!(!kind.icon().is_empty());
            assert!(kind.label_key().starts_with("ai.mention.kind."));
        }
    }

    // SDTEST-1655
    #[test]
    fn spans_cover_each_token_once_and_prefer_the_longest_label() {
        let labels = vec!["web".to_string(), "web-01".to_string()];
        let text = "compare @web-01 et @web";
        let spans = mention_spans(text, &labels);
        let painted: Vec<&str> = spans.iter().map(|s| &text[s.clone()]).collect();
        assert_eq!(painted, vec!["@web-01", "@web"]);
        // Sorted by position, so a run splitter can walk them in one pass.
        assert!(spans.windows(2).all(|w| w[0].end <= w[1].start));
    }

    // SDTEST-1656
    #[test]
    fn spans_ignore_an_email_and_an_unmentioned_label() {
        let labels = vec!["prod".to_string()];
        assert!(mention_spans("écris à karim@prod.fr", &labels).is_empty());
        assert!(mention_spans("aucun jeton ici", &labels).is_empty());
        assert_eq!(mention_spans("sur @prod", &labels).len(), 1);
    }

    // SDTEST-1657
    #[test]
    fn spans_are_byte_exact_on_accented_text() {
        let labels = vec!["hôte-é".to_string()];
        let text = "redémarre @hôte-é maintenant";
        let spans = mention_spans(text, &labels);
        assert_eq!(spans.len(), 1);
        assert_eq!(&text[spans[0].clone()], "@hôte-é");
        // Every boundary must be a char boundary or shaping panics downstream.
        assert!(text.is_char_boundary(spans[0].start) && text.is_char_boundary(spans[0].end));
    }
}
