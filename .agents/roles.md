# Roles & App modes

ShellDeck's role model reads from **Company Manager (CM)** — the identity
backend shared by every 1clic.pro app (Manage, ShellDeck, Ads, …). A user
carries a **bag of role names** in CM; permissions attached to each role
are edited via the Manage `role-permissions` matrix
(`permissions.updateRolePermissions`).

The client displays the full role bag on the User → Mes informations tab
and uses a small set of *convenience* predicates (super-admin, admin) to
gate the app-mode switcher. **The bag is the truth**; the predicates are
just shortcuts.

## Where the role bag comes from

- `bext/sites/shared/manage/src/lib/session.ts` → `roleBag(user)`
  merges `user.role` (possibly `;`-joined), `user.roleNames[]`, and
  `user.roles[]` from CM, lowercases + trims + dedupes.
- The ShellDeck token snapshots the bag at mint time
  (`ShelldeckToken.user_roles: string[] | null` — added in the "roles
  surfacing" PR).
- `/auth?action=whoami` and `/auth {action:"login"}` return `user.roles:
  string[]` alongside `is_superadmin` for legacy callers.

## Convenience predicates → App modes

The app-mode switcher only needs three coarse buckets; those buckets are
name-based, not permission-based (until we surface a permission catalog
on the client).

| Bucket | Server predicate | Server role names matched | Allowed AppModes |
|--------|------------------|---------------------------|-------------------|
| Regular user | neither | anything else | **User** only |
| Admin (`isManageAdmin`) | `roleBag` intersect ≠ ∅ | admin, owner, administrator, tenant_admin | **User + Support** |
| Super-admin (`isSuperAdmin`) | `role` starts with | superadmin, super_admin, super-admin | **User + Support + Dev** |

Super-admin implies admin (short-circuit in `isManageAdmin`). Custom
roles the tenant admin defines in Manage (`content_editor`,
`customer_service`, …) are **surfaced in the UI** but do not currently
elevate the app mode — extend the predicates above if a new role needs
Support/Dev access.

## Not signed in → welcome landing (no guest mode)

**ShellDeck requires an account to launch.** Historically the app ran a
"classic Dev" experience for logged-out users (full sidebar, terminals,
SSH — no cloud sync); that path is **retired**.

- `Workspace::show_welcome() == !signed_in()` intercepts every render
  when there is no session and drops the user on `render_welcome_screen`
  (brand mark + "Se connecter à Inklura Manage" primary CTA + "Créer un
  compte sur Manage" secondary CTA → opens the sign-up page in the
  system browser).
- `AppMode::resolve_effective(signed_in = false, …)` returns `User` as a
  defensive fallback — the welcome intercepts the render, but if any
  code path reaches the mode `match` without a session, **User** is the
  only acceptable answer. `Dev` is super-admin-only and must never
  render for anyone else.
- **No `welcome_bypass` / guest / standalone flag exists.** A returning
  user always sees the welcome after logout. Sign-in is the only way
  past.
- **Dev mode is super-admin-only, full stop.** Not a fallback, not a
  guest experience, not a preview — the switcher never surfaces it for
  admins or regular users, and `resolve_effective` never returns it for
  a non-super-admin session.

## Client-side wiring

- `AccountInfo.roles: Vec<String>` — the full bag, persisted in
  `~/.local/share/ShellDeck/shelldeck.toml` so the User → Mes
  informations tab can render offline.
- `AccountInfo.is_superadmin: bool` + `AccountInfo.is_admin: bool` —
  convenience flags derived server-side and delivered with the token.
- `AppMode::can_switch(signed_in, is_admin, is_superadmin)` — mirrors
  the table above. Non-super-admin non-admin is forced to User; admin
  gets User + Support; super-admin gets everything.
- `Workspace::effective_mode()` clamps the persisted mode against the
  current bucket — a downgrade at whoami time silently forces the
  user back to a valid mode.
- **Mes informations tab (Infos)** must list every role in the bag as
  chips/badges — that's how the operator sees what CM actually sends,
  and how a tenant admin verifies their custom role setup landed.

## Non-negotiables

- **Never** invent a role name client-side. Roles come from CM. If the
  bag is empty, the user is a regular user — do not fill in
  placeholders.
- **Never** display a mode the caller can't reach (see the mapping
  table above). An empty/error Support tab is worse than no tab.
- **Never** persist a role decision. Re-read the bag from
  `last_whoami` (or refresh it) — a logout / re-login flushes the
  bucket cleanly.
- **The bag is display-only for custom roles right now.** Any new
  permission-gated behaviour must land as an explicit predicate
  (`is_content_editor`, etc.) on both server and client, matching the
  convenience-predicate pattern — never conditionally render based on
  a raw role name string in the middle of a view function.
- **Client mode gates remain UX politeness.** The server is the source
  of truth for what any single request may do; the client just avoids
  surfacing dead paths.
