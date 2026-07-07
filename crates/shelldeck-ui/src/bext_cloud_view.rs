//! bext Cloud (Dev mode) — the hosted control plane (cloud.bext.dev) plus a
//! single-bext-instance manager (the loopback site SDK).
//!
//! Two tabs:
//!   - **Cloud**: connect (browser CLI flow), the sites list (create / go-live /
//!     config / destroy), a dashboard stat strip, and (super-admin) the known
//!     bext instances.
//!   - **Instance**: manage the sites on one bext box directly via its
//!     `/__bext/sdk/site/*` SDK (target base URL + app-id).
//!
//! Pure renderer: the workspace does all I/O and feeds state via setters.

use crate::scale::px;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::config::bext_cloud::{
    CloudInstance, CloudSite, CloudStats, CloudUser, SitesResponse,
};
use shelldeck_core::config::bext_instance::InstanceSite;

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BextTab {
    Cloud,
    Instance,
}

/// Which composer group `Input::on_enter` should submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Site,
    Instance,
    InstanceRefresh,
}

#[derive(Debug, Clone)]
pub enum BextViewEvent {
    Connect,
    Disconnect,
    RefreshCloud,
    CreateSite {
        name: String,
        title: String,
    },
    SiteAction {
        slug: String,
        action: String,
    },
    OpenSite(String),
    RefreshInstance {
        base: String,
        app_id: String,
    },
    InstanceCreate {
        base: String,
        app_id: String,
        slug: String,
        title: String,
    },
    InstanceGoLive {
        base: String,
        app_id: String,
        slug: String,
        domain: String,
    },
    InstanceDestroy {
        base: String,
        app_id: String,
        slug: String,
    },
}

impl EventEmitter<BextViewEvent> for BextCloudView {}

pub struct BextCloudView {
    tab: BextTab,
    // Cloud state (fed by the workspace).
    connected: bool,
    user: Option<CloudUser>,
    sites: SitesResponse,
    stats: CloudStats,
    instances: Vec<CloudInstance>,
    // Instance state.
    instance_sites: Vec<InstanceSite>,
    // Real `Input` states — one per composer field.
    site_name_state: Entity<InputState>,
    site_title_state: Entity<InputState>,
    inst_base_state: Entity<InputState>,
    inst_app_id_state: Entity<InputState>,
    inst_slug_state: Entity<InputState>,
    inst_domain_state: Entity<InputState>,
    confirm_destroy: Option<String>,
    confirm_instance_destroy: Option<String>,
    loading: bool,
    error: Option<String>,
    focus_handle: FocusHandle,
}

fn new_input_state_bx(cx: &mut Context<BextCloudView>, initial: &str) -> Entity<InputState> {
    let initial = initial.to_string();
    cx.new(|cx| {
        let mut s = InputState::new(cx);
        if !initial.is_empty() {
            s.content = initial.into();
        }
        s
    })
}

