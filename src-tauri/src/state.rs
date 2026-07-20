use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::db::connect::Connection;
use crate::error::{AppError, Result};
use crate::repl::session::ReplSession;
use crate::store::Paths;

/// Everything the commands need. Held by Tauri as managed state.
pub struct AppState {
    pub paths: Paths,
    connection: RwLock<Option<Arc<Connection>>>,
    repl_sessions: Mutex<HashMap<String, Arc<Mutex<ReplSession>>>>,
    /// Cancel handles for terminal sessions, kept **outside** the session mutex.
    ///
    /// `submit` holds the session lock for as long as the statement runs, so a
    /// cancel that had to take that lock could only ever fire once the query it
    /// wanted to kill had already finished.
    repl_cancels: Mutex<HashMap<String, tokio_postgres::CancelToken>>,
    /// Cancellation handle for the in-flight grid query, if any.
    ///
    /// A std mutex, not a tokio one: it is written from the synchronous
    /// callback `fetch_page` invokes *before* it awaits the query, which is the
    /// only moment at which registering the handle is useful.
    grid_cancel: std::sync::Mutex<Option<tokio_postgres::CancelToken>>,
    /// Dedicated client for the query editor.
    ///
    /// Not the browse pool: the editor must be able to write, so it needs an
    /// unrestricted connection like the terminal's. Not the terminal's client
    /// either — sharing would let an editor run clobber `SET`s and `\timing`
    /// state the user set up in the psql pane, and would serialise the two
    /// against each other.
    ///
    /// One client shared by all tabs, so runs from different tabs queue rather
    /// than interleave on the same session.
    editor_client: Mutex<Option<Arc<tokio_postgres::Client>>>,
    /// Cancel handle for the editor, kept outside `editor_client`'s mutex for
    /// the same reason as `repl_cancels`.
    editor_cancel: std::sync::Mutex<Option<tokio_postgres::CancelToken>>,
}

