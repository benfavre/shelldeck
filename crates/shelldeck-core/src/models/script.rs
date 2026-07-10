use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// ScriptLanguage — determines how the script body gets executed
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScriptLanguage {
    #[default]
    Shell,
    Python,
    Node,
    Bun,
    Php,
    Mysql,
    Postgresql,
    Docker,
    DockerCompose,
    Systemd,
    Nginx,
    Custom(CustomRunner),
}

impl ScriptLanguage {
    /// Human-readable short label for UI badges.
    pub fn label(&self) -> &str {
        match self {
            Self::Shell => "Shell",
            Self::Python => "Python",
            Self::Node => "Node",
            Self::Bun => "Bun",
            Self::Php => "PHP",
            Self::Mysql => "MySQL",
            Self::Postgresql => "PostgreSQL",
            Self::Docker => "Docker",
            Self::DockerCompose => "Compose",
            Self::Systemd => "Systemd",
            Self::Nginx => "Nginx",
            Self::Custom(_) => "Custom",
        }
    }

    /// Short badge text (max ~4 chars) for compact UI.
    pub fn badge(&self) -> &str {
        match self {
            Self::Shell => "SH",
            Self::Python => "PY",
            Self::Node => "JS",
            Self::Bun => "BUN",
            Self::Php => "PHP",
            Self::Mysql => "SQL",
            Self::Postgresql => "SQL",
            Self::Docker => "DKR",
            Self::DockerCompose => "DC",
            Self::Systemd => "SYS",
            Self::Nginx => "NGX",
            Self::Custom(_) => "CUS",
        }
    }

    /// Brand color (Simple Icons hex) as `(r, g, b)` for badges and icon tint.
    pub fn badge_color(&self) -> (u8, u8, u8) {
        match self {
            Self::Shell => (78, 170, 37),       // GNU Bash #4EAA25
            Self::Python => (55, 118, 171),     // #3776AB
            Self::Node => (95, 160, 78),        // Node.js #5FA04E
            Self::Bun => (251, 191, 36),        // #FBBF24 (visible on dark; SI mark is #000)
            Self::Php => (119, 123, 180),       // #777BB4
            Self::Mysql => (68, 121, 161),      // #4479A1
            Self::Postgresql => (65, 105, 225), // #4169E1
            Self::Docker | Self::DockerCompose => (36, 150, 237), // #2496ED
            Self::Systemd => (233, 84, 32),     // #E95420 (Tux recolored; no SI slug)
            Self::Nginx => (0, 150, 57),        // #009639
            Self::Custom(_) => (128, 128, 128),
        }
    }

    /// Simple Icons slug (embedded under `icons/simple/`). Source:
    /// https://github.com/LitoMore/simple-icons-cdn
    pub fn simple_icon(&self) -> &'static str {
        match self {
            Self::Shell => "gnubash",
            Self::Python => "python",
            Self::Node => "nodedotjs",
            Self::Bun => "bun",
            Self::Php => "php",
            Self::Mysql => "mysql",
            Self::Postgresql => "postgresql",
            Self::Docker => "docker",
            Self::DockerCompose => "dockercompose",
            Self::Systemd => "systemd",
            Self::Nginx => "nginx",
            Self::Custom(_) => "linux",
        }
    }

    /// Return the `RunnerSpec` that describes how to execute scripts in this language.
    pub fn runner_spec(&self) -> RunnerSpec {
        match self {
            Self::Shell => RunnerSpec {
                binary: "sh".into(),
                args: vec!["-c".into(), "{script}".into()],
                needs_file: false,
                file_ext: "sh".into(),
            },
            Self::Python => RunnerSpec {
                binary: "python3".into(),
                args: vec!["-c".into(), "{script}".into()],
                needs_file: false,
                file_ext: "py".into(),
            },
            Self::Node => RunnerSpec {
                binary: "node".into(),
                args: vec!["-e".into(), "{script}".into()],
                needs_file: false,
                file_ext: "js".into(),
            },
            Self::Bun => RunnerSpec {
                binary: "bun".into(),
                args: vec!["-e".into(), "{script}".into()],
                needs_file: false,
                file_ext: "ts".into(),
            },
            Self::Php => RunnerSpec {
                binary: "php".into(),
                args: vec!["-r".into(), "{script}".into()],
                needs_file: false,
                file_ext: "php".into(),
            },
            Self::Mysql => RunnerSpec {
                binary: "mysql".into(),
                args: vec!["-e".into(), "{script}".into()],
                needs_file: false,
                file_ext: "sql".into(),
            },
            Self::Postgresql => RunnerSpec {
                binary: "psql".into(),
                args: vec!["-c".into(), "{script}".into()],
                needs_file: false,
                file_ext: "sql".into(),
            },
            Self::Docker => RunnerSpec {
                binary: "docker".into(),
                args: vec!["{body_as_args}".into()],
                needs_file: false,
                file_ext: "".into(),
            },
            Self::DockerCompose => RunnerSpec {
                binary: "docker".into(),
                args: vec!["compose".into(), "{body_as_args}".into()],
                needs_file: false,
                file_ext: "".into(),
            },
            Self::Systemd => RunnerSpec {
                binary: "systemctl".into(),
                args: vec!["{body_as_args}".into()],
                needs_file: false,
                file_ext: "".into(),
            },
            Self::Nginx => RunnerSpec {
                binary: "nginx".into(),
                args: vec!["{body_as_args}".into()],
                needs_file: false,
                file_ext: "".into(),
            },
            Self::Custom(runner) => RunnerSpec {
                binary: runner.binary.clone(),
                args: runner.args.clone(),
                needs_file: runner.needs_file,
                file_ext: runner.file_ext.clone(),
            },
        }
    }

    /// All built-in variants (excluding Custom).
    pub const ALL: &[ScriptLanguage] = &[
        Self::Shell,
        Self::Python,
        Self::Node,
        Self::Bun,
        Self::Php,
        Self::Mysql,
        Self::Postgresql,
        Self::Docker,
        Self::DockerCompose,
        Self::Systemd,
        Self::Nginx,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomRunner {
    pub binary: String,
    pub args: Vec<String>,
    pub needs_file: bool,
    pub file_ext: String,
}

/// Describes how a language executes — returned by `ScriptLanguage::runner_spec()`.
#[derive(Debug, Clone)]
pub struct RunnerSpec {
    pub binary: String,
    pub args: Vec<String>,
    pub needs_file: bool,
    pub file_ext: String,
}

// ---------------------------------------------------------------------------
// ScriptCategory — for grouping in the UI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ScriptCategory {
    System,
    Database,
    Web,
    Runtime,
    Container,
    Network,
    Security,
    Custom,
    #[default]
    Uncategorized,
}

