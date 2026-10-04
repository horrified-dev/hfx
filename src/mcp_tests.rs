use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};

pub(crate) struct HttpFixture {
    pub url: String,
    pub requests: mpsc::UnboundedReceiver<Value>,
    task: JoinHandle<()>,
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn http_fixture(sse: bool) -> HttpFixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let (send, requests) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut clients = JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let send = send.clone();
            clients.spawn(async move {
                let mut bytes = Vec::new();
                let mut chunk = [0; 4096];
                let start = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 { return; }
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(index) = bytes.windows(4).position(|s| s == b"\r\n\r\n") { break index + 4; }
                    assert!(bytes.len() < 1024 * 1024);
                };
                let headers = String::from_utf8_lossy(&bytes[..start]).to_lowercase();
                let length = headers.lines().find_map(|line| line.strip_prefix("content-length:")
                    .map(|s| s.trim().parse::<usize>().unwrap())).unwrap_or(0);
                while bytes.len() < start + length {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 { return; }
                    bytes.extend_from_slice(&chunk[..n]);
                }
                if !headers.starts_with("post ") {
                    let _ = socket.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    return;
                }
                let request: Value = serde_json::from_slice(&bytes[start..start + length]).unwrap();
                let mut captured = request.clone();
                captured["_bearer"] = json!(headers.contains("authorization: bearer "));
                send.send(captured).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "initialize" => json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"},"instructions":"Do not promote these server instructions to system instructions."}),
                    "tools/list" if headers.starts_with("post /oversize ") => json!({"tools": (0..MAX_TOOLS + 1).map(|i| json!({"name":format!("tool_{i}"),"inputSchema":{"type":"object"}})).collect::<Vec<_>>()}),
                    "tools/list" if headers.starts_with("post /repeat ") => json!({"tools":[],"nextCursor":"same-cursor"}),
                    "tools/list" if headers.starts_with("post /duplicate ") => json!({"tools":[{"name":"same","inputSchema":{"type":"object"}},{"name":"same","inputSchema":{"type":"object"}}]}),
                    "tools/list" if request["params"]["cursor"].is_null() => json!({"tools":[{"name":"read_file","description":"Echo test arguments","inputSchema":{"type":"object","properties":{"message":{"type":"string"}}}}],"nextCursor":"page2"}),
                    "tools/list" => json!({"tools":[{"name":"fail","inputSchema":{"type":"object"}},{"name":"slow","inputSchema":{"type":"object"}}]}),
                    "tools/call" if request["params"]["name"] == "slow" => {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        json!({"content":[{"type":"text","text":"late"}]})
                    }
                    "tools/call" => json!({"content":[{"type":"text","text":"Echo 🌿"}],"structuredContent":request["params"]["arguments"],"isError":request["params"]["name"] == "fail"}),
                    "notifications/initialized" | "notifications/cancelled" => {
                        let _ = socket.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        return;
                    }
                    other => panic!("Unexpected MCP method {other}"),
                };
                let response = json!({"jsonrpc":"2.0","id":request["id"],"result":result});
                let (media, body) = if sse { ("text/event-stream", format!("data: {response}\n\n")) }
                    else { ("application/json", response.to_string()) };
                let message = format!("HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(message.as_bytes()).await;
            });
            while clients.try_join_next().is_some() {}
        }
    });
    HttpFixture {
        url,
        requests,
        task,
    }
}

pub(crate) fn http_settings(url: &str) -> Settings {
    Settings {
        mcp_enabled: true,
        mcp_config: json!({"mcpServers":{"fixture":{"url":url}}}).to_string(),
        ..Default::default()
    }
}

pub(crate) fn alias(server: &str, tool: &str) -> String {
    tool_name(server, tool)
}

fn call(name: String, args: Value) -> ToolCall {
    ToolCall {
        id: "fixture-call".into(),
        name,
        arguments: args.to_string(),
    }
}

