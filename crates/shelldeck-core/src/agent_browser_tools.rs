//! Ephemeral browser MCP launch data. These capabilities never enter saved
//! agent requests, session snapshots, or the user's provider configuration.
use crate::agent_runtime::{AgentInputOptions, AgentProvider, AgentRunRequest, AgentTarget};
use crate::{Result, ShellDeckError};
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, PartialEq, Eq)]
pub struct AgentMcpServerLaunch {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Claude receives a private, run-owned config file so capability values
    /// are never exposed in process arguments. The caller retains its lifetime.
    pub claude_config_path: Option<PathBuf>,
}

impl std::fmt::Debug for AgentMcpServerLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentMcpServerLaunch")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("argument_count", &self.args.len())
            .field("environment_keys", &self.env.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

pub fn validate_browser_mcp(request: &AgentRunRequest, options: &AgentInputOptions) -> Result<()> {
    if options.mcp_servers.is_empty() {
        return Ok(());
    }
    let fail =
        || ShellDeckError::Config("Invalid or unsupported scoped browser MCP configuration".into());
    if request.target != AgentTarget::Local
        || !matches!(
            request.provider,
            AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Automonique
        )
        || options.mcp_servers.len() > 4
    {
        return Err(fail());
    }
    let mut names = std::collections::BTreeSet::new();
    for server in &options.mcp_servers {
        if server.name.is_empty()
            || server.name.len() > 64
            || !server
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            || !names.insert(&server.name)
            || !server.command.is_absolute()
            || server
                .command
                .to_str()
                .is_none_or(|s| s.contains(['\0', '\r', '\n']) || s.len() > 4096)
            || server.args.len() > 32
            || server
                .args
                .iter()
                .any(|s| s.contains('\0') || s.len() > 4096)
            || server.env.len() > 32
            || server.env.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || value.contains('\0')
                    || value.len() > 16 * 1024
            })
        {
            return Err(fail());
        }
        if request.provider == AgentProvider::Claude
            && server.claude_config_path.as_ref().is_none_or(|p| {
                !p.is_absolute()
                    || p.to_str()
                        .is_none_or(|s| s.contains(['\0', '\r', '\n']) || s.len() > 4096)
            })
        {
            return Err(fail());
        }
    }
    Ok(())
}

pub fn codex_browser_config(servers: &[AgentMcpServerLaunch]) -> Value {
    let mut config = Map::new();
    for server in servers {
        config.insert(
            format!("mcp_servers.{}", server.name),
            json!({
                "command":server.command, "args":server.args, "env":server.env,
                "enabled":true, "required":true, "startup_timeout_sec":20, "tool_timeout_sec":30,
            }),
        );
    }
    Value::Object(config)
}

pub fn acp_browser_servers(
    servers: &[AgentMcpServerLaunch],
) -> Vec<agent_client_protocol::schema::v1::McpServer> {
    use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};
    servers
        .iter()
        .map(|s| {
            McpServer::Stdio(
                McpServerStdio::new(&s.name, &s.command)
                    .args(s.args.clone())
                    .env(
                        s.env
                            .iter()
                            .map(|(key, value)| EnvVariable::new(key, value))
                            .collect(),
                    ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{AgentAccessMode, AgentCommandSpec};
    fn fixture() -> (AgentRunRequest, AgentInputOptions) {
        let path = std::env::temp_dir();
        let request = AgentRunRequest::new(
            AgentProvider::Claude,
            AgentTarget::Local,
            AgentAccessMode::ReadOnly,
            path.to_string_lossy(),
            None,
            "Inspect the open browser",
        );
        let options = AgentInputOptions {
            mcp_servers: vec![AgentMcpServerLaunch {
                name: "shelldeck_browser".into(),
                command: path.join("helper"),
                args: vec!["--mcp".into(), "127.0.0.1:9000".into()],
                env: BTreeMap::from([(
                    "SHELLDECK_BROWSER_CAPABILITY".into(),
                    "private-capability".into(),
                )]),
                claude_config_path: Some(path.join("private browser config.json")),
            }],
            ..Default::default()
        };
        (request, options)
    }
    // SDTEST-2585 — SDUC-557
    #[test]
    fn browser_capabilities_stay_out_of_cli_arguments_and_debug_output() {
        let (request, options) = fixture();
        let spec = AgentCommandSpec::for_request_with_options(&request, &options).unwrap();
        assert!(spec
            .args
            .windows(2)
            .any(|a| a[0] == "--mcp-config" && a[1].contains("private browser config.json")));
        assert!(!format!("{options:?} {spec:?}").contains("private-capability"));
        let config = codex_browser_config(&options.mcp_servers);
        assert_eq!(
            config["mcp_servers.shelldeck_browser"]["env"]["SHELLDECK_BROWSER_CAPABILITY"],
            "private-capability"
        );
        assert_eq!(config["mcp_servers.shelldeck_browser"]["required"], true);
        let acp = serde_json::to_value(acp_browser_servers(&options.mcp_servers)).unwrap();
        assert_eq!(acp[0]["env"][0]["value"], "private-capability");
    }
    // SDTEST-2586 — SDUC-557
    #[test]
    fn browser_mcp_refuses_remote_targets_unsupported_providers_and_config_injection() {
        let (mut request, mut options) = fixture();
        request.target = AgentTarget::Ssh {
            connection_id: uuid::Uuid::new_v4(),
            connection_label: "remote".into(),
        };
        assert!(validate_browser_mcp(&request, &options).is_err());
        request.target = AgentTarget::Local;
        request.provider = AgentProvider::Jcode;
        assert!(validate_browser_mcp(&request, &options).is_err());
        request.provider = AgentProvider::Codex;
        options.mcp_servers[0].name = "name.override".into();
        assert!(validate_browser_mcp(&request, &options).is_err());
        options.mcp_servers[0].name = "shelldeck_browser".into();
        options.mcp_servers[0].command = PathBuf::from("relative-helper");
        assert!(validate_browser_mcp(&request, &options).is_err());
    }
}