impl BextCloudView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tab: BextTab::Cloud,
            connected: false,
            user: None,
            sites: SitesResponse::default(),
            stats: CloudStats::default(),
            instances: Vec::new(),
            instance_sites: Vec::new(),
            site_name_state: new_input_state_bx(cx, ""),
            site_title_state: new_input_state_bx(cx, ""),
            inst_base_state: new_input_state_bx(cx, "http://127.0.0.1"),
            inst_app_id_state: new_input_state_bx(cx, "default"),
            inst_slug_state: new_input_state_bx(cx, ""),
            inst_domain_state: new_input_state_bx(cx, ""),
            confirm_destroy: None,
            confirm_instance_destroy: None,
            loading: false,
            error: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn field_value(state: &Entity<InputState>, cx: &Context<Self>) -> String {
        state.read(cx).content().to_string()
    }

    fn set_input(state: &Entity<InputState>, value: &str, cx: &mut Context<Self>) {
        let v = value.to_string();
        state.update(cx, |s, cx| {
            s.content = v.into();
            cx.notify();
        });
    }

    fn reset_input(state: &Entity<InputState>, cx: &mut Context<Self>) {
        state.update(cx, |s, cx| {
            s.content = "".into();
            cx.notify();
        });
    }

    pub fn set_connection(&mut self, connected: bool, user: Option<CloudUser>) {
        self.connected = connected;
        self.user = user;
    }
    pub fn set_sites(&mut self, sites: SitesResponse) {
        self.sites = sites;
        self.loading = false;
        self.error = None;
    }
    pub fn set_stats(&mut self, stats: CloudStats) {
        self.stats = stats;
    }
    pub fn set_instances(&mut self, instances: Vec<CloudInstance>) {
        self.instances = instances;
    }
    pub fn set_instance_sites(
        &mut self,
        sites: Vec<InstanceSite>,
        base: String,
        app_id: String,
        cx: &mut Context<Self>,
    ) {
        self.instance_sites = sites;
        Self::set_input(&self.inst_base_state.clone(), &base, cx);
        Self::set_input(&self.inst_app_id_state.clone(), &app_id, cx);
        self.loading = false;
    }
    /// Open the Instance tab targeting a given box (from "Gérer bext").
    pub fn open_instance(&mut self, base: String, app_id: String, cx: &mut Context<Self>) {
        self.tab = BextTab::Instance;
        Self::set_input(&self.inst_base_state.clone(), &base, cx);
        Self::set_input(&self.inst_app_id_state.clone(), &app_id, cx);
    }
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        self.loading = false;
    }

    fn submit(&mut self, which: Field, cx: &mut Context<Self>) {
        match which {
            Field::Site => self.submit_create_site(cx),
            Field::Instance => self.submit_instance_create(cx),
            Field::InstanceRefresh => cx.emit(BextViewEvent::RefreshInstance {
                base: Self::field_value(&self.inst_base_state, cx)
                    .trim()
                    .to_string(),
                app_id: Self::field_value(&self.inst_app_id_state, cx)
                    .trim()
                    .to_string(),
            }),
        }
    }

    fn submit_create_site(&mut self, cx: &mut Context<Self>) {
        let name = Self::field_value(&self.site_name_state, cx)
            .trim()
            .to_lowercase();
        if name.is_empty() {
            return;
        }
        let title = Self::field_value(&self.site_title_state, cx)
            .trim()
            .to_string();
        Self::reset_input(&self.site_name_state.clone(), cx);
        Self::reset_input(&self.site_title_state.clone(), cx);
        cx.emit(BextViewEvent::CreateSite { name, title });
        cx.notify();
    }

    fn submit_instance_create(&mut self, cx: &mut Context<Self>) {
        let slug = Self::field_value(&self.inst_slug_state, cx)
            .trim()
            .to_lowercase();
        if slug.is_empty() {
            return;
        }
        let base = Self::field_value(&self.inst_base_state, cx)
            .trim()
            .to_string();
        let app_id = Self::field_value(&self.inst_app_id_state, cx)
            .trim()
            .to_string();
        Self::reset_input(&self.inst_slug_state.clone(), cx);
        cx.emit(BextViewEvent::InstanceCreate {
            base,
            app_id,
            slug,
            title: String::new(),
        });
        cx.notify();
    }

    /// Real `Input` widget, submit-routed via `submit(which, ...)` on Enter.
    fn input(
        &self,
        submit_field: Field,
        state: &Entity<InputState>,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Input::new(state)
            .size(InputSize::Sm)
            .placeholder(placeholder)
            .on_enter({
                let entity = cx.entity();
                move |_v, cx| {
                    entity.update(cx, |this, cx| this.submit(submit_field, cx));
                }
            })
    }

    fn btn(
        id: &'static str,
        label: &str,
        cx: &mut Context<Self>,
        on: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .px(px(9.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary())
            .cursor_pointer()
            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
            .child(label.to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on(this, cx)))
    }

    fn tab_btn(&self, tab: BextTab, label: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.tab == tab;
        let mut b = div()
            .id(ElementId::from(SharedString::from(format!(
                "bxtab-{label}"
            ))))
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .child(label.to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.tab = tab;
                if tab == BextTab::Instance {
                    cx.emit(BextViewEvent::RefreshInstance {
                        base: Self::field_value(&this.inst_base_state, cx)
                            .trim()
                            .to_string(),
                        app_id: Self::field_value(&this.inst_app_id_state, cx)
                            .trim()
                            .to_string(),
                    });
                }
                cx.notify();
            }));
        if active {
            b = b
                .bg(ShellDeckColors::selected_bg())
                .text_color(ShellDeckColors::text_primary());
        } else {
            b = b.text_color(ShellDeckColors::text_muted());
        }
        b
    }

    fn render_connection_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut card = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .m(px(16.0))
            .p(px(14.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar());

        if self.connected {
            let user = self.user.clone().unwrap_or_default();
            let mut ident = div().flex().flex_col();
            ident = ident.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(if user.name.is_empty() {
                                user.email.clone()
                            } else {
                                user.name.clone()
                            }),
                    )
                    .children(if user.is_super_admin {
                        Some(
                            div()
                                .px(px(5.0))
                                .rounded(px(6.0))
                                .bg(ShellDeckColors::primary().opacity(0.18))
                                .text_size(px(9.0))
                                .text_color(ShellDeckColors::primary())
                                .child("super-admin"),
                        )
                    } else {
                        None
                    }),
            );
            ident = ident.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("{} · cloud.bext.dev", user.email)),
            );
            card = card.child(ident).child(Self::btn(
                "bx-disconnect",
                "Se déconnecter",
                cx,
                |_t, cx| cx.emit(BextViewEvent::Disconnect),
            ));
        } else {
            card = card
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child("bext Cloud"),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child("Connectez-vous au plan de contrôle hébergé."),
                        ),
                )
                .child(
                    div()
                        .id("bx-connect")
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded(px(8.0))
                        .bg(ShellDeckColors::primary())
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(white())
                        .cursor_pointer()
                        .child("Se connecter")
                        .on_click(
                            cx.listener(|_t, _: &ClickEvent, _, cx| {
                                cx.emit(BextViewEvent::Connect)
                            }),
                        ),
                );
        }
        card
    }

    fn render_dashboard_strip(&self) -> impl IntoElement {
        let stat = |label: &str, n: u32| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .px(px(16.0))
                .py(px(8.0))
                .child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(n.to_string()),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(label.to_string()),
                )
        };
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .mx(px(16.0))
            .child(stat("Projets", self.stats.projects))
            .child(stat("Déploiements", self.stats.deploys))
            .child(stat("Domaines", self.stats.domains))
            .child(stat("Cibles", self.stats.targets))
    }

    fn render_cloud_sites(&self, cx: &mut Context<Self>) -> Div {
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .mx(px(16.0))
            .mt(px(8.0));
        if self.sites.sites.is_empty() {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Aucun site."),
            );
        }
        for site in &self.sites.sites {
            list = list.child(self.render_cloud_site_row(site, cx));
        }

        let at_max = self.sites.max > 0 && self.sites.count >= self.sites.max;
        let create = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .mx(px(16.0))
            .mt(px(10.0))
            .p(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child("Nouveau site WordPress"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("{}/{}", self.sites.count, self.sites.max.max(1))),
                    ),
            )
            .child(self.input(Field::Site, &self.site_name_state, "slug (minuscules)", cx))
            .child(self.input(
                Field::Site,
                &self.site_title_state,
                "Titre (facultatif)",
                cx,
            ))
            .child({
                let mut b = div()
                    .id("bx-create-site")
                    .px(px(12.0))
                    .py(px(7.0))
                    .rounded(px(6.0))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .w(px(140.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("Créer");
                if at_max {
                    b = b.bg(ShellDeckColors::text_muted()).opacity(0.5);
                } else {
                    b = b.bg(ShellDeckColors::primary()).cursor_pointer().on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.submit_create_site(cx)),
                    );
                }
                b
            });

        div().flex().flex_col().child(list).child(create)
    }

    fn render_cloud_site_row(&self, site: &CloudSite, cx: &mut Context<Self>) -> impl IntoElement {
        let slug = site.slug.clone();
        let has_domain = !site.primary_domain.is_empty();
        let confirming = self.confirm_destroy.as_deref() == Some(site.slug.as_str());

        let mut row = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if site.orphaned {
                ShellDeckColors::warning()
            } else {
                ShellDeckColors::border()
            })
            .bg(ShellDeckColors::bg_sidebar())
            .child(cloud_status_pill(&site.status))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(site.slug.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if has_domain {
                                site.primary_domain.clone()
                            } else {
                                format!("{} · {}", site.kind, site.env)
                            }),
                    ),
            );

        if has_domain {
            let dom = site.primary_domain.clone();
            row = row.child(Self::btn("bx-open", "Ouvrir ↗", cx, move |_t, cx| {
                cx.emit(BextViewEvent::OpenSite(dom.clone()))
            }));
        }
        {
            let s = slug.clone();
            row = row.child(Self::btn(
                "bx-golive",
                "Mettre en ligne",
                cx,
                move |_t, cx| {
                    cx.emit(BextViewEvent::SiteAction {
                        slug: s.clone(),
                        action: "go_live".into(),
                    })
                },
            ));
        }
        {
            let s = slug.clone();
            row = row.child(Self::btn("bx-config", "Config", cx, move |_t, cx| {
                cx.emit(BextViewEvent::SiteAction {
                    slug: s.clone(),
                    action: "config".into(),
                })
            }));
        }
        if confirming {
            let s1 = slug.clone();
            row = row
                .child(
                    div()
                        .id("bx-destroy-yes")
                        .px(px(9.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::error())
                        .text_size(px(12.0))
                        .text_color(white())
                        .cursor_pointer()
                        .child("Confirmer")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.confirm_destroy = None;
                            cx.emit(BextViewEvent::SiteAction {
                                slug: s1.clone(),
                                action: "destroy".into(),
                            });
                        })),
                )
                .child(Self::btn("bx-destroy-no", "Annuler", cx, |this, cx| {
                    this.confirm_destroy = None;
                    cx.notify();
                }));
        } else {
            let s = slug.clone();
            row = row.child(
                Self::btn("bx-destroy", "Détruire", cx, move |this, cx| {
                    this.confirm_destroy = Some(s.clone());
                    cx.notify();
                })
                .text_color(ShellDeckColors::error()),
            );
        }
        row
    }

    fn render_instances(&self) -> Div {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .mx(px(16.0))
            .mt(px(8.0));
        if self.instances.is_empty() {
            col = col.child(
                div()
                    .py(px(6.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Aucune instance."),
            );
        }
        for inst in &self.instances {
            let dot = match inst.status.as_str() {
                "online" | "healthy" => ShellDeckColors::success(),
                "degraded" => ShellDeckColors::warning(),
                "offline" | "down" => ShellDeckColors::error(),
                _ => ShellDeckColors::text_muted(),
            };
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .child(div().size(px(8.0)).rounded_full().bg(dot).flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(if inst.name.is_empty() {
                                        inst.host.clone()
                                    } else {
                                        inst.name.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!(
                                        "{} · {} · {}",
                                        inst.host, inst.status, inst.health
                                    )),
                            ),
                    ),
            );
        }
        col
    }

    fn render_cloud_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .id("bext-cloud-body")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(self.render_connection_card(cx));

        if self.connected {
            body = body
                .child(section("Tableau de bord"))
                .child(self.render_dashboard_strip())
                .child(section("Sites"))
                .child(self.render_cloud_sites(cx));
            if self
                .user
                .as_ref()
                .map(|u| u.is_super_admin)
                .unwrap_or(false)
            {
                body = body
                    .child(section("Instances bext (super-admin)"))
                    .child(self.render_instances());
            }
        }
        body
    }

    fn render_instance_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Target row.
        let target = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .mx(px(16.0))
            .mt(px(12.0))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Cible"),
            )
            .child(div().w(px(200.0)).child(self.input(
                Field::InstanceRefresh,
                &self.inst_base_state,
                "http://127.0.0.1",
                cx,
            )))
            .child(div().w(px(130.0)).child(self.input(
                Field::InstanceRefresh,
                &self.inst_app_id_state,
                "app-id",
                cx,
            )))
            .child(Self::btn("bx-inst-refresh", "Charger", cx, |this, cx| {
                cx.emit(BextViewEvent::RefreshInstance {
                    base: Self::field_value(&this.inst_base_state, cx)
                        .trim()
                        .to_string(),
                    app_id: Self::field_value(&this.inst_app_id_state, cx)
                        .trim()
                        .to_string(),
                });
            }));

        // Sites list.
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .mx(px(16.0))
            .mt(px(10.0));
        if self.instance_sites.is_empty() {
            list = list.child(
                div()
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Aucun site sur cette instance (ou non chargé)."),
            );
        }
        for site in &self.instance_sites {
            list = list.child(self.render_instance_site_row(site, cx));
        }

        // Create row.
        let create = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .mx(px(16.0))
            .mt(px(10.0))
            .p(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child("Nouveau site (SDK)"),
            )
            .child(self.input(Field::Instance, &self.inst_slug_state, "slug", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child("Domaine (mise en ligne)"),
                    )
                    .child(div().flex_1().child(self.input(
                        Field::Instance,
                        &self.inst_domain_state,
                        "exemple.com",
                        cx,
                    )))
                    .child(Self::btn("bx-inst-create", "Créer", cx, |this, cx| {
                        this.submit_instance_create(cx)
                    })),
            );

        div()
            .id("bext-instance-body")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(target)
            .child(section("Sites de l'instance"))
            .child(list)
            .child(create)
    }

    fn render_instance_site_row(
        &self,
        site: &InstanceSite,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let slug = site.slug.clone();
        let confirming = self.confirm_instance_destroy.as_deref() == Some(site.slug.as_str());
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .p(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_primary())
                            .child(if site.title.is_empty() {
                                site.slug.clone()
                            } else {
                                format!("{} ({})", site.title, site.slug)
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "{} · {} · {}",
                                site.kind,
                                site.env,
                                if site.primary_domain.is_empty() {
                                    site.unix_user.clone()
                                } else {
                                    site.primary_domain.clone()
                                }
                            )),
                    ),
            );

        {
            let base = Self::field_value(&self.inst_base_state, cx)
                .trim()
                .to_string();
            let app = Self::field_value(&self.inst_app_id_state, cx)
                .trim()
                .to_string();
            let s = slug.clone();
            let dom = Self::field_value(&self.inst_domain_state, cx)
                .trim()
                .to_string();
            row = row.child(Self::btn(
                "bx-inst-golive",
                "Mettre en ligne",
                cx,
                move |this, cx| {
                    if dom.is_empty() {
                        this.set_error("Saisissez un domaine pour la mise en ligne.");
                        cx.notify();
                        return;
                    }
                    cx.emit(BextViewEvent::InstanceGoLive {
                        base: base.clone(),
                        app_id: app.clone(),
                        slug: s.clone(),
                        domain: dom.clone(),
                    });
                },
            ));
        }
        if confirming {
            let base = Self::field_value(&self.inst_base_state, cx)
                .trim()
                .to_string();
            let app = Self::field_value(&self.inst_app_id_state, cx)
                .trim()
                .to_string();
            let s = slug.clone();
            row = row
                .child(
                    div()
                        .id("bx-inst-destroy-yes")
                        .px(px(9.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .bg(ShellDeckColors::error())
                        .text_size(px(12.0))
                        .text_color(white())
                        .cursor_pointer()
                        .child("Confirmer")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.confirm_instance_destroy = None;
                            cx.emit(BextViewEvent::InstanceDestroy {
                                base: base.clone(),
                                app_id: app.clone(),
                                slug: s.clone(),
                            });
                        })),
                )
                .child(Self::btn(
                    "bx-inst-destroy-no",
                    "Annuler",
                    cx,
                    |this, cx| {
                        this.confirm_instance_destroy = None;
                        cx.notify();
                    },
                ));
        } else {
            let s = slug.clone();
            row = row.child(
                Self::btn("bx-inst-destroy", "Détruire", cx, move |this, cx| {
                    this.confirm_instance_destroy = Some(s.clone());
                    cx.notify();
                })
                .text_color(ShellDeckColors::error()),
            );
        }
        row
    }
}

