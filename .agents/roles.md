# Roles & App modes

## The role system — single source, two consumption paths

Company Manager (CM) has **one** role-permission system, shared by every
1clic.pro app (Manage, ShellDeck, Ads, …). It's a single graph:

  * **Roles** are named entities on CM. A user carries a **bag of role
    names** (`user.role` — possibly `;`-joined, plus `roleNames[]` and
    `roles[]`), merged and de-duped by `roleBag(user)` in
    `bext/sites/shared/manage/src/lib/session.ts`.
  * **Permissions** are attached to each role via
    `permissions.updateRolePermissions` (Manage's role-permissions
    matrix). New roles are created with `permissions.createRole
    { name, label, color? }` (gate `roles:create`).
  * A user's effective permission set = union of the permissions of
    every role in the bag.

Server gates consume this graph in **two equivalent ways**:

  * `permissionProtectedProcedure(["stock:read"])` — "does the user's
    effective permission set include `stock:read`?" — granular, based
    on the graph.
  * `isSuperAdmin(user)` / `isManageAdmin(user)` — "does the user's
    role bag include the name `superadmin` / `admin` / …?" — a
    shortcut heuristic that ignores permissions and just checks names.

Both hit the same underlying data. There's no separate "permissions
system" and "roles system" — one graph, two consumption paths.

## The problem with the name-based predicates

`isManageAdmin(user)` matches on `{admin, owner, administrator,
tenant_admin}`. These names lump together **very different audiences**:

  * `superadmin` — Inklura platform staff (dev / SRE / ops)
  * `admin` — ambiguous (could be Inklura staff *or* a customer admin)
  * `owner` — usually the owner of a business entity — customer-side
  * `tenant_admin` — **admin of a customer tenant** — a customer role

Giving `tenant_admin` a Support-mode surface makes no sense: a customer
admin is the person Support helps, not a member of Support. The current
`isManageAdmin` → Support mapping was a first-pass approximation and is
wrong for the product.

## The correct model for ShellDeck modes

ShellDeck's three modes map to three audiences:

  * **User mode** — any authenticated user (customer or staff). The
    client-facing view: Mes sites, Mes demandes, Ouvrir Manage.
  * **Support mode** — **Inklura support staff only**. The tickets /
    requests triage surface (the person here is *helping* customers,
    not asking for help).
  * **Dev mode** — **Inklura platform staff only** (SSH, terminals,
    port forwards, sites, editor, JeanClaude, Fleet, bext Cloud).

The gate needs to distinguish **Inklura staff** from **customers**.
CM's existing name predicates don't — they mix the two. So ShellDeck
needs its own dedicated role names on CM:

| ShellDeck mode | CM role required | Server predicate |
|----------------|------------------|------------------|
| User           | (any authenticated user) | `signed_in()` |
| Support        | `inklura_support`, `superadmin` | `isInkluraSupport(user)` |
| Dev            | `superadmin` (existing) | `isSuperAdmin(user)` |

Rationale:

  * Reuse `superadmin` for Dev — the existing platform-staff role
    already gates the sensitive stuff.
  * A **new dedicated `inklura_support` role** for Support access.
    Attributed only to internal support agents. `superadmin` implies
    it (short-circuit in the predicate).
  * `admin` / `owner` / `tenant_admin` / `administrator` still exist
    on CM and still gate their usual admin surfaces in Manage — they
    just no longer unlock any ShellDeck surface beyond User.

