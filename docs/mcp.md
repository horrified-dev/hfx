# MCP servers

[README](../README.md) · [Tools & security](tools.md) · [Providers](providers.md)

hfx can use tools from **Model Context Protocol (MCP)** servers with Codex, OpenAI API, OpenRouter, and llama.cpp. MCP is **off by default** and configured explicitly in **Settings → Tools → MCP servers**. Project files cannot automatically register or start servers.

## Setup

1. Install the local MCP server, or obtain a remote server's Streamable HTTP endpoint.
2. Paste a configuration into the **Server configuration · JSON** editor.
3. Enable **agent tools** and **MCP**. For local stdio servers, also enable **Shell commands**. Use **Trusted host** mode; MCP is blocked in strict sandbox mode.
4. Click **Test MCP servers** to start/connect enabled servers and list their tools. This does **not** invoke tools, but starting a local server executes its code. Cancel the test to stop discovery.
5. Send a request to an inference provider. Enabled servers are initialized and discovered at the beginning of each task. Demo mode does not connect to MCP servers.

For a complete installed integration and a real scene-building demonstration, see [Blender MCP in hfx](blender-mcp.md).

### Local server (stdio)

This example starts the filesystem MCP server with access to the specified directory. Replace the path with a directory you intend to expose. `npx` must be available in the environment used to launch hfx; first use may download the package.

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "/absolute/path/to/project"
      ],
      "timeoutSecs": 120
    }
  }
}
```

`command` is an executable, not a shell command string. Arguments are passed separately, without shell expansion. The working directory is the currently selected project, and the server inherits hfx's normal host environment. Use an absolute executable path if a desktop launcher cannot find it. On Windows, use the executable/shim name installed for your platform (for example `npx.cmd`).

Optional environment overrides accept literal non-secret values or a whole-value `${VARIABLE}` reference:

```json
{
  "mcpServers": {
    "custom": {
      "command": "/absolute/path/to/my-mcp-server",
      "args": [],
      "env": {
        "SERVICE_TOKEN": "${MY_SERVICE_TOKEN}",
        "LOG_LEVEL": "warn"
      }
    }
  }
}
```

Set `MY_SERVICE_TOKEN` in the environment **before launching hfx**. References are resolved at connection time; their resolved values are not written into preferences. There is no shell evaluation, interpolation within longer strings, or automatic `.env` loading.

### Remote server (Streamable HTTP)

```json
{
  "mcpServers": {
    "remote": {
      "url": "https://mcp.example.com/mcp",
      "bearerTokenEnv": "MY_MCP_TOKEN",
      "timeoutSecs": 120
    }
  }
}
```

`bearerTokenEnv` is optional and names an environment variable containing the bearer token, **without** the `Bearer ` prefix. Omit it for an anonymous endpoint. Provider API keys and Codex credentials are not sent to MCP servers. HTTP is permitted only on `localhost`, `127.0.0.1`, or `::1`; remote endpoints require HTTPS. Redirects and credentials/query parameters/fragments in URLs are rejected. The complete endpoint path must be supplied.

### Configuration fields

| Field | Meaning |
| --- | --- |
| `mcpServers` | Object mapping unique server names to configurations. Required at the top level. |
| `enabled` | Optional per-server switch, default `true`. Set `false` to keep a configuration without connecting. |
| `command` | Local executable for stdio. Provide exactly one of `command` or `url`. |
| `args` | Array of strings, optional; stdio only. |
| `env` | Environment overrides, optional; stdio only. Prefer `${VARIABLE}` references for secrets. |
| `url` | Full Streamable HTTP endpoint. |
| `bearerTokenEnv` | Environment variable containing an HTTP bearer token, optional; HTTP only. |
| `timeoutSecs` | Tool-call timeout, 1–7200 seconds; default 120. Initialization and discovery each have a separate 15-second timeout per server. |

Configuration is saved with settings, including an unfinished/invalid editor draft. Invalid configuration is shown inline and blocks new MCP-enabled tasks. Unsupported fields are rejected rather than silently ignored. Settings changes apply to the **next task**, not an active task. Clear `mcpServers` to `{}` or disable MCP to remove external tools.

## Tool behavior

- The official Rust MCP SDK handles initialization, version negotiation, notifications, JSON-RPC, stdio, and Streamable HTTP (JSON and SSE responses).
- Tool discovery follows pagination. Each discovered tool gets a stable, provider-compatible `mcp_…` name with a collision-resistant suffix. Original names are preserved when calling the server; tools cannot replace native `read_file`, `run_command`, etc.
- MCP input schemas are preserved, including optional fields. Responses tools use `strict: false` because arbitrary MCP schemas do not necessarily satisfy OpenAI's strict-schema subset.
- Calls appear in the existing **action history**, with arguments, results, timing, and failure/decline/cancellation status. **Review each tool action** requires approval for every MCP call, even if the server labels it read-only. MCP calls stay sequential; server annotations cannot authorize parallel side effects.
- Text, structured results, embedded text resources, and resource links are returned as reference data. Supported image results reach the model and appear in the action history; they are not automatically published as reply attachments. Audio and binary resource blobs are omitted with an explicit notice.
- Tool results retain `isError`; a server-reported error becomes a failed action, not a successful one. Returned text is truncated explicitly at 64 KiB. Images use the usual 10 MiB/32-megapixel limits and at most eight images per call.
- Connections belong to one task. Completion, errors, or Stop close them; local processes are terminated (their process group on Unix, the direct child on Windows). Tool timeouts close the affected connection and do not automatically retry calls. **Remote operations may still finish after cancellation**; check the outcome before retrying an action with side effects.
- An enabled server that fails initialization/discovery stops the task before the first model request. Fix it or set that server's `enabled` to `false`; hfx does not silently continue with missing tools.

Limits: 16 configured servers, 64 KiB configuration, 119 enabled MCP tools in total (leaving room for nine native tools within the providers' 128-tool limit), and 256 KiB tool arguments. Server/tool schemas are subject to the model endpoint's own compatibility requirements.

## Security and current scope

**MCP servers are trusted external programs/services, not workspace-confined file tools.** Local servers run as your user, can read inherited credentials, and can modify files outside the project. Remote servers receive the arguments supplied to their tools. An allowlisted directory is only as reliable as that server's implementation. Review mode approves individual tool calls, **not initialization or server startup**.

MCP is blocked when **Strict workspace sandbox** is selected rather than falling back to unconfined execution. Disabling Shell commands also prevents starting stdio servers; it does not prevent explicitly enabled remote HTTP tools. Disabling agent tools or MCP prevents all MCP connections, including discovery.

Server descriptions, instructions, annotations, and results are untrusted reference data. Server instructions are not promoted into the assistant's system prompt. Review mode is enforced by the harness rather than by the model or server.

**Configuration is plaintext.** Do not paste literal credentials into `env`, command arguments, or URLs. Literal values entered into the JSON editor are persisted exactly as written. Use environment references for secrets. Tool arguments/results are also saved in plaintext chat history and may contain sensitive information returned by a server. Do not publish that history indiscriminately.

This first implementation supports **tools**, not a prompts/resources browser or standalone `resources/read` tools. It does not advertise sampling, roots, or elicitation capabilities. OAuth/browser login, arbitrary authentication headers, legacy HTTP+SSE endpoints, and sandboxed MCP execution are not implemented. For HTTP use anonymous access or `bearerTokenEnv`; servers that require unsupported client capabilities cannot use them in hfx yet.

## Troubleshooting

- **Cannot start stdio server:** install its executable/runtime and ensure hfx's launch environment can find it. Server stderr is inherited by hfx; launch hfx from a terminal to inspect diagnostics. Do not write logs to server stdout, which carries MCP messages.
- **Missing environment variable:** set it before starting hfx, then restart. Keys entered only in provider settings are session-only and are not exported to servers.
- **Initialization/discovery failure:** check the endpoint, supported transport, server logs, and authentication. Transport error details are intentionally kept generic to avoid echoing credentials from server headers/error payloads.
- **Timeout:** increase `timeoutSecs` only when appropriate. The call is not retried; verify whether it already changed external state.
- **Provider rejects a schema/tool count:** reduce enabled servers/tools or use a model/endpoint supporting those schemas. Discovery checks the harness's limits, not every provider-specific schema restriction.

Implementation and regression coverage: [`src/mcp.rs`](../src/mcp.rs), [`src/mcp_ui.rs`](../src/mcp_ui.rs), [`src/mcp_tests.rs`](../src/mcp_tests.rs), and [`src/backend_mcp_tests.rs`](../src/backend_mcp_tests.rs).
