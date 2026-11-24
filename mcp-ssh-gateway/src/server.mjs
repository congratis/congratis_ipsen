import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { appendFileSync } from "node:fs";

const PROTOCOL_VERSION = "2025-06-18";
const MAX_LINE_BYTES = 1024 * 1024;
const MAX_OUTPUT_BYTES = boundedInt(process.env.SSH_MAX_OUTPUT_BYTES, 131072, 4096, 1048576);
const COMMAND_TIMEOUT_MS = boundedInt(process.env.SSH_COMMAND_TIMEOUT_MS, 15000, 1000, 60000);
const SERVICES = new Set(["app", "nginx", "rethinkdb"]);
const AUDIT_FILE = process.env.AUDIT_FILE || "";

function boundedInt(value, fallback, min, max) {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isInteger(parsed) && parsed >= min && parsed <= max ? parsed : fallback;
}

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) throw new Error(`Missing required environment variable: ${name}`);
  return value;
}

const config = {
  host: requiredEnv("SSH_HOST"),
  user: requiredEnv("SSH_USER"),
  port: boundedInt(process.env.SSH_PORT, 22, 1, 65535),
  identityFile: requiredEnv("SSH_IDENTITY_FILE"),
  knownHostsFile: requiredEnv("SSH_KNOWN_HOSTS_FILE"),
};

if (!/^[a-zA-Z0-9.-]+$/.test(config.host) ||
    !/^[a-z_][a-z0-9_-]{0,31}$/.test(config.user)) {
  throw new Error("SSH_HOST or SSH_USER contains unsupported characters");
}

function audit(event, details = {}) {
  const record = JSON.stringify({ at: new Date().toISOString(), event, ...details });
  process.stderr.write(record + "\n");
  if (AUDIT_FILE) {
    try { appendFileSync(AUDIT_FILE, record + "\n", { mode: 0o600 }); }
    catch (error) { process.stderr.write(JSON.stringify({ event: "audit_write_failed", message: error.message }) + "\n"); }
  }
}

const tools = [
  {
    name: "host_overview",
    description: "Read-only host overview: uptime, memory and CPU count.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "disk_usage",
    description: "Read-only disk usage for the root and /opt filesystems.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "compose_status",
    description: "Read-only Docker Compose status for /opt/congratis.",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "service_logs",
    description: "Read the last 1-200 lines for one allowlisted Congratis service.",
    inputSchema: {
      type: "object",
      properties: {
        service: { type: "string", enum: ["app", "nginx", "rethinkdb"] },
        lines: { type: "integer", minimum: 1, maximum: 200, default: 50 },
      },
      required: ["service"],
      additionalProperties: false,
    },
  },
];

const REMOTE_COMMANDS = {
  host_overview: "printf '%s\\n' '== UPTIME ==' ; uptime ; printf '%s\\n' '== MEMORY ==' ; free -h ; printf '%s\\n' '== CPU COUNT ==' ; nproc",
  disk_usage: "df -h / /opt",
  compose_status: "cd /opt/congratis && docker compose -f docker-compose.yml ps",
};