## Client-side wiring

  * `AccountInfo` fields (persisted in `~/.local/share/ShellDeck/shelldeck.toml`):
    * `is_superadmin: bool` — Dev + Support (inclusive)
    * `is_inklura_support: bool` — Support only (excludes Dev). True when
      the role bag matches `isInkluraSupport(user)` server-side.
    * `is_admin: bool` — kept for Manage-side gates (invite links,
      admin console), **no longer used** for ShellDeck mode gating.
    * `roles: Vec<String>` — full CM role bag, displayed as badges in
      User → Mes informations tab. Custom roles included.
  * `AppMode::can_switch(signed_in, is_inklura_support, is_superadmin)` —
    signed-in super-admins OR inklura-support may switch.
  * `AppMode::resolve_effective(signed_in, is_inklura_support,
    is_superadmin, persisted)` — super-admin gets persisted; inklura-
    support gets persisted clamped to {User, Support}; regular
    (including customer admins) gets User forced.
  * **Mes informations tab** displays every role from the bag as
    primary-tinted badges. `is_inklura_support=true` shows an "Inklura
    Support" badge distinct from CM roles.

## Non-negotiables

  * **Never** infer staff status from generic admin roles
    (`admin`/`owner`/`tenant_admin`). Only the dedicated
    `inklura_support` and `superadmin` names unlock ShellDeck surfaces
    beyond User.
  * **Never** invent role names client-side. Roles come from CM. If a
    role signal is absent on the token, treat the user as regular.
  * **Never** display a mode the caller can't reach. Empty / erroring
    Support tab is worse than no tab.
  * **Never** persist role decisions. Re-read the bag from `last_whoami`
    or refresh it — logout / re-login is the only clean way to change
    tier client-side.
  * **The bag drives display; predicates drive gating.** Custom CM
    roles show up as badges without ceremony, but never magically
    unlock a mode — every gate is a spelled-out predicate on both
    server and client.

## Not signed in → welcome landing (no guest mode)

ShellDeck requires an account to launch. Historically the app ran a
"classic Dev" fallback for logged-out users; that path is retired.

  * `Workspace::show_welcome() == !signed_in()` intercepts every render
    without a session and drops the user on `render_welcome_screen`
    (brand mark + "Se connecter à Inklura Manage" primary CTA + "Créer
    un compte sur Manage" secondary CTA → opens sign-up in the system
    browser).
  * `AppMode::resolve_effective(signed_in = false, …)` returns `User`
    as a defensive fallback. Dev is super-admin-only and must never
    render for anyone else — not even as a fallback.
  * No `welcome_bypass` / guest / standalone flag exists.

## Rollout status (2026-07-13)

  * ✅ **PR bext #33** (soft-delete + list filters on issues) — merged.
  * ✅ **PR bext #37** (`is_admin` + `user_roles` on ShellDeck tokens) —
    merged, awaiting deploy on `manage.inklura.fr`.
  * ✅ **PR bext #38** (`inklura_support` role + `is_inklura_support`
    field on tokens/whoami/login) — merged, awaiting deploy.
  * ✅ **CM role `inklura_support`** created global (tenantId NULL),
    attributed to `karim+admin@webdesign29.net` for testing. Not the
    default `permissions.createRole` path — that proc requires a
    tenantId and would have scoped the role to a customer tenant;
    created directly in DB like the existing global roles
    (`superadmin`, `admin`, `dev`).
  * ✅ **Client-side gate switched from `is_admin` to
    `is_inklura_support`** — done. Compatible with all three server
    states (legacy pre-#37, post-#37 pre-#38, post-#38 target) via
    `#[serde(default)]`.
  * 🟡 **Backport to served tree** — `feat/paillard-mobilier-2883` is
    the tree serving prod and predates #37. Two commits (#37 + #38)
    to cherry-pick onto it before the 3-tier gate becomes live-testable.

## Post-deploy testing

Once `feat/paillard-mobilier-2883` carries #37 + #38 and is deployed:

  * Login `karim+user@webdesign29.net` (rôle CM `admin` seul) →
    User only, no switcher.
  * Login `karim+admin@webdesign29.net` (rôles `admin` + `inklura_support`)
    → User + Support, switcher visible.
  * Login super-admin (existing) → User + Support + Dev.

The Mes informations tab surfaces every CM role from the bag as a
badge; the role field shows the top tier (Super-admin → Inklura
Support → Admin → Utilisateur).