impl AppState {
    /// The disconnected starting state, built once at startup. Everything but
    /// `paths` is filled in lazily as the user connects.
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            connection: RwLock::new(None),
            repl_sessions: Mutex::new(HashMap::new()),
            repl_cancels: Mutex::new(HashMap::new()),
            grid_cancel: std::sync::Mutex::new(None),
            editor_client: Mutex::new(None),
            editor_cancel: std::sync::Mutex::new(None),
        }
    }

    /// Install a connection, or pass `None` to disconnect.
    ///
    /// Any change tears down everything derived from the previous connection —
    /// *including* replacing one connection with another, not only disconnecting.
    /// Terminal sessions and the editor client each hold their own client to the
    /// old server, so leaving them in place after a switch meant the psql pane
    /// and the editor kept running statements against the database the user
    /// thought they had left, under a prompt naming it correctly and a titlebar
    /// naming the new one.
    ///
    /// The cleanup lives here rather than at the call sites so no future path can
    /// swap the connection and forget it.
    ///
    /// Takes four locks in turn and never holds two at once, which is what keeps
    /// it from deadlocking against `add_repl`/`remove_repl` running concurrently.
    pub async fn set_connection(&self, conn: Option<Arc<Connection>>) {
        *self.connection.write().await = conn;
        self.repl_sessions.lock().await.clear();
        self.repl_cancels.lock().await.clear();
        *self.editor_client.lock().await = None;
        self.set_editor_cancel(None);
    }

    // ---- query editor ----

    /// The editor's client, connecting lazily on first use and reconnecting if
    /// the previous one died.
    pub async fn editor(&self) -> Result<Arc<tokio_postgres::Client>> {
        let conn = self.connection().await?;
        let mut guard = self.editor_client.lock().await;

        if let Some(client) = guard.as_ref() {
            if !client.is_closed() {
                return Ok(Arc::clone(client));
            }
        }

        let client = crate::db::connect::connect_client(
            &conn.profile,
            conn.password.as_deref(),
            "pgscope-editor",
            None,
        )
        .await?;
        let client = Arc::new(client);
        self.set_editor_cancel(Some(client.cancel_token()));
        *guard = Some(Arc::clone(&client));
        Ok(client)
    }

    /// Publish the cancel handle for the editor's client, or clear it with `None`.
    ///
    /// Synchronous by design: it is safe to call while `editor_client`'s mutex is
    /// held (`editor` does exactly that), because this lock is never held across
    /// an await and so cannot be the far side of a deadlock.
    ///
    /// A poisoned lock is swallowed rather than unwrapped — losing the ability to
    /// cancel is survivable, panicking a Tauri command is not.
    pub fn set_editor_cancel(&self, token: Option<tokio_postgres::CancelToken>) {
        if let Ok(mut guard) = self.editor_cancel.lock() {
            *guard = token;
        }
    }

    /// The handle needed to cancel an editor run in flight.
    ///
    /// Clones rather than takes, so the handle survives for the next run — and,
    /// crucially, is reachable *while* `editor_client`'s mutex is held by the
    /// statement being cancelled. `None` means no editor client has connected yet.
    pub fn editor_cancel_token(&self) -> Option<tokio_postgres::CancelToken> {
        self.editor_cancel.lock().ok().and_then(|g| g.clone())
    }

    /// The live connection, or `NotConnected` — which the UI turns into the
    /// connect modal rather than an error toast.
    ///
    /// Returns a clone of the `Arc`, so the read guard is released before this
    /// returns and callers can hold the result across an await without blocking
    /// a concurrent [`set_connection`](Self::set_connection).
    pub async fn connection(&self) -> Result<Arc<Connection>> {
        self.connection
            .read()
            .await
            .clone()
            .ok_or(AppError::NotConnected)
    }

    /// [`connection`](Self::connection) for callers to which being disconnected
    /// is an ordinary answer rather than a failure — the background pinger, which
    /// would otherwise raise an error every 15 seconds while the user is
    /// disconnected, and `connection_info`, for which "no connection" is a state
    /// the titlebar renders rather than an error.
    pub async fn connection_opt(&self) -> Option<Arc<Connection>> {
        self.connection.read().await.clone()
    }

    // ---- terminal sessions ----

    pub async fn add_repl(&self, session: ReplSession) -> String {
        let id = session.id.clone();
        // Snapshot the cancel handle before the session goes behind its mutex.
        self.repl_cancels
            .lock()
            .await
            .insert(id.clone(), session.cancel_token());
        self.repl_sessions
            .lock()
            .await
            .insert(id.clone(), Arc::new(Mutex::new(session)));
        id
    }

    /// The cancel handle for a session, reachable while `submit` holds the lock.
    pub async fn repl_cancel_token(&self, id: &str) -> Option<tokio_postgres::CancelToken> {
        self.repl_cancels.lock().await.get(id).cloned()
    }

    /// Look up a terminal session by id, erroring if it is gone — which it will
    /// be after a disconnect, since [`set_connection`](Self::set_connection)
    /// clears the map.
    ///
    /// The registry lock is released before this returns; the caller then locks
    /// the session itself, and may hold *that* across an await for as long as the
    /// statement runs. Cancellation stays reachable meanwhile because the cancel
    /// handle lives in `repl_cancels`, not behind the session mutex.
    pub async fn repl(&self, id: &str) -> Result<Arc<Mutex<ReplSession>>> {
        self.repl_sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NoSession(id.to_string()))
    }

    /// Forget a terminal session, dropping its cancel handle with it so the two
    /// maps cannot drift apart.
    ///
    /// The session's client is only actually closed once the last `Arc` to it is
    /// dropped, so removing it while a statement is still running is safe: that
    /// run finishes, then the session goes away.
    pub async fn remove_repl(&self, id: &str) {
        self.repl_sessions.lock().await.remove(id);
        self.repl_cancels.lock().await.remove(id);
    }

    // ---- grid cancellation ----

    pub fn set_grid_cancel(&self, token: Option<tokio_postgres::CancelToken>) {
        if let Ok(mut guard) = self.grid_cancel.lock() {
            *guard = token;
        }
    }

    /// Take the in-flight grid query's cancel handle, leaving `None` behind.
    ///
    /// Takes rather than clones — unlike the editor's, whose client outlives any
    /// one run — because a grid handle belongs to exactly one query. Removing it
    /// means a second cancel cannot fire at whatever query happened to start
    /// next. `None` therefore means "nothing to cancel", including the case where
    /// the query already finished.
    pub fn take_grid_cancel(&self) -> Option<tokio_postgres::CancelToken> {
        self.grid_cancel.lock().ok().and_then(|mut g| g.take())
    }
}