function ssh(command, toolName) {
  return new Promise((resolve, reject) => {
    const args = [
      "-T",
      "-p", String(config.port),
      "-i", config.identityFile,
      "-o", "BatchMode=yes",
      "-o", "IdentitiesOnly=yes",
      "-o", "StrictHostKeyChecking=yes",
      "-o", `UserKnownHostsFile=${config.knownHostsFile}`,
      "-o", "ConnectTimeout=8",
      "-o", "ServerAliveInterval=15",
      "-o", "ServerAliveCountMax=2",
      `${config.user}@${config.host}`,
      command,
    ];

    const child = spawn("ssh", args, { stdio: ["ignore", "pipe", "pipe"], shell: false });
    let stdout = "";
    let stderr = "";
    let total = 0;
    let settled = false;

    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolve(result);
    };

    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish(new Error("SSH command timed out"));
    }, COMMAND_TIMEOUT_MS);

    child.stdout.on("data", chunk => {
      total += chunk.length;
      if (total > MAX_OUTPUT_BYTES) {
        child.kill("SIGKILL");
        finish(new Error("SSH output exceeded configured limit"));
        return;
      }
      stdout += chunk.toString("utf8");
    });
    child.stderr.on("data", chunk => {
      if (stderr.length < 8192) stderr += chunk.toString("utf8").slice(0, 8192 - stderr.length);
    });
    child.on("error", error => finish(new Error(`Unable to start SSH: ${error.message}`)));
    child.on("close", code => {
      if (code === 0) finish(null, stdout || "(no output)");
      else finish(new Error(`SSH command failed (exit ${code ?? "unknown"}): ${stderr.trim().slice(0, 1000)}`));
    });
    audit("ssh_command_started", { tool: toolName, target: config.host });
  });
}

async function callTool(name, args) {
  if (!args || typeof args !== "object" || Array.isArray(args)) throw new Error("Tool arguments must be an object");

  let command = REMOTE_COMMANDS[name];
  if (name === "service_logs") {
    if (Object.keys(args).some(key => !["service", "lines"].includes(key))) throw new Error("Unexpected service_logs argument");
    const { service, lines = 50 } = args;
    if (!SERVICES.has(service)) throw new Error("Unsupported service");
    if (!Number.isInteger(lines) || lines < 1 || lines > 200) throw new Error("lines must be an integer from 1 to 200");
    command = `cd /opt/congratis && docker compose -f docker-compose.yml logs --no-color --tail=${lines} ${service}`;
  } else if (!command || Object.keys(args).length !== 0) {
    throw new Error("Unknown tool or unexpected arguments");
  }

  audit("tool_call", { tool: name, service: name === "service_logs" ? args.service : undefined });
  const output = await ssh(command, name);
  return { content: [{ type: "text", text: output }] };
}

function send(message) {
  process.stdout.write(JSON.stringify(message) + "\n");
}

async function handle(message) {
  if (!message || typeof message !== "object" || Array.isArray(message)) return;
  const { id, method, params = {} } = message;
  if (typeof method !== "string") return;

  try {
    let result;
    switch (method) {
      case "initialize":
        result = {
          protocolVersion: PROTOCOL_VERSION,
          capabilities: { tools: {} },
          serverInfo: { name: "congratis-mcp-ssh-gateway", version: "0.1.0" },
          instructions: "Read-only diagnostics only. No arbitrary shell, file writes, deployments, or service mutations are exposed.",
        };
        break;
      case "notifications/initialized":
      case "notifications/cancelled":
        return;
      case "ping":
        result = {};
        break;
      case "tools/list":
        result = { tools };
        break;
      case "tools/call": {
        const name = params.name;
        try {
          result = await callTool(name, params.arguments ?? {});
        } catch (error) {
          audit("tool_call_failed", { tool: name, message: error.message });
          result = { isError: true, content: [{ type: "text", text: error.message }] };
        }
        break;
      }
      default:
        if (id === undefined) return;
        send({ jsonrpc: "2.0", id, error: { code: -32601, message: "Method not found" } });
        return;
    }
    if (id !== undefined) send({ jsonrpc: "2.0", id, result });
  } catch (error) {
    if (id !== undefined) send({ jsonrpc: "2.0", id, error: { code: -32603, message: error.message } });
  }
}

const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on("line", line => {
  if (Buffer.byteLength(line, "utf8") > MAX_LINE_BYTES) {
    audit("protocol_line_rejected", { reason: "line_too_large" });
    return;
  }
  let message;
  try { message = JSON.parse(line); }
  catch {
    send({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "Parse error" } });
    return;
  }
  void handle(message);
});
input.on("close", () => process.exit(0));
audit("gateway_started", { host: config.host, user: config.user });