impl ScriptCategory {
    pub fn label(&self) -> &str {
        match self {
            Self::System => "System",
            Self::Database => "Database",
            Self::Web => "Web",
            Self::Runtime => "Runtime",
            Self::Container => "Container",
            Self::Network => "Network",
            Self::Security => "Security",
            Self::Custom => "Custom",
            Self::Uncategorized => "Uncategorized",
        }
    }

    /// Lucide slug for category chips (embedded under `icons/lucide/`).
    pub fn lucide_icon(&self) -> &'static str {
        match self {
            Self::System => "settings",
            Self::Database => "database",
            Self::Web => "globe",
            Self::Runtime => "cpu",
            Self::Container => "box",
            Self::Network => "server",
            Self::Security => "shield",
            Self::Custom => "pencil",
            Self::Uncategorized => "circle-help",
        }
    }

    /// All categories for filter tabs (excluding Uncategorized).
    pub const ALL: &[ScriptCategory] = &[
        Self::System,
        Self::Database,
        Self::Web,
        Self::Runtime,
        Self::Container,
        Self::Network,
        Self::Security,
        Self::Custom,
        Self::Uncategorized,
    ];
}

// ---------------------------------------------------------------------------
// ToolDependency — what a script needs installed
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDependency {
    pub name: String,
    pub check_command: String,
    pub install_commands: Vec<InstallCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallCommand {
    pub package_manager: PackageManager,
    pub command: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PackageManager {
    Apt,
    Yum,
    Dnf,
    Pacman,
    Brew,
    Apk,
}

impl PackageManager {
    pub fn label(&self) -> &str {
        match self {
            Self::Apt => "apt",
            Self::Yum => "yum",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Brew => "brew",
            Self::Apk => "apk",
        }
    }
}

// ---------------------------------------------------------------------------
// ScriptTarget (unchanged)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScriptTarget {
    Local,
    Remote(Uuid),
    AskOnRun,
}

// ---------------------------------------------------------------------------
// ScriptVariable — template variable metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptVariable {
    pub name: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub default_value: Option<String>,
}

