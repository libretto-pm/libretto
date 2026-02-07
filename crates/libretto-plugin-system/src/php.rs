//! PHP plugin support via IPC.
//!
//! This module provides Composer-compatible PHP plugin support by forking PHP
//! processes and communicating via IPC (Unix sockets on Unix, named pipes on Windows).

use crate::api::{EventContext, EventResult, PluginCapability};
use crate::error::{PluginError, Result};
use crate::hooks::Hook;
use bytes::{BufMut, BytesMut};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// Default PHP binary name.
const DEFAULT_PHP_BINARY: &str = "php";

/// IPC message types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MessageType {
    /// Request to invoke a method.
    Invoke,
    /// Response from plugin.
    Response,
    /// Error response.
    Error,
    /// Shutdown request.
    Shutdown,
    /// Ping/keepalive.
    Ping,
    /// Pong response.
    Pong,
    /// Get capabilities.
    GetCapabilities,
}

/// IPC message structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IpcMessage {
    /// Message type.
    #[serde(rename = "type")]
    msg_type: MessageType,
    /// Message ID for request/response correlation.
    id: u64,
    /// Hook being invoked (for invoke messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    hook: Option<String>,
    /// Context data (for invoke messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<serde_json::Value>,
    /// Result data (for response messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    /// Error message (for error responses).
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl IpcMessage {
    fn invoke(id: u64, hook: Hook, context: &EventContext) -> Result<Self> {
        let context_json = serde_json::to_value(context).map_err(|e| PluginError::Ipc {
            plugin: String::new(),
            message: e.to_string(),
        })?;

        Ok(Self {
            msg_type: MessageType::Invoke,
            id,
            hook: Some(hook.as_str().to_string()),
            context: Some(context_json),
            result: None,
            error: None,
        })
    }

    const fn get_capabilities(id: u64) -> Self {
        Self {
            msg_type: MessageType::GetCapabilities,
            id,
            hook: None,
            context: None,
            result: None,
            error: None,
        }
    }

    const fn shutdown(id: u64) -> Self {
        Self {
            msg_type: MessageType::Shutdown,
            id,
            hook: None,
            context: None,
            result: None,
            error: None,
        }
    }

    const fn ping(id: u64) -> Self {
        Self {
            msg_type: MessageType::Ping,
            id,
            hook: None,
            context: None,
            result: None,
            error: None,
        }
    }
}

/// PHP plugin bridge for managing PHP plugin processes.
#[derive(Debug)]
#[allow(dead_code)]
pub struct PhpPluginBridge {
    /// PHP binary path.
    php_binary: String,
    /// Default timeout for plugin operations.
    timeout: Duration,
    /// Bootstrap script path (generated).
    bootstrap_script: Option<PathBuf>,
}

