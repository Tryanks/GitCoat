//! Liveness endpoint.

use topcoat::{Result, router::route};

/// `GET /healthz` → `200 ok`.
#[route(GET "/healthz")]
pub async fn healthz() -> Result<&'static str> {
    Ok("ok\n")
}
