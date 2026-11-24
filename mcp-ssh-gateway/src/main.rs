use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{env, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::timeout,
};

const PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_LINE_BYTES: usize = 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const HARD_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_MAX_COMMAND_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 256 * 1024;

#[derive(Clone)]
struct Config {
    host: String,
    user: String,
    port: u16,
    identity_file: String,
    known_hosts_file: String,
    use_sudo: bool,
    max_command_bytes: usize,
    max_output_bytes: usize,
}

impl Config {
    fn from_env() -> Result<Self> {
        let host = required("SSH_HOST")?;
        let user = required("SSH_USER")?;
        let identity_file = required("SSH_IDENTITY_FILE")?;
        let known_hosts_file = required("SSH_KNOWN_HOSTS_FILE")?;
        if !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c)) {
            bail!("SSH_HOST contains unsupported characters");
        }
        if !user.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)) {
            bail!("SSH_USER contains unsupported characters");
        }
        let port = env_num("SSH_PORT", 22_u16)?;
        let use_sudo = env::var("SSH_SUDO").unwrap_or_else(|_| "true".into()) == "true";
        Ok(Self {
            host, user, port, identity_file, known_hosts_file, use_sudo,
            max_command_bytes: env_num("SSH_MAX_COMMAND_BYTES", DEFAULT_MAX_COMMAND_BYTES)?,
            max_output_bytes: env_num("SSH_MAX_OUTPUT_BYTES", DEFAULT_MAX_OUTPUT_BYTES)?,
        })
    }
}

fn required(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing required environment variable {name}"))
}

fn env_num<T: std::str::FromStr>(name: &str, default: T) -> Result<T>
where T::Err: std::fmt::Display {
    match env::var(name) {
        Ok(v) => v.parse().map_err(|e| anyhow!("invalid {name}: {e}")),
        Err(_) => Ok(default),
    }
}

fn audit(event: &str, details: Value) {
    eprintln!("{}", json!({
        "at": chrono_free_timestamp(),
        "event": event,
        "details": details
    }));
}

fn chrono_free_timestamp() -> String {
    // Unix timestamp is sufficient for a minimal audit record without another dependency.
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string()).unwrap_or_else(|_| "unknown".into())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace(char::from(39), "'\\''"))
}

async fn run_exec(config: &Config, command: &str, timeout_ms: u64) -> Result<String> {
    if command.trim().is_empty() { bail!("command must not be empty"); }
    if command.as_bytes().len() > config.max_command_bytes {
        bail!("command exceeds {} byte limit", config.max_command_bytes);
    }
    if command.contains('\0') { bail!("command contains NUL byte"); }

    let remote_command = if config.use_sudo {
        format!("sudo -n -- /bin/bash -lc {}", shell_quote(command))
    } else {
        format!("/bin/bash -lc {}", shell_quote(command))
    };

    let mut child = Command::new("ssh")
        .arg("-T")
        .arg("-p").arg(config.port.to_string())
        .arg("-i").arg(&config.identity_file)
        .args(["-o", "BatchMode=yes"])
        .args(["-o", "IdentitiesOnly=yes"])
        .args(["-o", "StrictHostKeyChecking=yes"])
        .arg("-o").arg(format!("UserKnownHostsFile={}", config.known_hosts_file))
        .args(["-o", "ConnectTimeout=10"])
        .args(["-o", "ServerAliveInterval=15"])
        .args(["-o", "ServerAliveCountMax=2"])
        .arg(format!("{}@{}", config.user, config.host))
        .arg(remote_command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("failed to start SSH client")?;

    let stdout = child.stdout.take().context("missing SSH stdout")?;
    let stderr = child.stderr.take().context("missing SSH stderr")?;
    let max_out = config.max_output_bytes;

    let out_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let mut reader = stdout.take((max_out + 1) as u64);
        reader.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let err_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let mut reader = stderr.take(16 * 1024);
        reader.read_to_end(&mut bytes).await.map(|_| bytes)
    });

    let effective_timeout = timeout_ms.clamp(1_000, HARD_TIMEOUT_MS);
    let status = match timeout(Duration::from_millis(effective_timeout), child.wait()).await {
        Ok(result) => result.context("failed waiting for SSH")?,
        Err(_) => {
            let _ = child.kill().await;
            bail!("SSH command timed out after {effective_timeout} ms");
        }
    };

    let out = out_task.await.context("stdout reader failed")??;
    let err = err_task.await.context("stderr reader failed")??;
    if out.len() > max_out { bail!("command output exceeded {} byte limit", max_out); }

    let stdout = String::from_utf8_lossy(&out).into_owned();
    let stderr = String::from_utf8_lossy(&err).into_owned();
    if !status.success() {
        bail!("remote command exited with {}: {}", status, stderr.trim());
    }
    if stderr.trim().is_empty() { Ok(stdout) }
    else { Ok(format!("{stdout}\n[stderr]\n{}", stderr.trim())) }
}

