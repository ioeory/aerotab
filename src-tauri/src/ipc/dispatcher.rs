//! Method dispatcher for the JSON-RPC layer.
//!
//! The dispatcher owns an `Arc<HashMap<&'static str, Handler>>`, where each
//! handler is an `async fn(params) -> Result<Value, RpcError>` packaged as a
//! `dyn Fn`. Methods are registered at startup via [`Dispatcher::register`].
//!
//! This module is transport-agnostic: a Tauri command, a unix socket, or a
//! plugin's stdio bridge can all call [`Dispatcher::dispatch`] with a raw
//! [`Request`] and surface the [`Response`].

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use super::{ErrorCode, Request, Response, RpcError};

pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send>>;
pub type Handler = Arc<dyn Fn(Value) -> HandlerFuture + Send + Sync>;

#[derive(Default, Clone)]
pub struct Dispatcher {
    handlers: Arc<parking_lot_lite::RwLock<HashMap<&'static str, Handler>>>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F, Fut>(&self, method: &'static str, f: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, RpcError>> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |params| Box::pin(f(params)));
        self.handlers.write().insert(method, handler);
    }

    pub fn methods(&self) -> Vec<&'static str> {
        self.handlers.read().keys().copied().collect()
    }

    pub async fn dispatch(&self, req: Request) -> Response {
        if req.jsonrpc != "2.0" {
            return error_response(req.id, ErrorCode::InvalidRequest, "jsonrpc must be \"2.0\"");
        }
        let handler = {
            let map = self.handlers.read();
            map.get(req.method.as_str()).cloned()
        };
        match handler {
            Some(h) => match h(req.params).await {
                Ok(value) => Response {
                    jsonrpc: "2.0".into(),
                    id: req.id,
                    result: Some(value),
                    error: None,
                },
                Err(err) => Response {
                    jsonrpc: "2.0".into(),
                    id: req.id,
                    result: None,
                    error: Some(err),
                },
            },
            None => error_response(
                req.id,
                ErrorCode::MethodNotFound,
                format!("method '{}' not found", req.method),
            ),
        }
    }
}

fn error_response(id: Option<Value>, code: ErrorCode, message: impl Into<String>) -> Response {
    Response {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(RpcError::new(code, message)),
    }
}

/// Tiny RwLock shim so we don't drag in `parking_lot` just for this.
mod parking_lot_lite {
    use std::sync::RwLock as StdRwLock;

    pub struct RwLock<T>(StdRwLock<T>);

    impl<T: Default> Default for RwLock<T> {
        fn default() -> Self {
            Self(StdRwLock::new(T::default()))
        }
    }

    impl<T> RwLock<T> {
        pub fn read(&self) -> std::sync::RwLockReadGuard<'_, T> {
            self.0.read().expect("ipc dispatcher rwlock poisoned")
        }
        pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, T> {
            self.0.write().expect("ipc dispatcher rwlock poisoned")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(method: &str, params: Value) -> Request {
        Request {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn unknown_method_returns_minus_32601() {
        let d = Dispatcher::new();
        let resp = d.dispatch(req("nope", json!(null))).await;
        assert_eq!(resp.error.unwrap().code, ErrorCode::MethodNotFound as i32);
    }

    #[tokio::test]
    async fn registered_handler_runs() {
        let d = Dispatcher::new();
        d.register("ping", |_p| async { Ok(json!("pong")) });
        let resp = d.dispatch(req("ping", json!(null))).await;
        assert_eq!(resp.result, Some(json!("pong")));
    }

    #[tokio::test]
    async fn bad_jsonrpc_version_rejected() {
        let d = Dispatcher::new();
        let mut r = req("ping", json!(null));
        r.jsonrpc = "1.0".into();
        let resp = d.dispatch(r).await;
        assert_eq!(resp.error.unwrap().code, ErrorCode::InvalidRequest as i32);
    }

    #[tokio::test]
    async fn handler_error_is_propagated() {
        let d = Dispatcher::new();
        d.register("boom", |_p| async {
            Err(RpcError::new(ErrorCode::SessionNotFound, "nope"))
        });
        let resp = d.dispatch(req("boom", json!(null))).await;
        let err = resp.error.unwrap();
        assert_eq!(err.code, ErrorCode::SessionNotFound as i32);
        assert!(err.message.contains("nope"));
    }

    #[tokio::test]
    async fn methods_lists_registered() {
        let d = Dispatcher::new();
        d.register("a", |_| async { Ok(json!(null)) });
        d.register("b", |_| async { Ok(json!(null)) });
        let mut m = d.methods();
        m.sort();
        assert_eq!(m, vec!["a", "b"]);
    }
}
