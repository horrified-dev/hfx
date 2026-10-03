# Provider setup

[README](../README.md) · [Providers](providers.md) · [Usage](usage.md) · [Tools & security](tools.md) · [Linux](linux.md) · [Development](development.md)

hfx starts in **Demo** mode, which uses scripted output and needs no credentials. Choose one of the four inference providers below to work with a model. Tool, reasoning, and image capabilities depend on the selected model and endpoint.

## Connect a provider

Open **Settings → Providers** from the gear icon, workspace menu, or `Ctrl+,` (`Cmd+,` on macOS).

### Codex sign-in

1. Select **Codex**, then click **Sign in with OpenAI**.
2. Complete sign-in in the browser. hfx receives the callback at `http://localhost:1455/auth/callback`, validates OAuth state, and exchanges the authorization code with its PKCE verifier.
3. Choose a discovered model, then send a message. The default is `gpt-6.1-sol`; access and reasoning effort depend on your ChatGPT account.

This is built-in OpenAI authentication in Rust, based on the flow in [this OAuth reference implementation](https://github.com/7shi/codex-oauth/blob/main/codex_oauth.py). It uses authorization code + PKCE (S256), the public Codex client ID, and `openid profile email offline_access`. hfx refreshes tokens before expiry and retries once after HTTP 401.

Responses requests carry the access token and `ChatGPT-Account-Id` header. They use `store: false`, explicit input text content, reasoning summaries, and a stable chat cache key. Conversation/tool items and encrypted reasoning context are retained for later turns. The backend uses the current `https://chatgpt.com/backend-api/codex` route; the sample uses the older WHAM route. These backend details are not a versioned public API and can change.

Tool calls are assembled from streamed output items and function argument events, even when the terminal response omits its output array. Repeated terminal items are merged by identity so each tool runs once. Tools execute after successful stream completion; incomplete responses do not trigger execution. Codex's `response.done` terminal event is also accepted.

hfx manages its own login. OAuth credentials are stored separately from chats in `codex-auth.json` in hfx's application data directory. On Unix, its directory is mode 0700 and the file is mode 0600. This file contains bearer credentials in plaintext. **Sign out of hfx** removes that cache. Cancel closes the pending callback listener. No Codex CLI installation is required.

### OpenAI API

1. Select **OpenAI API**.
2. Enter an API key, or launch with `OPENAI_API_KEY` in the environment.
3. Set a Responses-compatible reasoning model ID available to your account. The configured default is `gpt-6.1-sol`. Codex model IDs can also be entered directly.
4. Click **Test connection** to discover models, then choose one.

Requests use `/v1/responses`, SSE streaming, configurable reasoning effort, and `reasoning.summary = "auto"` when reasoning is visible. OpenAI provides **reasoning summaries**; raw internal reasoning is not exposed. Summary availability depends on the model and account. API billing and model access are separate from ChatGPT subscription access.

The endpoint is configurable for compatible gateways. Include `/v1` in the base URL. Remote OpenAI endpoints must use HTTPS. Requests use `store: false`; encrypted reasoning items are preserved within tool rounds.

### OpenRouter

1. Select **OpenRouter**.
2. Enter your OpenRouter API key, or launch with `OPENROUTER_API_KEY` in the environment.
3. Use `https://openrouter.ai/api/v1` and a `provider/model` ID available to your account. **Test connection** discovers available models. The default is `openai/gpt-6-luna`.

Requests use Chat Completions with SSE streaming, configurable reasoning effort, tools, temperature, and output limits. hfx renders `delta.reasoning` and textual reasoning details. Assistant tool calls, tool results, encrypted reasoning details, and the final answer are saved and replayed on subsequent user turns. Earlier chats also recover their recorded action outputs as reference context. Reasoning and tool support vary by model and provider. Usage is billed through your OpenRouter account.

### llama.cpp

Start a current llama.cpp server with a model that supports chat and, if wanted, reasoning/tools:

```bash
llama-server -m /path/to/model.gguf --alias local-model \
  --host 127.0.0.1 --port 8080 --jinja --reasoning-format deepseek
```

Select **llama.cpp**, use `http://127.0.0.1:8080/v1`, and test the connection. Match the model ID to the server alias. An API key is optional; `LLAMA_API_KEY` is also supported.

Requests use `/v1/chat/completions` and stream `delta.content` separately from `delta.reasoning_content`. Reasoning effort, tool calls, and reasoning extraction depend on the chosen model's chat template and server version. Temperature and output limits are configurable.

For context-window settings and automatic compaction, see [Automatic context compaction](usage.md#automatic-context-compaction).
