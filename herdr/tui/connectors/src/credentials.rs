//! A server's OAuth login, kept in the macOS keychain, else in a file only its owner reads.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use rmcp::transport::auth::{AuthError, CredentialStore, StoredCredentials};

const SERVICE: &str = "cockpit-mcp";

/// The login of one server, by its URL.
pub struct Store {
    pub server: String,
}

impl Store {
    pub fn new(server: &str) -> Self {
        Store { server: server.to_string() }
    }

    fn file(&self) -> PathBuf {
        let name: String = self.server.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join(".config/cockpit/credentials").join(format!("{name}.json"))
    }

    fn read(&self) -> Option<String> {
        if cfg!(target_os = "macos") {
            let out = Command::new("security").args(["find-generic-password", "-s", SERVICE, "-a", &self.server, "-w"]).output().ok()?;
            return out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
        std::fs::read_to_string(self.file()).ok()
    }

    fn write(&self, text: &str) -> Result<(), String> {
        if cfg!(target_os = "macos") {
            // Through security's own prompt, as hex, so the secret is never in a process's arguments.
            let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
            let account = self.server.replace('"', "");
            let mut child = Command::new("security").arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().map_err(|e| e.to_string())?;
            let line = format!("add-generic-password -U -s {SERVICE} -a \"{account}\" -X {hex}\n");
            child.stdin.take().ok_or("no stdin")?.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            let status = child.wait().map_err(|e| e.to_string())?;
            return if status.success() { Ok(()) } else { Err("the keychain refused it".into()) };
        }
        use std::os::unix::fs::OpenOptionsExt;
        let path = self.file();
        std::fs::create_dir_all(path.parent().unwrap_or(&path)).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn forget(&self) {
        if cfg!(target_os = "macos") {
            let _ = Command::new("security").args(["delete-generic-password", "-s", SERVICE, "-a", &self.server]).output();
        } else {
            let _ = std::fs::remove_file(self.file());
        }
    }

    pub fn has_login(&self) -> bool {
        self.read().is_some()
    }
}

#[async_trait::async_trait]
impl CredentialStore for Store {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match self.read() {
            None => Ok(None),
            Some(text) => serde_json::from_str(&text).map(Some).map_err(|e| AuthError::InternalError(format!("stored login unreadable: {e}"))),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials).map_err(|e| AuthError::InternalError(e.to_string()))?;
        self.write(&text).map_err(AuthError::InternalError)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.forget();
        Ok(())
    }
}