impl PhpPluginBridge {
    /// Create a new PHP plugin bridge.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            php_binary: std::env::var("PHP_BINARY").unwrap_or_else(|_| DEFAULT_PHP_BINARY.into()),
            timeout,
            bootstrap_script: None,
        }
    }

    /// Set the PHP binary path.
    pub fn set_php_binary(&mut self, path: impl Into<String>) {
        self.php_binary = path.into();
    }

    /// Create a PHP plugin instance.
    ///
    /// # Errors
    /// Returns error if plugin creation fails.
    pub async fn create_plugin(&self, class: &str, path: &Path) -> Result<PhpPlugin> {
        info!(class = %class, path = %path.display(), "creating PHP plugin");

        // Verify PHP is available
        self.verify_php().await?;

        // Generate the bootstrap script
        let bootstrap = self.generate_bootstrap_script(class, path)?;

        Ok(PhpPlugin {
            class: class.to_string(),
            path: path.to_path_buf(),
            bootstrap_script: bootstrap,
            php_binary: self.php_binary.clone(),
            timeout: self.timeout,
            process: RwLock::new(None),
            message_id: std::sync::atomic::AtomicU64::new(1),
            sender: RwLock::new(None),
            receiver: RwLock::new(None),
        })
    }

    /// Verify PHP is available and working.
    async fn verify_php(&self) -> Result<()> {
        let output = Command::new(&self.php_binary)
            .args(["-v"])
            .output()
            .await
            .map_err(|e| PluginError::PhpRuntime {
                plugin: String::new(),
                message: format!("failed to execute PHP: {e}"),
            })?;

        if !output.status.success() {
            return Err(PluginError::PhpRuntime {
                plugin: String::new(),
                message: "PHP version check failed".into(),
            });
        }

        let version = String::from_utf8_lossy(&output.stdout);
        debug!(version = %version.lines().next().unwrap_or("unknown"), "PHP version detected");

        Ok(())
    }

    /// Generate a PHP bootstrap script for the plugin.
    fn generate_bootstrap_script(&self, class: &str, plugin_path: &Path) -> Result<PathBuf> {
        let script = format!(
            r#"<?php
/**
 * Libretto PHP Plugin Bootstrap
 * Auto-generated - do not edit
 */

// Error handling
set_error_handler(function($severity, $message, $file, $line) {{
    throw new ErrorException($message, 0, $severity, $file, $line);
}});

// Load Composer autoloader
$autoloadPaths = [
    __DIR__ . '/vendor/autoload.php',
    __DIR__ . '/../vendor/autoload.php',
    __DIR__ . '/../../vendor/autoload.php',
    '{plugin_path}/vendor/autoload.php',
];

$loaded = false;
foreach ($autoloadPaths as $path) {{
    if (file_exists($path)) {{
        require_once $path;
        $loaded = true;
        break;
    }}
}}

if (!$loaded) {{
    fwrite(STDERR, "Could not find autoloader\n");
    exit(1);
}}

// Instantiate the plugin
$pluginClass = '{class}';
if (!class_exists($pluginClass)) {{
    fwrite(STDERR, "Plugin class not found: $pluginClass\n");
    exit(1);
}}

$plugin = new $pluginClass();

// IPC message handling
function sendMessage($message) {{
    $json = json_encode($message);
    $length = strlen($json);
    fwrite(STDOUT, pack('N', $length) . $json);
    fflush(STDOUT);
}}

function readMessage() {{
    $header = fread(STDIN, 4);
    if ($header === false || strlen($header) < 4) {{
        return null;
    }}
    $length = unpack('N', $header)[1];
    $data = fread(STDIN, $length);
    return json_decode($data, true);
}}

// Main loop
while (true) {{
    $message = readMessage();
    if ($message === null) {{
        break;
    }}

    $response = [
        'type' => 'response',
        'id' => $message['id'],
    ];

    try {{
        switch ($message['type']) {{
            case 'invoke':
                $hook = $message['hook'] ?? null;
                $context = $message['context'] ?? [];

                // Map hook names to Composer event methods
                $methodMap = [
                    'pre-install-cmd' => 'onPreInstall',
                    'post-install-cmd' => 'onPostInstall',
                    'pre-update-cmd' => 'onPreUpdate',
                    'post-update-cmd' => 'onPostUpdate',
                    'pre-autoload-dump' => 'onPreAutoloadDump',
                    'post-autoload-dump' => 'onPostAutoloadDump',
                    'pre-package-install' => 'onPrePackageInstall',
                    'post-package-install' => 'onPostPackageInstall',
                    'pre-package-update' => 'onPrePackageUpdate',
                    'post-package-update' => 'onPostPackageUpdate',
                    'pre-package-uninstall' => 'onPrePackageUninstall',
                    'post-package-uninstall' => 'onPostPackageUninstall',
                    'pre-dependencies-solving' => 'onPreDependenciesSolving',
                    'post-dependencies-solving' => 'onPostDependenciesSolving',
                    'pre-file-download' => 'onPreFileDownload',
                    'command' => 'onCommand',
                    'init' => 'onInit',
                ];

                $method = $methodMap[$hook] ?? null;

                if ($method && method_exists($plugin, $method)) {{
                    $result = $plugin->$method($context);
                    $response['result'] = [
                        'continue_processing' => $result['continue_processing'] ?? true,
                        'messages' => $result['messages'] ?? [],
                        'modified_data' => $result['modified_data'] ?? [],
                    ];
                }} else {{
                    // No handler, return success
                    $response['result'] = [
                        'continue_processing' => true,
                        'messages' => [],
                    ];
                }}
                break;

            case 'get_capabilities':
                $capabilities = [];
                if (method_exists($plugin, 'getCapabilities')) {{
                    $capabilities = $plugin->getCapabilities();
                }} else {{
                    // Default capabilities based on implemented methods
                    if (method_exists($plugin, 'onPreInstall') || method_exists($plugin, 'onPostInstall')) {{
                        $capabilities[] = 'install';
                    }}
                    if (method_exists($plugin, 'onPreUpdate') || method_exists($plugin, 'onPostUpdate')) {{
                        $capabilities[] = 'install';
                    }}
                    if (method_exists($plugin, 'onPreAutoloadDump') || method_exists($plugin, 'onPostAutoloadDump')) {{
                        $capabilities[] = 'autoload';
                    }}
                    if (method_exists($plugin, 'onPreDependenciesSolving') || method_exists($plugin, 'onPostDependenciesSolving')) {{
                        $capabilities[] = 'resolve';
                    }}
                    if (method_exists($plugin, 'onCommand') || method_exists($plugin, 'getCommands')) {{
                        $capabilities[] = 'command';
                    }}
                }}
                $response['result'] = array_unique($capabilities);
                break;

            case 'shutdown':
                sendMessage($response);
                exit(0);

            case 'ping':
                $response['type'] = 'pong';
                break;

            default:
                $response['type'] = 'error';
                $response['error'] = 'Unknown message type: ' . ($message['type'] ?? 'null');
        }}
    }} catch (Throwable $e) {{
        $response['type'] = 'error';
        $response['error'] = $e->getMessage();
    }}

    sendMessage($response);
}}
"#,
            class = class,
            plugin_path = plugin_path.display()
        );

        // Write to a temporary file
        let temp_dir = std::env::temp_dir().join("libretto-plugins");
        std::fs::create_dir_all(&temp_dir)?;

        let script_hash = blake3::hash(script.as_bytes());
        let script_path = temp_dir.join(format!("bootstrap_{}.php", &script_hash.to_hex()[..16]));

        std::fs::write(&script_path, script)?;

        Ok(script_path)
    }
}