impl Render for BextCloudView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(self.tab_btn(BextTab::Cloud, "Cloud", cx))
                    .child(self.tab_btn(BextTab::Instance, "Instance", cx)),
            )
            .child(
                div()
                    .id("bext-refresh")
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .child(if self.loading {
                        "…"
                    } else {
                        "↻ Actualiser"
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        match this.tab {
                            BextTab::Cloud => cx.emit(BextViewEvent::RefreshCloud),
                            BextTab::Instance => cx.emit(BextViewEvent::RefreshInstance {
                                base: Self::field_value(&this.inst_base_state, cx)
                                    .trim()
                                    .to_string(),
                                app_id: Self::field_value(&this.inst_app_id_state, cx)
                                    .trim()
                                    .to_string(),
                            }),
                        }
                    })),
            );

        let body: AnyElement = match self.tab {
            BextTab::Cloud => self.render_cloud_tab(cx).into_any_element(),
            BextTab::Instance => self.render_instance_tab(cx).into_any_element(),
        };

        let mut root = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(header)
            .child(body);

        if let Some(err) = &self.error {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(12.0))
                    .left(px(12.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::error())
                    .text_size(px(12.0))
                    .text_color(white())
                    .child(err.clone()),
            );
        }
        root
    }
}

fn section(label: &str) -> impl IntoElement {
    div()
        .px(px(16.0))
        .pt(px(12.0))
        .pb(px(2.0))
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ShellDeckColors::text_muted())
        .child(label.to_string())
}

fn cloud_status_pill(status: &str) -> impl IntoElement {
    let (color, label) = match status {
        "live" | "running" | "active" => (ShellDeckColors::success(), status),
        "creating" | "provisioning" | "pending" => (ShellDeckColors::warning(), status),
        "error" | "failed" => (ShellDeckColors::error(), status),
        other => (ShellDeckColors::text_muted(), other),
    };
    div()
        .flex_shrink_0()
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(6.0))
        .bg(color.opacity(0.15))
        .text_size(px(10.0))
        .text_color(color)
        .child(label.to_string())
}