/// Extract `{{name}}` and `{{name:default}}` variable references from a script body.
/// Returns `(name, optional_default)` pairs, deduplicated by name, in order of first occurrence.
pub fn extract_variables(body: &str) -> Vec<(String, Option<String>)> {
    let mut results: Vec<(String, Option<String>)> = Vec::new();
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i + 1 < len {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            // Find closing }}
            let mut j = start;
            while j + 1 < len {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    let inner = &body[start..j];
                    if !inner.is_empty() {
                        let (name, default) = if let Some(colon_pos) = inner.find(':') {
                            let n = inner[..colon_pos].trim();
                            let d = inner[colon_pos + 1..].trim();
                            (
                                n.to_string(),
                                if d.is_empty() {
                                    None
                                } else {
                                    Some(d.to_string())
                                },
                            )
                        } else {
                            (inner.trim().to_string(), None)
                        };
                        if !name.is_empty() && !results.iter().any(|(n, _)| n == &name) {
                            results.push((name, default));
                        }
                    }
                    i = j + 2;
                    break;
                }
                j += 1;
            }
            if j + 1 >= len {
                break;
            }
        } else {
            i += 1;
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Script — the main struct, extended with new fields
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub body: String,
    pub target: ScriptTarget,
    pub env_vars: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub run_as: Option<String>,
    pub tags: Vec<String>,
    // New fields — all #[serde(default)] for backward compat
    #[serde(default)]
    pub language: ScriptLanguage,
    #[serde(default)]
    pub category: ScriptCategory,
    #[serde(default)]
    pub dependencies: Vec<ToolDependency>,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub pinned_to_toolbar: bool,
    #[serde(default)]
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub run_count: u32,
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub variables: Vec<ScriptVariable>,
}