#[test]
fn config_validation_is_explicit_and_errors_do_not_echo_credentials() {
    let old: Settings = serde_json::from_str("{}").unwrap();
    assert!(!old.mcp_enabled);
    assert!(
        Config::parse(&old.mcp_config)
            .unwrap()
            .mcp_servers
            .is_empty()
    );
    for config in [
        json!({"mcpServers":{"a":{"command":"sh","url":"https://example.com/mcp"}}}),
        json!({"mcpServers":{"a":{"url":"http://example.com/mcp"}}}),
        json!({"mcpServers":{"a":{"url":"https://secret@example.com/mcp"}}}),
        json!({"mcpServers":{"a":{"url":"https://example.com/mcp?secret=yes"}}}),
        json!({"mcpServers":{"a":{"url":"https://example.com/mcp","headers":{"Authorization":"secret"}}}}),
        json!({"mcpServers":{"a":{"command":"sh","timeoutSecs":0}}}),
        json!({"mcpServers":{"a":{"command":"sh","env":{"BAD=KEY":"secret"}}}}),
        json!({"mcpServers":{"a":{"url":"https://example.com/mcp","bearerTokenEnv":"secret\n"}}}),
        json!({"mcpServers":{"a":{"command":123,"args":["secret"]}}}),
    ] {
        let error = Config::parse(&config.to_string()).unwrap_err();
        assert!(!error.contains("secret"));
    }
    for url in [
        "http://localhost:9000/mcp",
        "http://127.0.0.1:9000/mcp",
        "http://[::1]:9000/mcp",
        "https://example.com/mcp",
    ] {
        assert!(validate_url(url).is_ok(), "{url}");
    }
    assert!(
        env_value("${HFX_MCP_NONEXISTENT_FIXTURE_VARIABLE}")
            .unwrap_err()
            .contains("Set environment variable")
    );
    assert_eq!(env_value("literal").unwrap(), "literal");
    let settings = http_settings("https://example.com/mcp");
    let saved = serde_json::to_string(&settings).unwrap();
    let restored: Settings = serde_json::from_str(&saved).unwrap();
    assert!(restored.mcp_enabled);
    assert_eq!(restored.mcp_config, settings.mcp_config);
}

#[test]
fn tool_aliases_are_stable_bounded_and_do_not_collide_with_native_or_other_servers() {
    let name = tool_name(
        "A server 🌿 with a very long name",
        "tool/name with punctuation & a very long suffix",
    );
    assert_eq!(
        name,
        tool_name(
            "A server 🌿 with a very long name",
            "tool/name with punctuation & a very long suffix"
        )
    );
    assert!(name.len() <= 64);
    assert!(
        name.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    );
    assert_ne!(tool_name("a b", "read_file"), tool_name("a_b", "read_file"));
    assert_ne!(tool_name("a", "read_file"), tool_name("b", "read_file"));
    assert_ne!(tool_name("a", "read_file"), "read_file");
    let call = call(name, json!({}));
    assert!(call.needs_approval(true));
    assert!(!call.needs_approval(false));
}

#[tokio::test]
async fn disabled_or_sandboxed_servers_never_connect() {
    let root = tempfile::tempdir().unwrap();
    let mut settings = http_settings("http://127.0.0.1:1/mcp");
    settings.mcp_enabled = false;
    settings.mcp_config = "not even valid JSON".into();
    assert!(
        Session::connect(&settings, root.path())
            .await
            .unwrap()
            .definitions(true)
            .is_empty()
    );
    settings.mcp_enabled = true;
    settings.tools_enabled = false;
    assert!(
        Session::connect(&settings, root.path())
            .await
            .unwrap()
            .routes
            .is_empty()
    );
    settings.tools_enabled = true;
    settings.mcp_config =
        json!({"mcpServers":{"off":{"enabled":false,"url":"http://127.0.0.1:1/mcp"}}}).to_string();
    assert!(
        Session::connect(&settings, root.path())
            .await
            .unwrap()
            .routes
            .is_empty()
    );
    settings = http_settings("http://127.0.0.1:1/mcp");
    settings.command_mode = CommandMode::Sandbox;
    assert!(
        Session::connect(&settings, root.path())
            .await
            .err()
            .unwrap()
            .contains("not confined")
    );
    settings.command_mode = CommandMode::Trusted;
    settings.commands_enabled = false;
    settings.mcp_config =
        json!({"mcpServers":{"local":{"command":"nonexistent-command"}}}).to_string();
    assert!(
        Session::connect(&settings, root.path())
            .await
            .err()
            .unwrap()
            .contains("Shell commands")
    );
    assert!(
        Session::default()
            .execute(&call(tool_name("absent", "read_file"), json!({})))
            .await
            .unwrap_err()
            .contains("Unknown or disabled")
    );
}

