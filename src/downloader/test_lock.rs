// SPDX-License-Identifier: GPL-3.0-only
use std::sync::LazyLock;
use axum::{
    Router,
    body::Body,
    http::StatusCode,
    response::Response,
    routing::{get, post},
};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;

static MOCK_HTTP_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct LocalTestServer {
    base_url: String,
    _handle: JoinHandle<()>,
}

impl LocalTestServer {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }
}

async fn spawn_local_server() -> LocalTestServer {
    let router = Router::new()
        .route(
            "/test.zip",
            get(|| async { (StatusCode::OK, "test file content") }),
        )
        .route(
            "/workshop.zip",
            get(|| async { (StatusCode::OK, "zip content") }),
        )
        .route(
            "/maps/custom/test_map.zip",
            get(|| async { (StatusCode::OK, "map content") }),
        )
        .route("/download", get(|| async { (StatusCode::OK, "content") }))
        .route(
            "/large.zip",
            get(|| async { (StatusCode::OK, "x".repeat(32 * 1024)) }),
        )
        .route(
            "/notfound.zip",
            get(|| async {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::empty())
                    .unwrap()
            }),
        )
        .route(
            "/error.zip",
            get(|| async {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::empty())
                    .unwrap()
            }),
        )
        .route(
            "/internalerror.zip",
            get(|| async {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .unwrap()
            }),
        )
        .route(
            "/steam/GetPublishedFileDetails/v1/",
            post(|| async {
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "response": {
                            "publishedfiledetails": [{
                                "file_url": "https://cdn.steamusercontent.com/ugc/15796922369319871036/71E2E9A2C09C7D8A82E9DE8F4DAAC044B62FD00E/"
                            }]
                        }
                    })),
                )
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve test HTTP");
    });

    let base_url = format!("http://{addr}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .build()
        .expect("probe client");
    let probe_url = format!("{base_url}/test.zip");
    for _ in 0..50 {
        if let Ok(response) = client.get(&probe_url).send().await {
            if response.status().is_success() {
                if let Ok(body) = response.text().await {
                    if body == "test file content" {
                        return LocalTestServer {
                            base_url,
                            _handle: handle,
                        };
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    handle.abort();
    panic!("test HTTP server not ready at {probe_url}");
}

pub struct HttpTestGuard {
    _lock: MutexGuard<'static, ()>,
    server: LocalTestServer,
}

impl HttpTestGuard {
    pub fn url(&self, path: &str) -> String {
        self.server.url(path)
    }
}

/// Hold for the entire HTTP download test; serializes tests and uses a fresh local server.
pub async fn acquire_http_test_lock() -> HttpTestGuard {
    let lock = MOCK_HTTP_LOCK.lock().await;
    let server = spawn_local_server().await;
    HttpTestGuard {
        _lock: lock,
        server,
    }
}
