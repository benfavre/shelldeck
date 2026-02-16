use crate::client::SshClient;
use crate::session::SshSession;
use parking_lot::RwLock;
use shelldeck_core::models::Connection;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Manages a pool of active SSH sessions keyed by connection ID.
pub struct ConnectionPool {
    client: SshClient,
    sessions: Arc<RwLock<HashMap<Uuid, SshSession>>>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self {
            client: SshClient::new(),
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Connect to a remote host and add the session to the pool.
    /// Returns the connection ID on success.
    pub async fn connect(&self, connection: &Connection) -> crate::Result<Uuid> {
        let session = self.client.connect(connection).await?;
        let id = connection.id;
        self.sessions.write().insert(id, session);
        tracing::info!("Connection {} added to pool", id);
        Ok(id)
    }

    /// Execute a callback with a reference to a session.
    /// This avoids lifetime issues with returning references from RwLock guards.
    pub fn with_session<F, R>(&self, id: &Uuid, f: F) -> Option<R>
    where
        F: FnOnce(&SshSession) -> R,
    {
        let sessions = self.sessions.read();
        sessions.get(id).map(f)
    }

    /// Execute a callback with a mutable reference to a session.
    pub fn with_session_mut<F, R>(&self, id: &Uuid, f: F) -> Option<R>
    where
        F: FnOnce(&mut SshSession) -> R,
    {
        let mut sessions = self.sessions.write();
        sessions.get_mut(id).map(f)
    }

    /// Remove a session from the pool and disconnect it.
    pub async fn disconnect(&self, id: &Uuid) -> crate::Result<()> {
        let session = self.sessions.write().remove(id);
        if let Some(session) = session {
            session.disconnect().await?;
            tracing::info!("Connection {} removed from pool", id);
        }
        Ok(())
    }

    /// Disconnect all sessions and clear the pool.
    pub async fn disconnect_all(&self) {
        let sessions: Vec<(Uuid, SshSession)> = self.sessions.write().drain().collect();
        for (id, session) in sessions {
            if let Err(e) = session.disconnect().await {
                tracing::warn!("Error disconnecting session {}: {}", id, e);
            }
        }
        tracing::info!("All connections disconnected");
    }

    /// Check if a connection is in the pool.
    pub fn is_connected(&self, id: &Uuid) -> bool {
        self.sessions.read().contains_key(id)
    }

    /// Get the number of active sessions.
    pub fn active_count(&self) -> usize {
        self.sessions.read().len()
    }

    /// Get a list of all connected session IDs.
    pub fn connected_ids(&self) -> Vec<Uuid> {
        self.sessions.read().keys().cloned().collect()
    }

    /// Take a session out of the pool, transferring ownership to the caller.
    /// Useful when you need to move the session into an async context.
    pub fn take_session(&self, id: &Uuid) -> Option<SshSession> {
        self.sessions.write().remove(id)
    }

    /// Insert a session back into the pool.
    pub fn return_session(&self, id: Uuid, session: SshSession) {
        self.sessions.write().insert(id, session);
    }
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}
