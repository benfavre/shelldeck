use std::collections::HashMap;

use super::script::{PackageManager, Script, ScriptLanguage, ToolDependency};

/// The result of building a command from a script — provides both a
/// single-string SSH command and local binary+args.
#[derive(Debug, Clone)]
pub struct ScriptCommand {
    /// A full shell command string suitable for `ssh exec` (remote execution).
    pub ssh_command: String,
    /// The binary to run for local execution.
    pub local_binary: String,
    /// Arguments for local execution.
    pub local_args: Vec<String>,
    /// Environment variables to set.
    pub env_vars: Vec<(String, String)>,
}

/// Shell-escape a string for safe embedding in single quotes.
pub fn shell_escape(s: &str) -> String {
    // Replace all single quotes with '\'' (end quote, escaped quote, start quote)
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// Replace `{{name}}` and `{{name:default}}` placeholders in `body` with values from `values`.
/// Falls back to the inline default if present, or leaves the placeholder unchanged.
pub fn substitute_variables(body: &str, values: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(body.len());
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            let mut j = start;
            let mut found_close = false;
            while j + 1 < len {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    found_close = true;
                    break;
                }
                j += 1;
            }
            if found_close {
                let inner = &body[start..j];
                let (name, default) = if let Some(colon_pos) = inner.find(':') {
                    let n = inner[..colon_pos].trim();
                    let d = inner[colon_pos + 1..].trim();
                    (n, if d.is_empty() { None } else { Some(d) })
                } else {
                    (inner.trim(), None)
                };
                if let Some(val) = values.get(name) {
                    result.push_str(val);
                } else if let Some(d) = default {
                    result.push_str(d);
                } else {
                    // Leave placeholder unchanged
                    result.push_str(&body[i..j + 2]);
                }
                i = j + 2;
            } else {
                result.push('{');
                i += 1;
            }
        } else {
            let ch = body[i..]
                .chars()
                .next()
                .expect("i < body.len() guarantees a char");
            result.push(ch);
            i += ch.len_utf8();
        }
    }

    result
}