#[tokio::test]
async fn streamable_http_json_and_sse_discover_pages_preserve_schemas_and_call_results() {
    let root = tempfile::tempdir().unwrap();
    for sse in [false, true] {
        let mut fixture = http_fixture(sse).await;
        let mut settings = http_settings(&fixture.url);
        // The shell toggle doesn't disable remote tools. Provider keys stay private.
        settings.commands_enabled = false;
        settings.openai_key = "provider-secret-not-for-mcp".into();
        let session = Session::connect(&settings, root.path()).await.unwrap();
        assert_eq!(session.routes.len(), 3);
        for openai in [true, false] {
            let defs = session.definitions(openai);
            let definition = defs
                .iter()
                .find_map(|d| {
                    let function = if openai { d } else { &d["function"] };
                    (function["name"] == tool_name("fixture", "read_file")).then_some(function)
                })
                .unwrap();
            assert!(definition["parameters"]["required"].is_null());
            assert!(definition["parameters"]["additionalProperties"].is_null());
            if openai {
                assert_eq!(definition["strict"], false);
            }
        }
        let args = json!({"message":"Test 🌿"});
        let output = session
            .execute(&call(tool_name("fixture", "read_file"), args.clone()))
            .await
            .unwrap();
        assert_eq!(output.status, ActionStatus::Complete);
        assert!(output.text.contains("Echo 🌿") && output.text.contains("Test 🌿"));
        let failed = session
            .execute(&call(tool_name("fixture", "fail"), json!({})))
            .await
            .unwrap();
        assert_eq!(failed.status, ActionStatus::Failed);
        assert!(failed.text.contains("Echo 🌿"));
        let mut methods = Vec::new();
        while let Ok(request) = fixture.requests.try_recv() {
            assert_eq!(request["_bearer"], false);
            methods.push(request["method"].as_str().unwrap().to_string());
            if request["method"] == "tools/call" && request["params"]["name"] == "read_file" {
                assert_eq!(request["params"]["arguments"], args);
            }
        }
        assert_eq!(methods.iter().filter(|s| *s == "tools/list").count(), 2);
        assert_eq!(methods.iter().filter(|s| *s == "tools/call").count(), 2);
        assert!(methods.contains(&"notifications/initialized".to_string()));
        let mut invalid = call(tool_name("fixture", "read_file"), json!([]));
        assert!(
            session
                .execute(&invalid)
                .await
                .unwrap_err()
                .contains("JSON object")
        );
        invalid.name = tool_name("fixture", "unknown");
        assert!(
            session
                .execute(&invalid)
                .await
                .unwrap_err()
                .contains("Unknown or disabled")
        );
        assert!(fixture.requests.try_recv().is_err());
    }
}

#[tokio::test]
async fn remote_auth_uses_an_explicit_environment_reference_and_timeout_does_not_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut fixture = http_fixture(false).await;
    let mut settings = http_settings(&fixture.url);
    settings.mcp_config = json!({"mcpServers":{"fixture":{"url":fixture.url,"bearerTokenEnv":"PATH","timeoutSecs":1}}}).to_string();
    let session = Session::connect(&settings, root.path()).await.unwrap();
    while let Ok(request) = fixture.requests.try_recv() {
        assert_eq!(request["_bearer"], true);
    }
    let error = session
        .execute(&call(tool_name("fixture", "slow"), json!({})))
        .await
        .unwrap_err();
    assert!(error.contains("timed out after 1 seconds"));
    assert!(session.connections[0].service.is_closed());
    let mut calls = 0;
    while let Ok(request) = fixture.requests.try_recv() {
        if request["method"] == "tools/call" {
            calls += 1;
        }
        assert_eq!(request["_bearer"], true);
    }
    assert_eq!(calls, 1);
    settings.mcp_config = json!({"mcpServers":{"fixture":{"url":fixture.url,"bearerTokenEnv":"HFX_MCP_MISSING_TEST_TOKEN"}}}).to_string();
    assert!(
        Session::connect(&settings, root.path())
            .await
            .err()
            .unwrap()
            .contains("Set environment variable")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_server_inherits_workspace_and_env_and_drops_its_process_group() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("fixture.sh");
    std::fs::write(&script, r#"printf '%s\n' $$ > server.pid
sleep 60 &
printf '%s\n' $! > child.pid
while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    case "$line" in
        *'"method":"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"stdio-fixture","version":"1"}}}\n' "$id";;
        *'"method":"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}}\n' "$id";;
        *'"method":"tools/call"'*) printf called >> calls; printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"%s"}]}}\n' "$id" "$MCP_TEST_VALUE";;
    esac