fn tool_definition() -> Value {
    json!({
        "name": "exec",
        "description": "Execute an arbitrary shell command on the configured Congratis VPS over SSH. Commands run as the dedicated SSH user; with SSH_SUDO=true (default), they run through non-interactive sudo as root. This is full administrative access: commands can modify or destroy production data and services. Use only commands explicitly authorized by the operator.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute on the remote VPS." },
                "timeout_ms": { "type": "integer", "minimum": 1000, "maximum": 300000, "default": 30000 }
            },
            "required": ["command"],
            "additionalProperties": false
        }
    })
}

async fn handle(message: Value, config: &Config) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str)?;
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    let result: Result<Value> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "congratis-mcp-ssh-gateway", "version": "0.2.0" },
            "instructions": "This server exposes full remote shell execution over SSH. SSH_SUDO=true runs commands as root through sudo. Treat all exec calls as privileged production operations."
        })),
        "notifications/initialized" | "notifications/cancelled" => return None,
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": [tool_definition()] })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if name != "exec" {
                Err(anyhow!("unknown tool: {name}"))
            } else {
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let command = args.get("command").and_then(Value::as_str);
                let timeout_ms = args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(DEFAULT_TIMEOUT_MS);
                if args.as_object().map(|o| o.keys().any(|k| k != "command" && k != "timeout_ms")).unwrap_or(true) {
                    Err(anyhow!("unexpected or invalid tool arguments"))
                } else if let Some(command) = command {
                    audit("exec_started", json!({
                        "target": config.host,
                        "ssh_user": config.user,
                        "sudo": config.use_sudo,
                        "command_bytes": command.len(),
                    }));
                    match run_exec(config, command, timeout_ms).await {
                        Ok(output) => {
                            audit("exec_succeeded", json!({"target": config.host}));
                            Ok(json!({ "content": [{ "type": "text", "text": output }] }))
                        }
                        Err(error) => {
                            audit("exec_failed", json!({"target": config.host, "error": error.to_string()}));
                            Ok(json!({ "isError": true, "content": [{ "type": "text", "text": error.to_string() }] }))
                        }
                    }
                } else {
                    Err(anyhow!("command is required and must be a string"))
                }
            }
        }
        _ => Err(anyhow!("method not found: {method}")),
    };

    let id = id?;
    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": error.to_string() } }),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;
    audit("gateway_started", json!({
        "target": config.host, "ssh_user": config.user, "sudo": config.use_sudo
    }));

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        if line.len() > MAX_LINE_BYTES {
            audit("protocol_line_rejected", json!({"reason": "line_too_large"}));
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                let response = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}});
                stdout.write_all(format!("{response}\n").as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };
        if let Some(response) = handle(message, &config).await {
            stdout.write_all(format!("{response}\n").as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn shell_quote_preserves_spaces_and_quotes() {
        assert_eq!(shell_quote("echo hello"), "'echo hello'");
        assert_eq!(shell_quote("echo 'hello'"), "'echo '\\''hello'\\'''");
    }
}