/// Build an executable command from a script, respecting its language.
/// When `var_values` is `Some`, template variables in the body are substituted first.
pub fn build_command(
    script: &Script,
    var_values: Option<&HashMap<String, String>>,
) -> ScriptCommand {
    let spec = script.language.runner_spec();
    let body: String = match var_values {
        Some(values) if !values.is_empty() => substitute_variables(&script.body, values),
        _ => script.body.clone(),
    };

    let env_vars: Vec<(String, String)> = script
        .env_vars
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // Build the env prefix for SSH (export KEY=VALUE; ...)
    let env_prefix = if env_vars.is_empty() {
        String::new()
    } else {
        let exports: Vec<String> = env_vars
            .iter()
            .map(|(k, v)| format!("export {}={};", k, shell_escape(v)))
            .collect();
        format!("{} ", exports.join(" "))
    };

    // Build cd prefix if working_dir is set
    let cd_prefix = script
        .working_dir
        .as_ref()
        .map(|dir| format!("cd {} && ", shell_escape(dir)))
        .unwrap_or_default();

    // Build sudo prefix if run_as is set
    let sudo_prefix = script
        .run_as
        .as_ref()
        .map(|user| format!("sudo -u {} ", user))
        .unwrap_or_default();

    let full_prefix = format!("{}{}{}", env_prefix, cd_prefix, sudo_prefix);

    match &script.language {
        // Languages that use -c/-e with the script body as an argument
        ScriptLanguage::Shell
        | ScriptLanguage::Python
        | ScriptLanguage::Node
        | ScriptLanguage::Bun
        | ScriptLanguage::Php
        | ScriptLanguage::Mysql
        | ScriptLanguage::Postgresql => {
            let mut local_args = Vec::new();
            let mut ssh_parts = Vec::new();

            ssh_parts.push(full_prefix.clone());
            ssh_parts.push(spec.binary.clone());

            for arg in &spec.args {
                if arg == "{script}" {
                    local_args.push(body.clone());
                    ssh_parts.push(shell_escape(&body));
                } else {
                    local_args.push(arg.clone());
                    ssh_parts.push(arg.clone());
                }
            }

            // Build the full ssh command
            let ssh_command = ssh_parts.join(" ");

            // For local: the first arg pair was already handled
            let mut final_local_args = Vec::new();
            for arg in &spec.args {
                if arg == "{script}" {
                    final_local_args.push(body.clone());
                } else {
                    final_local_args.push(arg.clone());
                }
            }

            ScriptCommand {
                ssh_command,
                local_binary: spec.binary.clone(),
                local_args: final_local_args,
                env_vars,
            }
        }

        // Languages where the body is passed as positional args (docker, systemctl, nginx)
        ScriptLanguage::Docker
        | ScriptLanguage::DockerCompose
        | ScriptLanguage::Systemd
        | ScriptLanguage::Nginx => {
            // Split body into args (shell word splitting)
            let body_args: Vec<&str> = body.split_whitespace().collect();

            let mut local_args = Vec::new();
            let mut ssh_parts = vec![full_prefix.clone(), spec.binary.clone()];

            // For DockerCompose, the spec has "compose" as first arg
            for arg in &spec.args {
                if arg == "{body_as_args}" {
                    for ba in &body_args {
                        local_args.push(ba.to_string());
                        ssh_parts.push(ba.to_string());
                    }
                } else {
                    local_args.push(arg.clone());
                    ssh_parts.push(arg.clone());
                }
            }

            ScriptCommand {
                ssh_command: ssh_parts.join(" "),
                local_binary: spec.binary.clone(),
                local_args,
                env_vars,
            }
        }

        // Custom runners follow the spec literally
        ScriptLanguage::Custom(_) => {
            let mut local_args = Vec::new();
            let mut ssh_parts = vec![full_prefix.clone(), spec.binary.clone()];

            for arg in &spec.args {
                if arg == "{script}" {
                    local_args.push(body.clone());
                    ssh_parts.push(shell_escape(&body));
                } else if arg == "{body_as_args}" {
                    let body_args: Vec<&str> = body.split_whitespace().collect();
                    for ba in &body_args {
                        local_args.push(ba.to_string());
                        ssh_parts.push(ba.to_string());
                    }
                } else {
                    local_args.push(arg.clone());
                    ssh_parts.push(arg.clone());
                }
            }

            ScriptCommand {
                ssh_command: ssh_parts.join(" "),
                local_binary: spec.binary.clone(),
                local_args,
                env_vars,
            }
        }
    }
}

/// Build a shell command that checks whether all required tools are available.
/// Returns a script that prints JSON-like output: `tool_name: OK` or `tool_name: MISSING`.
pub fn build_dependency_check_command(deps: &[ToolDependency]) -> String {
    if deps.is_empty() {
        return "echo 'No dependencies to check'".to_string();
    }

    let checks: Vec<String> = deps
        .iter()
        .map(|dep| {
            format!(
                "if {} >/dev/null 2>&1; then echo '{}: OK'; else echo '{}: MISSING'; fi",
                dep.check_command, dep.name, dep.name
            )
        })
        .collect();

    checks.join(" && ")
}

/// Build a command that detects the available package manager.
pub fn build_package_manager_detect_command() -> String {
    "if command -v apt-get >/dev/null 2>&1; then echo 'apt'; \
     elif command -v dnf >/dev/null 2>&1; then echo 'dnf'; \
     elif command -v yum >/dev/null 2>&1; then echo 'yum'; \
     elif command -v pacman >/dev/null 2>&1; then echo 'pacman'; \
     elif command -v brew >/dev/null 2>&1; then echo 'brew'; \
     elif command -v apk >/dev/null 2>&1; then echo 'apk'; \
     else echo 'unknown'; fi"
        .to_string()
}

/// Given a detected package manager name and a tool dependency,
/// return the install command if available.
pub fn get_install_command(pm_name: &str, dep: &ToolDependency) -> Option<String> {
    let pm = match pm_name {
        "apt" => PackageManager::Apt,
        "yum" => PackageManager::Yum,
        "dnf" => PackageManager::Dnf,
        "pacman" => PackageManager::Pacman,
        "brew" => PackageManager::Brew,
        "apk" => PackageManager::Apk,
        _ => return None,
    };

    dep.install_commands
        .iter()
        .find(|ic| ic.package_manager == pm)
        .map(|ic| ic.command.clone())
}