impl Script {
    pub fn new(name: String, body: String, target: ScriptTarget) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            description: None,
            body,
            target,
            env_vars: HashMap::new(),
            working_dir: None,
            run_as: None,
            tags: Vec::new(),
            language: ScriptLanguage::Shell,
            category: ScriptCategory::Uncategorized,
            dependencies: Vec::new(),
            is_favorite: false,
            pinned_to_toolbar: false,
            last_run: None,
            run_count: 0,
            is_template: false,
            template_id: None,
            variables: Vec::new(),
        }
    }

    /// Create a new script with language and category.
    pub fn new_with_language(
        name: String,
        body: String,
        target: ScriptTarget,
        language: ScriptLanguage,
        category: ScriptCategory,
    ) -> Self {
        let mut script = Self::new(name, body, target);
        script.language = language;
        script.category = category;
        script
    }

    /// Merge auto-detected `{{var}}` placeholders from the body with any
    /// explicit metadata in `self.variables`. Variables found in the body but
    /// not in metadata get a bare entry. Variables in metadata but not in the
    /// body are silently dropped.
    pub fn resolved_variables(&self) -> Vec<ScriptVariable> {
        let detected = extract_variables(&self.body);
        detected
            .into_iter()
            .map(|(name, inline_default)| {
                if let Some(meta) = self.variables.iter().find(|v| v.name == name) {
                    // Prefer metadata, but fall back to inline default if metadata has none
                    ScriptVariable {
                        name: meta.name.clone(),
                        label: meta.label.clone(),
                        description: meta.description.clone(),
                        default_value: meta.default_value.clone().or(inline_default),
                    }
                } else {
                    ScriptVariable {
                        name,
                        label: None,
                        description: None,
                        default_value: inline_default,
                    }
                }
            })
            .collect()
    }

    /// Built-in: Check disk usage
    pub fn builtin_disk_usage() -> Self {
        let mut s = Self::new(
            "Check Disk Usage".to_string(),
            "df -h".to_string(),
            ScriptTarget::AskOnRun,
        );
        s.category = ScriptCategory::System;
        s
    }

    /// Built-in: Tail logs
    pub fn builtin_tail_logs() -> Self {
        let mut s = Self::new(
            "Tail System Logs".to_string(),
            "tail -f /var/log/syslog".to_string(),
            ScriptTarget::AskOnRun,
        );
        s.category = ScriptCategory::System;
        s
    }

    /// Built-in: System info
    pub fn builtin_system_info() -> Self {
        let mut s = Self::new(
            "System Info".to_string(),
            "echo '=== OS ===' && uname -a && echo '=== Memory ===' && free -h && echo '=== CPU ===' && nproc && echo '=== Uptime ===' && uptime".to_string(),
            ScriptTarget::AskOnRun,
        );
        s.category = ScriptCategory::System;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_variables, ScriptLanguage};

    // SDTEST-037 — every built-in ScriptLanguage produces a runnable
    // spec: non-empty binary and non-empty args. Adding a new variant
    // to `ScriptLanguage::ALL` without wiring it into `runner_spec`
    // will trigger this test.
    #[test]
    fn every_builtin_language_has_a_runnable_spec() {
        for lang in ScriptLanguage::ALL {
            let spec = lang.runner_spec();
            assert!(
                !spec.binary.is_empty(),
                "{lang:?}: runner_spec.binary must not be empty",
            );
            assert!(
                !spec.args.is_empty(),
                "{lang:?}: runner_spec.args must not be empty",
            );
        }
    }

    // File-based languages (`{script}` template in args) advertise a
    // file extension so downstream can spill the body to a temp file
    // when `needs_file` is set. Container / service languages
    // (Docker, Compose, Systemd, Nginx) run a subcommand instead and
    // legitimately expose an empty `file_ext` — asserting that
    // separates the two families explicitly.
    #[test]
    fn file_based_languages_declare_an_extension() {
        use ScriptLanguage::*;
        let file_based = [Shell, Python, Node, Bun, Php, Mysql, Postgresql];
        for lang in file_based {
            let spec = lang.runner_spec();
            assert!(
                !spec.file_ext.is_empty(),
                "{lang:?}: file_ext must be set for a file-based language",
            );
        }

        let subcommand = [Docker, DockerCompose, Systemd, Nginx];
        for lang in subcommand {
            let spec = lang.runner_spec();
            assert!(
                spec.file_ext.is_empty(),
                "{lang:?}: subcommand-style languages should not expose a file_ext",
            );
        }
    }

    // Every builtin resolves to a distinct combination — regression
    // sensor for copy-paste bugs (e.g. two variants pointing at the
    // same binary + args).
    #[test]
    fn each_builtin_has_a_unique_runner_binary_or_args() {
        use std::collections::HashSet;
        let mut seen: HashSet<(String, Vec<String>)> = HashSet::new();
        for lang in ScriptLanguage::ALL {
            let spec = lang.runner_spec();
            let key = (spec.binary.clone(), spec.args.clone());
            assert!(
                seen.insert(key),
                "{lang:?}: (binary, args) collides with an earlier variant",
            );
        }
    }

    // SDTEST-034 — `{{name}}` and `{{name:default}}` extraction is the
    // foundation of every template. First occurrence wins on dedup,
    // whitespace inside braces is trimmed, and an empty inner (`{{}}`)
    // is ignored.
    #[test]
    fn extracts_bare_names_dedup_preserves_first_occurrence() {
        let body = "echo {{host}} && ssh {{user}}@{{host}} echo {{message}}";
        let vars = extract_variables(body);
        assert_eq!(
            vars,
            vec![
                ("host".to_string(), None),
                ("user".to_string(), None),
                ("message".to_string(), None),
            ],
            "dedup keeps first occurrence, order preserved",
        );
    }

    #[test]
    fn extracts_defaults_after_colon() {
        let body = "curl {{url:https://example.com}} -H {{header:X-Auth: bearer}}";
        let vars = extract_variables(body);
        // NB: the split is on the FIRST `:`, so a colon inside the default
        // is preserved intact (e.g. an `Authorization: bearer` header).
        assert_eq!(
            vars,
            vec![
                ("url".to_string(), Some("https://example.com".to_string())),
                ("header".to_string(), Some("X-Auth: bearer".to_string())),
            ],
        );
    }

    #[test]
    fn trims_inner_whitespace_and_ignores_empty() {
        let body = "{{  spaced  }} then {{}} nothing then {{ :onlydefault }}";
        let vars = extract_variables(body);
        // `{{}}` is empty inside → skipped.
        // `{{ :onlydefault }}` has empty `name` after trim → skipped
        // (the impl checks `!name.is_empty()`).
        assert_eq!(vars, vec![("spaced".to_string(), None)]);
    }

    #[test]
    fn same_name_second_occurrence_ignored_even_with_default() {
        // First `{{host}}` has no default; second `{{host:localhost}}` would
        // introduce one, but dedup keeps the FIRST metadata as-is. This is
        // the documented contract (dedup by name, first wins).
        let body = "{{host}} then {{host:localhost}}";
        let vars = extract_variables(body);
        assert_eq!(vars, vec![("host".to_string(), None)]);
    }

    #[test]
    fn unclosed_placeholder_is_silently_dropped() {
        // `{{host}}` closes, `{{oops` never closes.
        let body = "{{host}} and {{oops never closes";
        let vars = extract_variables(body);
        assert_eq!(vars, vec![("host".to_string(), None)]);
    }

    // SDTEST-035 — Pin CURRENT behaviour: fenced code blocks are
    // NOT special-cased. A `{{host}}` inside triple-backticks is
    // still extracted as a variable. This bites if a user pastes an
    // Ansible / Vue / Handlebars snippet that legitimately uses
    // `{{…}}` syntax — they'll be prompted for it.
    //
    // Documented limitation, not a bug per se — the fix requires
    // implementing fence-skip in the parser (SDTEST-035 was
    // originally P1 in my inventory). This test locks the shape so
    // a future fence-aware refactor is a deliberate contract change.
    #[test]
    fn extracts_placeholders_even_inside_code_fences() {
        let body = "\
            echo {{real_var}}\n\
            ```yaml\n\
            server: {{ansible_var}}\n\
            template: '{{handlebars_var}}'\n\
            ```\n\
        ";
        let vars = extract_variables(body);
        let names: Vec<&str> = vars.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"real_var"));
        assert!(names.contains(&"ansible_var"));
        assert!(names.contains(&"handlebars_var"));
        assert_eq!(vars.len(), 3, "current parser doesn't fence-skip");
    }
}