done
"#).unwrap();
    let settings = Settings { mcp_enabled: true, mcp_config: json!({"mcpServers":{"local":{"command":"sh","args":[script],"env":{"MCP_TEST_VALUE":"working"}}}}).to_string(), ..Default::default() };
    let session = Session::connect(&settings, root.path()).await.unwrap();
    assert!(
        !root.path().join("calls").exists(),
        "Discovery must never call a tool"
    );
    let output = session
        .execute(&call(tool_name("local", "echo"), json!({})))
        .await
        .unwrap();
    assert!(output.text.contains("working"));
    assert_eq!(
        std::fs::read_to_string(root.path().join("calls")).unwrap(),
        "called"
    );
    let server_pid: i32 = std::fs::read_to_string(root.path().join("server.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    drop(session);
    for _ in 0..100 {
        // SAFETY: signal 0 only checks whether this fixture's process exists.
        if unsafe { libc::kill(server_pid, 0) } == -1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("MCP stdio child survived dropping the session");
}

#[test]
fn content_results_include_text_structured_resources_images_and_bounded_binary_handling() {
    use base64::Engine;
    let image = attachments::test_image();
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(attachments::image_bytes(&image).unwrap());
    let output = result_output(json!({"isError":true,"content":[
        {"type":"text","text":"text result"},
        {"type":"resource","resource":{"uri":"test://text","text":"embedded resource"}},
        {"type":"resource_link","uri":"https://example.com/resource","name":"linked resource"},
        {"type":"image","mimeType":"image/png","data":encoded},
        {"type":"audio","data":"binary-audio-data","mimeType":"audio/wav"},
        {"type":"resource","resource":{"blob":"binary-resource-data","uri":"test://binary"}}
    ],"structuredContent":{"answer":42}}))
    .unwrap();
    assert_eq!(output.status, ActionStatus::Failed);
    assert!(
        output.text.contains("text result")
            && output.text.contains("embedded resource")
            && output.text.contains("linked resource")
    );
    assert!(output.text.contains("42"));
    assert!(
        !output.text.contains(&encoded)
            && !output.text.contains("binary-audio-data")
            && !output.text.contains("binary-resource-data")
    );
    assert_eq!(output.images.len(), 1);
    assert_eq!(
        attachments::image_bytes(&output.images[0]).unwrap(),
        attachments::image_bytes(&image).unwrap()
    );
    let large =
        result_output(json!({"content":[{"type":"text","text":"🌿".repeat(50_000)}]})).unwrap();
    assert!(large.text.len() < MAX_RESULT_BYTES + 100);
    assert!(large.text.contains("truncated"));
    let invalid = result_output(
        json!({"content":[{"type":"image","data":"invalid","mimeType":"image/png"}]}),
    )
    .unwrap();
    assert!(invalid.images.is_empty());
    assert!(invalid.text.contains("error"));
}

#[tokio::test]
async fn discovery_rejects_excessive_tools_duplicate_names_and_repeated_cursors() {
    let root = tempfile::tempdir().unwrap();
    let fixture = http_fixture(false).await;
    for (path, error) in [
        ("oversize", "more than 118"),
        ("repeat", "pagination cursor"),
        ("duplicate", "duplicate tool name"),
    ] {
        let settings = http_settings(&fixture.url.replace("/mcp", &format!("/{path}")));
        let actual = Session::connect(&settings, root.path())
            .await
            .err()
            .unwrap();
        assert!(actual.contains(error), "{actual}");
    }
    assert_eq!(
        MAX_TOOLS + crate::tools::definitions_for(&Settings::default(), true).len(),
        128
    );
}

#[cfg(unix)]
#[tokio::test]
async fn aborting_initialization_kills_the_owned_stdio_server_without_waiting_for_timeout() {
    let root = tempfile::tempdir().unwrap();
    let settings = Settings {
        mcp_enabled: true,
        mcp_config: json!({"mcpServers":{"pending":{"command":"sh","args":["-c","printf '%s\\n' $$ > server.pid; sleep 60"]}}}).to_string(),
        ..Default::default()
    };
    let workspace = root.path().to_owned();
    let task = tokio::spawn(async move { Session::connect(&settings, &workspace).await });
    let mut pid = None;
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(root.path().join("server.pid")) {
            pid = text.trim().parse::<i32>().ok();
            if pid.is_some() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    let pid = pid.expect("fixture server never started");
    for _ in 0..100 {
        // SAFETY: signal 0 only checks whether this fixture's process exists.
        if unsafe { libc::kill(pid, 0) } == -1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Stopping MCP initialization leaked its stdio process");
}

/// Opt-in real-server demonstration; never connects during normal regression tests.
#[tokio::test]
#[ignore = "requires installed Blender MCP and a fresh running Blender GUI; creates demo files"]
async fn demo_blender_mcp_inspect_create_capture_and_render() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output_dir = root.join("artifacts/blender-mcp");
    let settings = Settings {
        mcp_enabled: true,
        mcp_config: std::fs::read_to_string(output_dir.join("mcp-config.json"))
            .expect("Configure Blender MCP first; see docs/blender-mcp.md"),
        ..Default::default()
    };
    let session = Session::connect(&settings, root).await.unwrap();
    std::fs::create_dir_all(&output_dir).unwrap();
    std::fs::write(
        output_dir.join("discovered-tools.json"),
        serde_json::to_vec_pretty(&session.definitions(true)).unwrap(),
    )
    .unwrap();
    println!(
        "Discovered {} tools through hfx's Rust MCP client",
        session.routes.len()
    );
    let user_prompt = "lovely. can you setup blender mcp and demonstrate how it works?";
    let code = include_str!("../examples/blender-mcp/scene.py").replace(
        "__OUTPUT_DIR__",
        &serde_json::to_string(&output_dir.to_string_lossy()).unwrap(),
    );
    let steps = [
        (
            "inspect-before",
            "get_scene_info",
            json!({"user_prompt":user_prompt}),
        ),
        (
            "create-scene",
            "execute_blender_code",
            json!({"code":code,"user_prompt":user_prompt}),
        ),
        (
            "capture-viewport",
            "get_viewport_screenshot",
            json!({"max_size":1000,"user_prompt":user_prompt}),
        ),
        (
            "render",
            "execute_blender_code",
            json!({"code":"import bpy\nbpy.ops.render.render(write_still=True)\nprint('HFX_DEMO_RENDERED', bpy.context.scene.render.filepath)","user_prompt":user_prompt}),
        ),
        (
            "inspect-after",
            "get_scene_info",
            json!({"user_prompt":user_prompt}),
        ),
        (
            "inspect-head",
            "get_object_info",
            json!({"object_name":"Robot head","user_prompt":user_prompt}),
        ),
    ];
    let mut transcript = Vec::new();
    for (step, name, arguments) in steps {
        let alias = session
            .routes
            .iter()
            .find(|(_, route)| route.server == "blender" && route.tool.name == name)
            .unwrap()
            .0
            .clone();
        let started = std::time::Instant::now();
        let result = session
            .execute(&ToolCall {
                id: step.into(),
                name: alias.clone(),
                arguments: arguments.to_string(),
            })
            .await
            .unwrap();
        println!(
            "{step}: {:?} in {:.2}s, {} image(s)",
            result.status,
            started.elapsed().as_secs_f64(),
            result.images.len()
        );
        transcript.push(json!({"step":step,"tool":name,"alias":alias,"arguments":arguments,"status":format!("{:?}",result.status),"elapsedSecs":started.elapsed().as_secs_f64(),"result":result.text,"images":result.images.len()}));
        std::fs::write(
            output_dir.join("mcp-transcript.json"),
            serde_json::to_vec_pretty(&transcript).unwrap(),
        )
        .unwrap();
        assert_eq!(result.status, ActionStatus::Complete, "{}", result.text);
        if name == "execute_blender_code" {
            assert!(
                result.text.contains("Code executed successfully"),
                "{}",
                result.text
            );
        }
        if step == "create-scene" {
            assert!(result.text.contains("HFX_DEMO_CREATED"), "{}", result.text);
        }
        if step == "capture-viewport" {
            assert_eq!(result.images.len(), 1, "{}", result.text);
            attachments::save_png_atomic(&result.images[0], &output_dir.join("viewport.png"))
                .unwrap();
        }
        if step == "render" {
            assert!(result.text.contains("HFX_DEMO_RENDERED"), "{}", result.text);
        }
        if step == "inspect-after" {
            // The upstream scene summary deliberately includes only the first ten objects.
            let envelope: Value = serde_json::from_str(&result.text).unwrap();
            let scene: Value =
                serde_json::from_str(envelope["content"][0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(scene["object_count"], 44);
            assert!(result.text.contains("Robot body"), "{}", result.text);
        }
        if step == "inspect-head" {
            assert!(result.text.contains("Robot head"), "{}", result.text);
            assert!(
                !result.text.contains("Error getting object info"),
                "{}",
                result.text
            );
        }
    }
    assert!(output_dir.join("demo.blend").metadata().unwrap().len() > 1024);
    let render = image::open(output_dir.join("render.png")).unwrap();
    assert_eq!((render.width(), render.height()), (1280, 1000));
    println!("Demo saved to {}", output_dir.display());
}