/// PHP plugin instance.
#[derive(Debug)]
pub struct PhpPlugin {
    /// Plugin class name.
    class: String,
    /// Plugin path.
    path: PathBuf,
    /// Bootstrap script path.
    bootstrap_script: PathBuf,
    /// PHP binary path.
    php_binary: String,
    /// Operation timeout.
    timeout: Duration,
    /// PHP process handle.
    process: RwLock<Option<Child>>,
    /// Message ID counter.
    message_id: std::sync::atomic::AtomicU64,
    /// Message sender channel.
    sender: RwLock<Option<mpsc::Sender<IpcMessage>>>,
    /// Message receiver channel.
    receiver: RwLock<Option<mpsc::Receiver<IpcMessage>>>,
}

impl PhpPlugin {
    /// Start the PHP plugin process.
    ///
    /// # Errors
    /// Returns error if the process fails to start.
    pub async fn start(&self) -> Result<()> {
        info!(
            class = %self.class,
            path = %self.path.display(),
            "starting PHP plugin process"
        );

        let mut cmd = Command::new(&self.php_binary);
        cmd.arg(&self.bootstrap_script)
            .current_dir(&self.path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| PluginError::PhpRuntime {
            plugin: self.class.clone(),
            message: format!("failed to spawn PHP process: {e}"),
        })?;

        // Set up IPC channels
        let stdin = child.stdin.take().ok_or_else(|| PluginError::Ipc {
            plugin: self.class.clone(),
            message: "failed to capture stdin".into(),
        })?;

        let stdout = child.stdout.take().ok_or_else(|| PluginError::Ipc {
            plugin: self.class.clone(),
            message: "failed to capture stdout".into(),
        })?;

        let stderr = child.stderr.take().ok_or_else(|| PluginError::Ipc {
            plugin: self.class.clone(),
            message: "failed to capture stderr".into(),
        })?;

        // Create message channels
        let (tx, mut rx) = mpsc::channel::<IpcMessage>(100);
        let (response_tx, response_rx) = mpsc::channel::<IpcMessage>(100);

        // Spawn writer task
        let plugin_name = self.class.clone();
        let mut stdin = tokio::io::BufWriter::new(stdin);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let json = match serde_json::to_vec(&msg) {
                    Ok(j) => j,
                    Err(e) => {
                        error!(plugin = %plugin_name, error = %e, "failed to serialize message");
                        continue;
                    }
                };

                // Write length-prefixed message
                let mut buf = BytesMut::with_capacity(4 + json.len());
                buf.put_u32(json.len() as u32);
                buf.extend_from_slice(&json);

                if let Err(e) = stdin.write_all(&buf).await {
                    error!(plugin = %plugin_name, error = %e, "failed to write to plugin");
                    break;
                }

                if let Err(e) = stdin.flush().await {
                    error!(plugin = %plugin_name, error = %e, "failed to flush to plugin");
                    break;
                }
            }
        });

        // Spawn reader task
        let plugin_name = self.class.clone();
        let mut stdout = BufReader::new(stdout);
        tokio::spawn(async move {
            let mut len_buf = [0u8; 4];
            loop {
                // Read length prefix
                if let Err(e) = stdout.read_exact(&mut len_buf).await {
                    if e.kind() != std::io::ErrorKind::UnexpectedEof {
                        error!(plugin = %plugin_name, error = %e, "failed to read from plugin");
                    }
                    break;
                }

                let len = u32::from_be_bytes(len_buf) as usize;
                if len > 10 * 1024 * 1024 {
                    error!(plugin = %plugin_name, "message too large: {} bytes", len);
                    break;
                }

                let mut data = vec![0u8; len];
                if let Err(e) = stdout.read_exact(&mut data).await {
                    error!(plugin = %plugin_name, error = %e, "failed to read message body");
                    break;
                }

                let msg: IpcMessage = match serde_json::from_slice(&data) {
                    Ok(m) => m,
                    Err(e) => {
                        error!(plugin = %plugin_name, error = %e, "failed to parse message");
                        continue;
                    }
                };

                if response_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Spawn stderr reader for logging
        let plugin_name = self.class.clone();
        let mut stderr = BufReader::new(stderr);
        tokio::spawn(async move {
            let mut line = String::new();
            while stderr.read_line(&mut line).await.is_ok() {
                if line.is_empty() {
                    break;
                }
                warn!(plugin = %plugin_name, stderr = %line.trim(), "PHP plugin stderr");
                line.clear();
            }
        });

        // Store handles
        *self.process.write() = Some(child);
        *self.sender.write() = Some(tx);
        *self.receiver.write() = Some(response_rx);

        // Send a ping to verify the connection
        self.ping().await?;

        info!(class = %self.class, "PHP plugin started");
        Ok(())
    }

    /// Stop the PHP plugin process.
    ///
    /// # Errors
    /// Returns error if stopping fails.
    pub async fn stop(&self) -> Result<()> {
        // Send shutdown message
        let _ = self
            .send_message(IpcMessage::shutdown(self.next_id()))
            .await;

        // Close channels
        *self.sender.write() = None;
        *self.receiver.write() = None;

        // Kill process if still running
        let maybe_child = { self.process.write().take() };
        if let Some(mut child) = maybe_child {
            let _ = child.kill().await;
        }

        info!(class = %self.class, "PHP plugin stopped");
        Ok(())
    }

    /// Invoke a hook on the PHP plugin.
    ///
    /// # Errors
    /// Returns error if invocation fails.
    pub async fn invoke(&self, hook: Hook, context: &EventContext) -> Result<EventResult> {
        let id = self.next_id();
        let msg = IpcMessage::invoke(id, hook, context)?;

        let response = self.send_and_receive(msg).await?;

        match response.msg_type {
            MessageType::Response => {
                let result = response.result.unwrap_or_default();
                Ok(EventResult {
                    continue_processing: result
                        .get("continue_processing")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true),
                    messages: result
                        .get("messages")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    modified_data: result
                        .get("modified_data")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        })
                        .unwrap_or_default(),
                    error: None,
                    warnings: Vec::new(),
                })
            }
            MessageType::Error => Err(PluginError::PhpRuntime {
                plugin: self.class.clone(),
                message: response.error.unwrap_or_else(|| "unknown error".into()),
            }),
            _ => Err(PluginError::Ipc {
                plugin: self.class.clone(),
                message: format!("unexpected response type: {:?}", response.msg_type),
            }),
        }
    }

    /// Get plugin capabilities.
    ///
    /// # Errors
    /// Returns error if the request fails.
    pub async fn get_capabilities(&self) -> Result<Vec<PluginCapability>> {
        let id = self.next_id();
        let msg = IpcMessage::get_capabilities(id);

        let response = self.send_and_receive(msg).await?;

        match response.msg_type {
            MessageType::Response => {
                let caps = response.result.unwrap_or_default();
                let cap_strs: Vec<String> = serde_json::from_value(caps).unwrap_or_default();

                Ok(cap_strs
                    .iter()
                    .filter_map(|s| match s.as_str() {
                        "install" => Some(PluginCapability::Install),
                        "resolve" => Some(PluginCapability::Resolve),
                        "command" => Some(PluginCapability::Command),
                        "event" => Some(PluginCapability::Event),
                        "repository" => Some(PluginCapability::Repository),
                        "autoload" => Some(PluginCapability::Autoload),
                        "download" => Some(PluginCapability::Download),
                        "source" => Some(PluginCapability::Source),
                        "script" => Some(PluginCapability::Script),
                        _ => None,
                    })
                    .collect())
            }
            MessageType::Error => Err(PluginError::PhpRuntime {
                plugin: self.class.clone(),
                message: response.error.unwrap_or_else(|| "unknown error".into()),
            }),
            _ => Ok(Vec::new()),
        }
    }

    /// Send a ping to verify the connection.
    async fn ping(&self) -> Result<()> {
        let id = self.next_id();
        let msg = IpcMessage::ping(id);

        let response = timeout(Duration::from_secs(5), self.send_and_receive(msg))
            .await
            .map_err(|_| PluginError::timeout(&self.class, 5))??;

        if response.msg_type != MessageType::Pong {
            return Err(PluginError::Ipc {
                plugin: self.class.clone(),
                message: "expected pong response".into(),
            });
        }

        Ok(())
    }

    /// Send a message and wait for a response.
    async fn send_and_receive(&self, msg: IpcMessage) -> Result<IpcMessage> {
        let id = msg.id;

        self.send_message(msg).await?;

        timeout(self.timeout, self.receive_response(id))
            .await
            .map_err(|_| PluginError::timeout(&self.class, self.timeout.as_secs()))?
    }

    /// Send a message to the plugin.
    async fn send_message(&self, msg: IpcMessage) -> Result<()> {
        let sender = self
            .sender
            .read()
            .as_ref()
            .cloned()
            .ok_or_else(|| PluginError::Ipc {
                plugin: self.class.clone(),
                message: "plugin not started".into(),
            })?;

        sender.send(msg).await.map_err(|e| PluginError::Ipc {
            plugin: self.class.clone(),
            message: format!("failed to send message: {e}"),
        })
    }

    /// Receive a response with the given ID.
    async fn receive_response(&self, id: u64) -> Result<IpcMessage> {
        let mut receiver = self
            .receiver
            .write()
            .take()
            .ok_or_else(|| PluginError::Ipc {
                plugin: self.class.clone(),
                message: "plugin not started".into(),
            })?;

        let mut result = Err(PluginError::Ipc {
            plugin: self.class.clone(),
            message: "channel closed".into(),
        });

        while let Some(msg) = receiver.recv().await {
            if msg.id == id {
                result = Ok(msg);
                break;
            }
            // Discard messages with different IDs (shouldn't happen in normal operation)
            warn!(
                plugin = %self.class,
                expected = id,
                got = msg.id,
                "received out-of-order message"
            );
        }

        *self.receiver.write() = Some(receiver);
        result
    }

    /// Get the next message ID.
    fn next_id(&self) -> u64 {
        self.message_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for PhpPlugin {
    fn drop(&mut self) {
        // Try to cleanly stop the process
        if let Some(child) = self.process.get_mut().take() {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &child.id().unwrap_or(0).to_string()])
                .output();
        }
    }
}

