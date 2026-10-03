# Blender MCP in hfx

[General MCP setup](mcp.md) · [Example configuration](../examples/blender-mcp/hfx.json) · [Demo scene source](../examples/blender-mcp/scene.py)

This integration uses the third-party [MCP for Blender](https://github.com/ahujasid/mcp-for-blender) project (formerly `blender-mcp`), not an official Blender Foundation add-on.

## Installed setup on this machine

- **Blender 4.5.14 LTS**, downloaded from [Blender's official release directory](https://download.blender.org/release/Blender4.5/) and verified against its published SHA-256 checksum.
- Portable installation: `~/.local/opt/blender-4.5.14-linux-x64/`; command: `~/.local/bin/blender`.
- Applications-menu entry: **Blender (MCP ready)**.
- Add-on: `~/.config/blender/4.5/scripts/addons/blender_mcp.py`, installed from **`mcp-for-blender==2.1.3`** and enabled in Blender's saved preferences.
- Blender's add-on socket: **`127.0.0.1:9876`**. Blender must remain open; this is a GUI add-on, not a headless rendering daemon.
- hfx **Settings → Tools → MCP servers** is configured and enabled. Agent tools and Shell commands were already enabled, with Trusted host mode selected. The existing chat history was not edited to configure this: changes were made through the running UI and verified in its saved preferences.
- hfx's **Test MCP servers** discovered **36 tools**.

The executable path in the example JSON is specific to this Linux machine. On another machine, replace `/usr/bin/uvx` with the result of `command -v uvx`, or the equivalent absolute executable path for that platform.

```json
{
  "mcpServers": {
    "blender": {
      "command": "/usr/bin/uvx",
      "args": ["--python", "3.12", "mcp-for-blender==2.1.3"],
      "env": {
        "BLENDER_HOST": "127.0.0.1",
        "BLENDER_PORT": "9876",
        "BLENDER_MCP_DISABLE_TELEMETRY": "1",
        "BLENDER_MCP_SAFE_MODE": "1"
      },
      "timeoutSecs": 120
    }
  }
}
```

Telemetry is disabled both in the MCP server environment and in Blender's add-on preferences. Poly Haven, Sketchfab, Hyper3D, Hunyuan3D, Tripo, and Poly Pizza integrations are off; this demonstration uses only Blender primitives, with no external assets or paid APIs.

**Safe mode is an additional third-party script validator, not OS confinement.** Normal modeling/rendering/saving remains available, and Blender/MCP still runs with your host permissions. Keep the socket local. Use hfx's **Review each tool action** if you want to approve each modeling action; the setup did not change your existing review-mode preference. Strict sandbox mode blocks MCP entirely.

## Try it in chat

Settings changes apply to the **next task**. With Blender open, send a new message in hfx, for example:

> Use Blender MCP to inspect the current scene. Change the robot's eyes and antenna light to warm orange without altering anything else, save a new copy, and show me a viewport screenshot.

hfx will initialize the stdio MCP server, discover its tools, and expose stable `mcp_blender_…` aliases to the selected model. The server sends Blender operations to the local add-on. Blender updates its live scene, then MCP returns text and/or images. Calls appear in the normal action history; review mode uses the normal approval dialog.

Useful upstream tools include `get_scene_info`, `get_object_info`, `execute_blender_code`, `get_viewport_screenshot`, `describe_node_type`, and `bpy_api_lookup`. The scene summary includes only the first ten objects; use object inspection for details about other objects.

To reopen the saved demonstration:

```bash
blender /home/lord/Documents/hfx/artifacts/blender-mcp/demo.blend
```

Only run one Blender instance on port 9876. If Blender is already open, use **File → Open** instead, or close it before running that command. The add-on normally auto-starts when Blender opens; **N → MCP for Blender → Start MCP Server** is the manual alternative.

## Verified real-server demonstration

The opt-in Rust demonstration uses **hfx's actual MCP `Session` client** against the installed server and running Blender, not a mock:

1. Discover 36 tools and inspect the existing scene.
2. Execute the [scene script](../examples/blender-mcp/scene.py) to create a purple robot holding an MCP cube, with a presentation stage, camera, and studio lights.
3. Capture a real viewport image through `get_viewport_screenshot` and decode it with hfx's image-result handling.
4. Render a 1280 × 1000 PNG through `execute_blender_code`.
5. Inspect the final scene (44 objects) and the robot head, then verify the saved files.

Artifacts are in `artifacts/blender-mcp/`:

| File | Contents |
| --- | --- |
| `demo.blend` | Editable scene, camera, lights, and materials. |
| `render.png` | Final Cycles render. |
| `viewport.png` | Image returned by an actual MCP tool call. |
| `mcp-transcript.json` | Tool aliases, arguments, results, timings, and image counts for the six calls. |
| `discovered-tools.json` | Actual discovered schemas in hfx's Responses-compatible format. |
| `hfx-discovery.png` | Screenshot of the running Settings panel showing successful discovery. |
| `setup/` | Installation/readiness diagnostics and live-demo logs. |

This developer demonstration calls the Rust MCP client directly. It does **not** send an inference-provider request or insert synthetic actions into your conversation. Your next normal chat task uses the regular provider, approval, and action-history flow.

### Repeat the demo

**This command modifies the open Blender scene and rewrites the demonstration files.** The script refuses an unrelated `.blend` file or an unsaved scene containing custom objects. Open a new default scene, or the existing demo, before running it. It does not connect or create files during ordinary `cargo test` runs.

```bash
mkdir -p artifacts/blender-mcp
cp examples/blender-mcp/hfx.json artifacts/blender-mcp/mcp-config.json
cargo test --locked demo_blender_mcp -- --ignored --nocapture
```

Generated artifacts, logs, and `.blend` backups are ignored by Git. The reusable scene source and configuration are under `examples/blender-mcp/`.

## Troubleshooting

- **Connection refused:** open Blender and confirm the add-on's server is running on port 9876. hfx's discovery test checks MCP metadata, not every Blender operation.
- **Cannot find uvx:** use its absolute executable path. The Python 3.12/package installation has been warmed up, so new tasks do not need a cold package download inside the 15-second initialization timeout.
- **Safe-mode rejection:** rewrite the modeling script within the validator's limits. Do not disable safe mode merely to bypass an unexpected error.
- **Long render:** reduce resolution/samples or deliberately increase `timeoutSecs`. Do not blindly repeat a timed-out operation; the Blender scene or output may already have changed.
- **Missing interactive viewport app:** hfx supports MCP image results, not the project's optional embedded MCP Apps/HTML widgets. Request a viewport screenshot instead.
