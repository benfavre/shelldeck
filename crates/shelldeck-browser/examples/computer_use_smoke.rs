//! SDTEST-2555 / SDUC-557 — opt-in real BrowserClient + MCP stdio + native CEF workflow.
//! `cargo run -p shelldeck-browser --example computer_use_smoke -- HELPER_EXECUTABLE`
//! Uses only a loopback fixture and disposable profile. No provider is launched.
use base64::Engine;
use serde_json::{json, Value};
use shelldeck_browser::{
    mcp::{self, BrowserMcpConfig, BrowserPermission},
    BrowserClient, BrowserId, BrowserLaunch, Command, Event, KeyEvent, KeyEventKind,
};
use std::{
    error::Error,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, ChildStdin, Command as Process, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const TIMEOUT: Duration = Duration::from_secs(15);
const HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>Native computer use fixture</title>
<style>body{font:20px sans-serif;height:3000px;background:#1f1f2d;color:white}input,button{font:20px sans-serif;position:absolute;box-sizing:border-box;height:40px}#field{left:40px;top:60px;width:320px}#counter{left:40px;top:130px;width:180px}#popup{left:260px;top:130px;width:200px}</style>
<h1>Native computer use fixture</h1><form onsubmit="window.submitted++;return false"><input id="field" aria-label="Native text field"></form><button id="counter" onclick="window.clicked++;this.textContent='Clicked '+window.clicked">Click Native</button><button id="popup" onclick="window.open('/popup','shelldeck-smoke-popup')">Open related popup</button>
<script>window.clicked=0;window.submitted=0;window.lastKey=null;window.popupMessage=null;document.addEventListener('keydown',event=>window.lastKey=event.key);window.addEventListener('message',event=>{if(event.origin===location.origin)window.popupMessage=event.data});</script>"#;
const POPUP: &str = r#"<!doctype html><meta charset="utf-8"><title>Related computer use popup</title><style>input{position:absolute;left:40px;top:60px;width:320px;height:40px;font:20px sans-serif}</style><h1>Related popup</h1><input id="field" aria-label="Popup text field"><script>window.opener.postMessage('native-opener-ready',location.origin)</script>"#;

struct Fixture {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = format!("http://{}/", listener.local_addr()?);
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        thread::spawn(move || {
                            let _ = serve(stream);
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn serve(mut stream: TcpStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = [0; 8192];
    let length = stream.read(&mut bytes)?;
    let first_line = String::from_utf8_lossy(&bytes[..length]);
    let body = if first_line.starts_with("GET /popup ") {
        POPUP
    } else {
        HTML
    };
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
}

struct Proxy {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<io::Result<Value>>,
    next_id: u64,
}
impl Proxy {
    fn start(config: BrowserMcpConfig) -> io::Result<Self> {
        let mut child = Process::new(&config.command)
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("MCP stdin is missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("MCP stdout is missing"))?;
        let (sender, responses) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let result =
                    line.and_then(|line| serde_json::from_str(&line).map_err(io::Error::other));
                if sender.send(result).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            responses,
            next_id: 1,
        })
    }
    fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        serde_json::to_writer(
            &mut self.stdin,
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        let response = self.responses.recv_timeout(TIMEOUT).map_err(|error| {
            io::Error::other(format!("MCP proxy ended or timed out: {error}"))
        })??;
        if response["id"] != id {
            return Err(io::Error::other("MCP response correlation failed"));
        }
        Ok(response)
    }
    fn initialize(&mut self) -> Result<()> {
        let reply = self.request("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"ShellDeck local smoke","version":"1"}}))?;
        assert_eq!(
            reply["result"]["serverInfo"]["name"],
            "shelldeck-native-browser"
        );
        Ok(())
    }
    fn tool(&mut self, name: &str, args: Value) -> Result<Value> {
        let response = self.request("tools/call", json!({"name":name,"arguments":args}))?;
        if response.get("error").is_some() {
            return Err(format!("MCP JSON-RPC error from {name}").into());
        }
        Ok(response["result"].clone())
    }
    fn allowed_tool(&mut self, name: &str, args: Value) -> Result<Value> {
        let result = self.tool(name, args)?;
        if result["isError"] == true {
            return Err(
                format!("MCP tool {name} refused: {}", result["content"][0]["text"]).into(),
            );
        }
        Ok(result)
    }
    fn text_tool(&mut self, name: &str, args: Value) -> Result<Value> {
        let result = self.allowed_tool(name, args)?;
        Ok(serde_json::from_str(
            result["content"][0]["text"]
                .as_str()
                .ok_or("MCP text content missing")?,
        )?)
    }
    fn screenshot(&mut self, id: BrowserId) -> Result<Vec<u8>> {
        let result =
            self.allowed_tool("browser_screenshot", json!({"browser_id":id.to_string()}))?;
        let image = result["content"]
            .as_array()
            .ok_or("MCP screenshot content missing")?
            .iter()
            .find(|item| item["type"] == "image")
            .ok_or("MCP screenshot did not contain an image")?;
        assert_eq!(image["mimeType"], "image/png");
        let bytes = base64::engine::general_purpose::STANDARD.decode(
            image["data"]
                .as_str()
                .ok_or("MCP screenshot data missing")?,
        )?;
        let decoded = image::load_from_memory(&bytes)?;
        assert_eq!((decoded.width(), decoded.height()), (800, 600));
        Ok(bytes)
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn until(mut predicate: impl FnMut() -> bool, label: &str) -> Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if predicate() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(15));
    }
    Err(format!("Timed out waiting for {label}").into())
}
fn evaluate(client: &BrowserClient, id: BrowserId, expression: &str) -> Result<Value> {
    let result = client.devtools(
        id,
        "Runtime.evaluate",
        json!({"expression":expression,"returnByValue":true,"userGesture":true}),
        TIMEOUT,
    )?;
    if result.get("exceptionDetails").is_some() {
        return Err("Trusted fixture assertion threw a JavaScript exception".into());
    }
    Ok(result["result"]["value"].clone())
}
fn dom(client: &BrowserClient, id: BrowserId, expression: &str, expected: Value) -> Result<()> {
    until(
        || evaluate(client, id, expression).is_ok_and(|value| value == expected),
        expression,
    )
}
fn click(proxy: &mut Proxy, id: BrowserId, x: i32, y: i32) -> Result<()> {
    proxy.screenshot(id)?;
    proxy.allowed_tool(
        "browser_click",
        json!({"browser_id":id.to_string(),"x":x,"y":y}),
    )?;
    Ok(())
}
fn tabs(proxy: &mut Proxy) -> Result<Vec<String>> {
    let result = proxy.text_tool("browser_tabs", json!({}))?;
    Ok(result["tabs"]
        .as_array()
        .ok_or("Tab list missing")?
        .iter()
        .map(|tab| {
            tab["browser_id"]
                .as_str()
                .expect("MCP tab IDs must remain exact strings")
                .to_owned()
        })
        .collect())
}

fn main() -> Result<()> {
    let helper = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Pass a staged browser helper executable")?,
    )
    .canonicalize()?;
    let artifact_dir = std::env::temp_dir().join(format!(
        "shelldeck-computer-use-smoke-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&artifact_dir)?;
    let fixture = Fixture::start()?;
    let client = BrowserClient::spawn(BrowserLaunch {
        executable: helper,
        profile_dir: artifact_dir.join("profile"),
    })?;
    let mut ready = false;
    until(
        || {
            for event in client.drain_events() {
                if matches!(event, Event::Ready { .. }) {
                    ready = true;
                }
            }
            ready
        },
        "CEF readiness",
    )?;
    let id = client.create_tab(&fixture.address, 800, 600, 1.0)?;
    until(
        || {
            client.latest_frame(id).is_some()
                && client.state(id).is_some_and(|state| !state.loading)
        },
        "initial native paint",
    )?;
    let foreign = client.create_tab("about:blank", 800, 600, 1.0)?;
    let observe = mcp::grant(client.clone(), [id], BrowserPermission::ObserveOnly)?;
    let mut observer = Proxy::start(observe.configuration())?;
    observer.initialize()?;
    let tools = observer.request("tools/list", json!({}))?;
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 3);
    assert_eq!(tabs(&mut observer)?, [id.to_string()]);
    fs::write(
        artifact_dir.join("mcp-observe.png"),
        observer.screenshot(id)?,
    )?;
    let accessibility =
        observer.text_tool("browser_snapshot", json!({"browser_id":id.to_string()}))?;
    assert_eq!(accessibility["untrusted_page_content"], true);
    assert!(accessibility["accessibility"]["nodes"]
        .as_array()
        .is_some_and(|nodes| nodes.iter().any(|node| node["name"]["value"]
            .as_str()
            .is_some_and(|name| name.contains("Native computer use fixture")))));
    assert_eq!(
        observer.tool(
            "browser_click",
            json!({"browser_id":id.to_string(),"x":100,"y":150})
        )?["isError"],
        true
    );
    assert_eq!(
        observer.tool(
            "browser_snapshot",
            json!({"browser_id":foreign.to_string()})
        )?["isError"],
        true
    );
    dom(&client, id, "window.clicked", json!(0))?;
    observe.revoke();
    assert!(observer.request("ping", json!({})).is_err());
    drop(observer);

    let control = mcp::grant(client.clone(), [id], BrowserPermission::Control)?;
    control.include_new_tabs();
    let mut controller = Proxy::start(control.configuration())?;
    controller.initialize()?;
    let tools = controller.request("tools/list", json!({}))?;
    assert!(tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "browser_click"));
    assert!(!tabs(&mut controller)?.contains(&foreign.to_string()));
    click(&mut controller, id, 120, 80)?;
    dom(&client, id, "document.activeElement.id", json!("field"))?;
    controller.allowed_tool(
        "browser_type",
        json!({"browser_id":id.to_string(),"text":"Native MCP é 🦀"}),
    )?;
    dom(
        &client,
        id,
        "document.querySelector('#field').value",
        json!("Native MCP é 🦀"),
    )?;
    controller.allowed_tool(
        "browser_key",
        json!({"browser_id":id.to_string(),"key":"ArrowLeft"}),
    )?;
    dom(&client, id, "window.lastKey", json!("ArrowLeft"))?;
    controller.allowed_tool(
        "browser_key",
        json!({"browser_id":id.to_string(),"key":"Enter"}),
    )?;
    dom(&client, id, "window.submitted", json!(1))?;
    // Match the human GPUI path: raw printable key + one IME commit + release.
    // In particular, macOS normalization must never turn RawKeyDown into Char.
    evaluate(
        &client,
        id,
        "document.querySelector('#field').value='';document.querySelector('#field').focus();true",
    )?;
    for kind in [KeyEventKind::RawKeyDown, KeyEventKind::KeyUp] {
        client.send(Command::Key {
            browser_id: id,
            event: KeyEvent {
                kind,
                windows_key_code: 65,
                native_key_code: 0,
                modifiers: 0,
                character: None,
                unmodified_character: None,
                is_system_key: false,
            },
        })?;
        if matches!(kind, KeyEventKind::RawKeyDown) {
            client.send(Command::Text {
                browser_id: id,
                text: "a".into(),
            })?;
        }
    }
    let settled = client.devtools(id,"Runtime.evaluate",json!({"expression":"new Promise(resolve=>setTimeout(()=>resolve(document.querySelector('#field').value),150))","returnByValue":true,"awaitPromise":true}),TIMEOUT)?;
    assert_eq!(
        settled["result"]["value"], "a",
        "Human raw-key/IME sequence must insert exactly once"
    );

    click(&mut controller, id, 120, 150)?;
    dom(&client, id, "window.clicked", json!(1))?;
    controller.allowed_tool(
        "browser_scroll",
        json!({"browser_id":id.to_string(),"x":700,"y":500,"delta_y":-600}),
    )?;
    dom(&client, id, "window.scrollY>100", json!(true))?;
    controller.allowed_tool(
        "browser_scroll",
        json!({"browser_id":id.to_string(),"x":700,"y":500,"delta_y":600}),
    )?;
    dom(&client, id, "window.scrollY", json!(0))?;
    click(&mut controller, id, 350, 150)?;
    until(
        || client.owned_ids().iter().any(|id| *id >= (1u64 << 63)),
        "correlated popup adoption",
    )?;
    let child = client
        .owned_ids()
        .into_iter()
        .find(|id| *id >= (1u64 << 63))
        .ok_or("Popup ID missing")?;
    dom(
        &client,
        id,
        "window.popupMessage",
        json!("native-opener-ready"),
    )?;
    until(
        || client.latest_frame(child).is_some(),
        "popup native paint",
    )?;
    assert!(tabs(&mut controller)?.contains(&child.to_string()));
    click(&mut controller, child, 120, 80)?;
    dom(&client, child, "document.activeElement.id", json!("field"))?;
    controller.allowed_tool(
        "browser_type",
        json!({"browser_id":child.to_string(),"text":"Exact popup ID"}),
    )?;
    dom(
        &client,
        child,
        "document.querySelector('#field').value",
        json!("Exact popup ID"),
    )?;
    assert_eq!(
        controller.tool(
            "browser_type",
            json!({"browser_id":child,"text":"must refuse imprecise numeric ID"})
        )?["isError"],
        true
    );
    assert_eq!(
        controller.tool(
            "browser_type",
            json!({"browser_id":foreign.to_string(),"text":"must stay private"})
        )?["isError"],
        true
    );
    fs::write(
        artifact_dir.join("mcp-control.png"),
        controller.screenshot(child)?,
    )?;
    client.revoke_control_grants();
    assert_eq!(
        controller.tool(
            "browser_type",
            json!({"browser_id":id.to_string(),"text":"must refuse revoked control"})
        )?["isError"],
        true
    );
    controller.screenshot(id)?;
    control.revoke();
    assert!(controller.request("ping", json!({})).is_err());
    drop(controller);
    client.send(Command::Shutdown)?;
    until(|| client.failure().is_some(), "helper shutdown")?;
    println!("SDTEST-2555 PASS: native CEF + BrowserClient + actual MCP stdio, observe/control, PNG/AX, input, popup string IDs, foreign-tab isolation, revocation. Artifacts: {}",artifact_dir.display());
    Ok(())
}