/// Serialization helpers for `EventContext`.
mod context_serde {
    use super::EventContext;
    use serde::{Serialize, Serializer};

    impl Serialize for EventContext {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            use serde::ser::SerializeMap;

            let mut map = serializer.serialize_map(None)?;

            // Serialize packages as strings
            let packages: Vec<String> = self
                .packages
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            map.serialize_entry("packages", &packages)?;

            if let Some(ref op) = self.operation {
                map.serialize_entry("operation", op)?;
            }

            if let Some(ref root) = self.project_root {
                map.serialize_entry("project_root", &root.display().to_string())?;
            }

            if let Some(ref vendor) = self.vendor_dir {
                map.serialize_entry("vendor_dir", &vendor.display().to_string())?;
            }

            map.serialize_entry("data", &self.data)?;
            map.serialize_entry("args", &self.args)?;
            map.serialize_entry("dev_mode", &self.dev_mode)?;
            map.serialize_entry("verbose", &self.verbose)?;
            map.serialize_entry("urls", &self.urls)?;
            map.serialize_entry("versions", &self.versions)?;

            map.end()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_message_serialization() {
        let msg = IpcMessage {
            msg_type: MessageType::Invoke,
            id: 1,
            hook: Some("pre-install-cmd".into()),
            context: Some(serde_json::json!({"test": true})),
            result: None,
            error: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("invoke"));
        assert!(json.contains("pre-install-cmd"));

        let parsed: IpcMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, 1);
        assert_eq!(parsed.hook, Some("pre-install-cmd".into()));
    }

    #[test]
    fn message_constructors() {
        let ping = IpcMessage::ping(42);
        assert_eq!(ping.msg_type, MessageType::Ping);
        assert_eq!(ping.id, 42);

        let shutdown = IpcMessage::shutdown(99);
        assert_eq!(shutdown.msg_type, MessageType::Shutdown);

        let caps = IpcMessage::get_capabilities(1);
        assert_eq!(caps.msg_type, MessageType::GetCapabilities);
    }

    #[test]
    fn bridge_creation() {
        let bridge = PhpPluginBridge::new(Duration::from_secs(30));
        assert_eq!(bridge.timeout, Duration::from_secs(30));
    }
}
